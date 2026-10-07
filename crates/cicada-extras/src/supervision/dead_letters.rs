use std::collections::BTreeMap;

use cicada::prelude::*;
use cicada::time::{Timestamp, serde_instant};
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The dead-letter queue, a singleton at `actor://cicada/dead-letters`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLetters;

impl ActorRef for DeadLetters {
    const TEMPLATE: &'static str = "actor://cicada/dead-letters";
    type Message = DeadLettersMsg;
}

/// Messages of [`DeadLetters`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
#[expect(clippy::large_enum_variant, reason = "a message is decoded once per turn and rarely moved")]
pub enum DeadLettersMsg {
    /// Records one failed message; sent by the [`super::FailureWatcher`] with an id derived from the failed message's
    /// id.
    RecordFailure {
        /// The failed message.
        letter: DeadLetter,
    },
    /// Sends the failed message `failed_message_id` again: same target, type, payload and correlation id. Its sender
    /// is the dead-letter actor, not the original one.
    Redrive {
        /// The failed message's id.
        failed_message_id: String,
    },
    /// Forgets a dead letter.
    Discard {
        /// The failed message's id.
        failed_message_id: String,
    },
}

/// One failed message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeadLetter {
    /// The failed message's id.
    pub failed_message_id: String,
    /// Its target.
    pub actor: String,
    /// Its type name.
    #[serde(rename = "type")]
    pub message_type: String,
    /// Its payload.
    pub payload: Value,
    /// Its correlation id.
    #[serde(default)]
    pub correlation_id: Option<String>,
    /// Why it failed.
    #[serde(default)]
    pub error: Option<String>,
    /// When it failed (store time).
    #[serde(with = "serde_instant")]
    #[schemars(with = "String")]
    pub failed_at: Timestamp,
    /// Id of the message sent by `Redrive`, once redriven.
    #[serde(default)]
    pub redriven_as: Option<String>,
    /// Why the last `Redrive` could not send.
    #[serde(default)]
    pub redrive_error: Option<String>,
}

/// State of [`DeadLetters`]: the dead letters by failed message id. The map's key order carries no meaning: a store may
/// not keep it (PostgreSQL's `jsonb` sorts object keys), so age is judged by [`DeadLetter::failed_at`] alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeadLettersState {
    /// The dead letters by failed message id.
    #[serde(default)]
    pub letters: BTreeMap<String, DeadLetter>,
}

/// The dead-letter type. Keeps at most its capacity of letters; the oldest (by [`DeadLetter::failed_at`]) go first.
///
/// ```
/// # use std::sync::Arc;
/// # fn node(store: Arc<dyn cicada::store::Store>) -> Result<(), String> {
/// use cicada::ActorSystem;
/// use cicada_extras::DeadLettersActor;
///
/// let system = ActorSystem::builder(store).host(DeadLettersActor::with_capacity(10_000)?).build();
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadLettersActor {
    capacity: usize,
}

impl DeadLettersActor {
    /// The default capacity.
    pub const DEFAULT_CAPACITY: usize = 1_000;

    /// A dead-letter type that keeps at most `capacity` letters.
    pub fn with_capacity(capacity: usize) -> Result<Self, String> {
        if capacity == 0 {
            return Err("capacity must be positive".to_owned());
        }
        Ok(Self { capacity })
    }

    /// The capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for DeadLettersActor {
    /// A dead-letter type with the default capacity of 1 000 letters.
    fn default() -> Self {
        Self { capacity: Self::DEFAULT_CAPACITY }
    }
}

impl Actor for DeadLettersActor {
    type Ref = DeadLetters;
    type State = DeadLettersState;
    type Message = DeadLettersMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: DeadLettersMsg) -> HandlerResult {
        match message {
            DeadLettersMsg::RecordFailure { letter } => {
                if turn.state().letters.contains_key(&letter.failed_message_id) {
                    return Ok(());
                }
                let letters = &mut turn.state_mut().letters;
                letters.insert(letter.failed_message_id.clone(), letter);
                if letters.len() > self.capacity {
                    let mut by_age: Vec<(Timestamp, String)> =
                        letters.values().map(|l| (l.failed_at, l.failed_message_id.clone())).collect();
                    by_age.sort();
                    for (_, id) in by_age.into_iter().take(letters.len() - self.capacity) {
                        letters.remove(&id);
                    }
                }
            }
            DeadLettersMsg::Redrive { failed_message_id } => {
                let Some(letter) = turn.state().letters.get(&failed_message_id).cloned() else {
                    return Ok(());
                };
                if letter.redriven_as.is_some() {
                    return Ok(()); // redrive once; record the failure again to redrive again
                }
                let to = ActorAddress::parse(&letter.actor)?;
                let mut options = SendOptions::new();
                options.correlation_id = letter.correlation_id.clone();
                // A raw send: the type name is checked now; the target's existence at commit (an absent target fails
                // this whole turn, which then shows up in the failures itself).
                let updated = match turn.send_raw(&to, &letter.message_type, letter.payload.clone(), options).await {
                    Ok(id) => DeadLetter { redriven_as: Some(id.to_string()), redrive_error: None, ..letter },
                    Err(e @ TurnError::SendRejected { .. }) => DeadLetter { redrive_error: Some(e.to_string()), ..letter },
                    Err(e) => return Err(e.into()),
                };
                turn.state_mut().letters.insert(failed_message_id, updated);
            }
            DeadLettersMsg::Discard { failed_message_id } => {
                turn.state_mut().letters.remove(&failed_message_id);
            }
        }
        Ok(())
    }
}
