//! Dead letters (ADR 0028): an actor that keeps failed messages, told of them by a watch, and sends one again or
//! forgets it when an operator says so.

use std::collections::BTreeMap;

use cicada::prelude::*;
use cicada::time::Timestamp;
use cicada::turn::TurnError;
use cicada::{Failed, FailureCause, Stopped};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A dead-letters actor, `cicada.dead-letters/<name>`; any name, `_` by convention ([`DeadLetters::default`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeadLetters {
    /// The name: the key of its address.
    pub name: String,
}

impl DeadLetters {
    /// The dead-letters actor named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Default for DeadLetters {
    /// `cicada.dead-letters/_`.
    fn default() -> Self {
        Self::named("_")
    }
}

impl ActorRef for DeadLetters {
    const TYPE: &'static str = "cicada.dead-letters";
    type Message = DeadLettersMsg;
}

/// The messages of [`DeadLetters`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum DeadLettersMsg {
    /// Sends the failed message of a letter again, once: to the same actor, with the same type, payload and
    /// correlation id. Its sender is the dead-letters actor.
    Redrive {
        /// The id of the failed message.
        message_id: MessageId,
    },
    /// Forgets a letter.
    Discard {
        /// The id of the failed message.
        message_id: MessageId,
    },
    /// A message ended `failed`: it is recorded as a letter, unless it is a message of the dead-letters actor itself.
    Failed(Failed),
    /// Ignored.
    Stopped(Stopped),
}

/// One failed message, as the `Failed` of its watch told it, and what became of a redrive of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeadLetter {
    /// The actor the failed message was for.
    pub actor: ActorAddress,
    /// Its type name.
    pub message_type: String,
    /// Its payload.
    pub payload: Value,
    /// Its correlation id; null when it has none.
    pub correlation_id: Option<String>,
    /// Its sender; null for a message sent from outside a turn.
    pub from: Option<ActorAddress>,
    /// The error it failed with.
    pub error: String,
    /// What failed.
    pub cause: FailureCause,
    /// How often it was claimed.
    pub attempts: u32,
    /// When the letter was recorded (store time).
    pub recorded_at: Timestamp,
    /// The id of the message a redrive sent; once set, the letter is not sent again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redriven_as: Option<MessageId>,
    /// Why the last redrive sent nothing: the type of the actor no longer takes the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redrive_error: Option<String>,
}

/// The state of [`DeadLetters`]: the letters by the id of the failed message. The order of the keys means nothing (a
/// store may sort them); age is told by [`DeadLetter::recorded_at`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeadLettersState {
    /// The letters by the id of the failed message.
    #[serde(default)]
    pub letters: BTreeMap<MessageId, DeadLetter>,
}

/// The dead-letters type. It keeps at most its capacity of letters, and drops the oldest first: by the time they were
/// recorded, and of letters recorded at the same store time, by the id of the failed message.
///
/// A host creates the actor and sets its watch, on every type (`*`) or on the types it cares for, with
/// [`ActorSystem::watch`], `PUT /watches` or `cicada create watch`. Its own failures (a redrive to an actor that is
/// gone) make no letters.
///
/// ```
/// # async fn node(store: cicada::Store) -> Result<(), Box<dyn std::error::Error>> {
/// use cicada::{ActorRef, ActorSystem, WatchTarget};
/// use cicada_actors::{DeadLetters, DeadLettersActor, DeadLettersState};
///
/// let system = ActorSystem::builder(store).host(DeadLettersActor::with_capacity(10_000)?).build();
/// system.create(&DeadLetters::default(), &DeadLettersState::default()).await?;
/// system.watch(&WatchTarget::Every, &DeadLetters::default().address()?).await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadLettersActor {
    capacity: usize,
}

impl DeadLettersActor {
    /// The default capacity.
    pub const DEFAULT_CAPACITY: usize = 1_000;

    /// A dead-letters type that keeps at most `capacity` letters.
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
    /// A dead-letters type with the default capacity of 1,000 letters.
    fn default() -> Self {
        Self { capacity: Self::DEFAULT_CAPACITY }
    }
}

impl Actor for DeadLettersActor {
    type Ref = DeadLetters;
    type State = DeadLettersState;

    async fn handle(&self, turn: &mut Turn<Self>, message: DeadLettersMsg) -> HandlerResult {
        match message {
            DeadLettersMsg::Failed(failed) => {
                if &failed.actor != turn.self_address() && !turn.state().letters.contains_key(&failed.message_id) {
                    self.record(turn, failed);
                }
            }
            DeadLettersMsg::Redrive { message_id } => redrive(turn, message_id).await?,
            DeadLettersMsg::Discard { message_id } => {
                if turn.state().letters.contains_key(&message_id) {
                    turn.state_mut().letters.remove(&message_id);
                }
            }
            DeadLettersMsg::Stopped(_) => {}
        }
        Ok(())
    }
}

impl DeadLettersActor {
    fn record(&self, turn: &mut Turn<Self>, failed: Failed) {
        let letter = DeadLetter {
            actor: failed.actor,
            message_type: failed.message_type,
            payload: failed.payload,
            correlation_id: failed.correlation_id,
            from: failed.from,
            error: failed.error,
            cause: failed.cause,
            attempts: failed.attempts,
            recorded_at: turn.now(),
            redriven_as: None,
            redrive_error: None,
        };
        let letters = &mut turn.state_mut().letters;
        letters.insert(failed.message_id, letter);
        if letters.len() > self.capacity {
            let mut by_age: Vec<(Timestamp, MessageId)> =
                letters.iter().map(|(id, letter)| (letter.recorded_at, id.clone())).collect();
            by_age.sort();
            for (_, id) in by_age.into_iter().take(letters.len() - self.capacity) {
                letters.remove(&id);
            }
        }
    }
}

async fn redrive(turn: &mut Turn<DeadLettersActor>, message_id: MessageId) -> Result<(), TurnError> {
    let Some(letter) = turn.state().letters.get(&message_id).cloned() else {
        return Ok(());
    };
    if letter.redriven_as.is_some() {
        return Ok(());
    }
    let mut options = SendOptions::new();
    options.correlation_id = letter.correlation_id.clone();
    // The type name is checked now; an actor that is not there rejects the commit, and the letter stays as it was.
    let updated = match turn.send_raw(&letter.actor, &letter.message_type, letter.payload.clone(), options).await {
        Ok(id) => DeadLetter { redriven_as: Some(id), redrive_error: None, ..letter },
        Err(e @ TurnError::Store(_)) => return Err(e),
        Err(e) => DeadLetter { redrive_error: Some(e.to_string()), ..letter },
    };
    turn.state_mut().letters.insert(message_id, updated);
    Ok(())
}
