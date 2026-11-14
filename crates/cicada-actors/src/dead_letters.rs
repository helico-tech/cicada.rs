//! Dead letters (ADR 0028): an actor that keeps failed messages, told of them by a watch, and sends one again or
//! forgets it when an operator says so.

use std::borrow::Cow;
use std::collections::BTreeMap;

use cicada::prelude::*;
use cicada::runtime::limits::MAX_DEPTH;
use cicada::time::Timestamp;
use cicada::turn::TurnError;
use cicada::{Failed, FailureCause, Stopped};
use schemars::{Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize};
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
    /// Only a notification that the store wrote counts: one that a client sent records nothing.
    Failed(Failed),
    /// Ignored.
    Stopped(Stopped),
}

/// One failed message, as the `Failed` of its watch told it, and what became of a redrive of it.
///
/// Its JSON holds the payload in one of two fields: `payload`, the value itself, or, for a payload deeper than 61
/// levels, `payloadJson`, its JSON text, so that the letter keeps the state within 64 levels (ADR 0043).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "LetterWire", into = "LetterWire")]
pub struct DeadLetter {
    /// The actor the failed message was for.
    pub actor: ActorAddress,
    /// Its type name.
    pub message_type: String,
    /// Its payload: the value, or the JSON text of one too deep to keep as a value.
    pub payload: LetterPayload,
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
    /// The `seq` of the notification that told of it: it grows with every message of the store, so it orders letters
    /// recorded at the same store time.
    pub seq: i64,
    /// The id of the message a redrive sent; once set, the letter is not sent again.
    pub redriven_as: Option<MessageId>,
    /// Why the last redrive sent nothing: the kept payload cannot be read or sent, or the type of the actor no longer
    /// takes the message.
    pub redrive_error: Option<String>,
}

/// The payload of a [`DeadLetter`]. A letter records a payload of at most 61 levels as a [`Value`](Self::Value) and a
/// deeper one as its compact JSON text, [`Json`](Self::Json). Reading a state does not parse the text: [`value`]
/// does, when it is called. A letter is written back in the form it was read in.
///
/// [`value`]: Self::value
#[derive(Debug, Clone, PartialEq)]
pub enum LetterPayload {
    /// The payload itself: the field `payload`, which holds any JSON value, `null` too.
    Value(Value),
    /// The JSON text of the payload: the field `payloadJson`, a string.
    Json(String),
}

impl LetterPayload {
    /// The payload as a value: a copy of a [`Value`](Self::Value), or the [`Json`](Self::Json) text parsed with
    /// serde_json's usual nesting limit of 128. Parsing keeps every integer and the bits of every finite float.
    pub fn value(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::Value(value) => Ok(value.clone()),
            Self::Json(text) => serde_json::from_str(text),
        }
    }
}

/// The deepest payload a letter keeps as a value: the letter holds it three levels into the state.
const DEEPEST_VALUE: usize = MAX_DEPTH - 3;

/// One failed message, as the `Failed` of its watch told it, and what became of a redrive of it. It holds exactly one
/// of `payload` and `payloadJson`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(rename = "DeadLetter", extend("oneOf" = [{"required": ["payload"]}, {"required": ["payloadJson"]}]))]
struct LetterWire {
    /// The actor the failed message was for.
    actor: ActorAddress,
    /// Its type name.
    message_type: String,
    /// Its payload, when it is at most 61 levels deep: any JSON value, null too.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Value")]
    payload: Option<Value>,
    /// The compact JSON text of its payload, when it is deeper than 61 levels.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    payload_json: Option<String>,
    /// Its correlation id; null when it has none.
    correlation_id: Option<String>,
    /// Its sender; null for a message sent from outside a turn.
    from: Option<ActorAddress>,
    /// The error it failed with.
    error: String,
    /// What failed.
    cause: FailureCause,
    /// How often it was claimed.
    attempts: u32,
    /// When the letter was recorded (store time).
    recorded_at: Timestamp,
    /// The `seq` of the notification that told of it: it grows with every message of the store, so it orders letters
    /// recorded at the same store time.
    seq: i64,
    /// The id of the message a redrive sent; once set, the letter is not sent again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redriven_as: Option<MessageId>,
    /// Why the last redrive sent nothing: the kept payload cannot be read or sent, or the type of the actor no longer
    /// takes the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redrive_error: Option<String>,
}

/// A field that is there, `null` included: absent is `None` (by `default`), and `null` is decoded as the field's type,
/// so `payload: null` is a value and `payloadJson: null` is refused.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl TryFrom<LetterWire> for DeadLetter {
    type Error = String;

    fn try_from(wire: LetterWire) -> Result<Self, String> {
        let payload = match (wire.payload, wire.payload_json) {
            (Some(value), None) => LetterPayload::Value(value),
            (None, Some(text)) => LetterPayload::Json(text),
            (Some(_), Some(_)) => return Err("a letter holds both payload and payloadJson".to_owned()),
            (None, None) => return Err("a letter holds neither payload nor payloadJson".to_owned()),
        };
        Ok(Self {
            actor: wire.actor,
            message_type: wire.message_type,
            payload,
            correlation_id: wire.correlation_id,
            from: wire.from,
            error: wire.error,
            cause: wire.cause,
            attempts: wire.attempts,
            recorded_at: wire.recorded_at,
            seq: wire.seq,
            redriven_as: wire.redriven_as,
            redrive_error: wire.redrive_error,
        })
    }
}

impl From<DeadLetter> for LetterWire {
    fn from(letter: DeadLetter) -> Self {
        let (payload, payload_json) = match letter.payload {
            LetterPayload::Value(value) => (Some(value), None),
            LetterPayload::Json(text) => (None, Some(text)),
        };
        Self {
            actor: letter.actor,
            message_type: letter.message_type,
            payload,
            payload_json,
            correlation_id: letter.correlation_id,
            from: letter.from,
            error: letter.error,
            cause: letter.cause,
            attempts: letter.attempts,
            recorded_at: letter.recorded_at,
            seq: letter.seq,
            redriven_as: letter.redriven_as,
            redrive_error: letter.redrive_error,
        }
    }
}

impl JsonSchema for DeadLetter {
    fn schema_name() -> Cow<'static, str> {
        "DeadLetter".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        LetterWire::json_schema(generator)
    }
}

/// True when `value` holds more than `limit` arrays and objects inside each other; empty ones count. One walk with a
/// stack of the open containers' iterators, which stops at the first container beyond `limit`: its memory grows with
/// the depth it walks, not with the width of the value, and it never recurses.
fn deeper_than(value: &Value, limit: usize) -> bool {
    enum Inside<'a> {
        Array(std::slice::Iter<'a, Value>),
        Object(serde_json::map::Values<'a>),
    }
    fn inside(value: &Value) -> Option<Inside<'_>> {
        match value {
            Value::Array(items) => Some(Inside::Array(items.iter())),
            Value::Object(fields) => Some(Inside::Object(fields.values())),
            _ => None,
        }
    }
    let mut open = Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(container) = next.and_then(inside) {
            if open.len() == limit {
                return true;
            }
            open.push(container);
        }
        next = match open.last_mut() {
            None => return false,
            Some(Inside::Array(items)) => items.next(),
            Some(Inside::Object(fields)) => fields.next(),
        };
        if next.is_none() {
            open.pop();
        }
    }
}

/// The state of [`DeadLetters`]: the letters by the id of the failed message. The order of the keys means nothing (a
/// store may sort them); age is told by [`DeadLetter::recorded_at`] and then [`DeadLetter::seq`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeadLettersState {
    /// The letters by the id of the failed message.
    #[serde(default)]
    pub letters: BTreeMap<MessageId, DeadLetter>,
}

/// The dead-letters type. It keeps at most its capacity of letters, and drops the oldest first: by the time they were
/// recorded, and of letters recorded at the same store time, by the `seq` of the notification that told of them.
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

/// A capacity of dead letters that is not positive. It prints "capacity must be positive".
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("capacity must be positive")]
pub struct CapacityError;

impl DeadLettersActor {
    /// The default capacity.
    pub const DEFAULT_CAPACITY: usize = 1_000;

    /// A dead-letters type that keeps at most `capacity` letters; fails for 0.
    pub fn with_capacity(capacity: usize) -> Result<Self, CapacityError> {
        if capacity == 0 {
            return Err(CapacityError);
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
                // A `Failed` from a client is no failure: a letter of it could make a redrive send what the client
                // chose, in the name of this actor.
                let notified = turn.message().notification;
                if notified
                    && &failed.actor != turn.self_address()
                    && !turn.state().letters.contains_key(&failed.message_id)
                {
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
        let payload = if deeper_than(&failed.payload, DEEPEST_VALUE) {
            LetterPayload::Json(failed.payload.to_string())
        } else {
            LetterPayload::Value(failed.payload)
        };
        let letter = DeadLetter {
            actor: failed.actor,
            message_type: failed.message_type,
            payload,
            correlation_id: failed.correlation_id,
            from: failed.from,
            error: failed.error,
            cause: failed.cause,
            attempts: failed.attempts,
            recorded_at: turn.now(),
            seq: turn.message().seq,
            redriven_as: None,
            redrive_error: None,
        };
        let letters = &mut turn.state_mut().letters;
        letters.insert(failed.message_id, letter);
        if letters.len() > self.capacity {
            let mut by_age: Vec<(Timestamp, i64, MessageId)> =
                letters.iter().map(|(id, letter)| (letter.recorded_at, letter.seq, id.clone())).collect();
            by_age.sort();
            for (_, _, id) in by_age.into_iter().take(letters.len() - self.capacity) {
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
    // Text that cannot be parsed sends nothing, and the letter keeps it.
    let payload = match letter.payload.value() {
        Ok(payload) => payload,
        Err(e) => {
            let updated = DeadLetter { redrive_error: Some(format!("The kept payload cannot be read: {e}")), ..letter };
            turn.state_mut().letters.insert(message_id, updated);
            return Ok(());
        }
    };
    let mut options = SendOptions::new();
    options.correlation_id = letter.correlation_id.clone();
    // The type name, the depth and the text of the payload are checked now; an actor that is not there rejects the
    // commit, and the letter stays as it was.
    let updated = match turn.send_raw(&letter.actor, &letter.message_type, payload, options).await {
        Ok(id) => DeadLetter { redriven_as: Some(id), redrive_error: None, ..letter },
        Err(e @ TurnError::Store(_)) => return Err(e),
        Err(e) => DeadLetter { redrive_error: Some(e.to_string()), ..letter },
    };
    turn.state_mut().letters.insert(message_id, updated);
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// `inner` inside `levels` containers: arrays and objects in turn.
    fn wrapped(levels: usize, inner: Value) -> Value {
        (0..levels).fold(inner, |inner, level| if level % 2 == 0 { json!([inner]) } else { json!({"k": inner}) })
    }

    #[test]
    fn the_walk_counts_every_container_empty_ones_too_and_mixed_nesting() {
        for (value, levels) in [
            (json!(1), 0),
            (json!(null), 0),
            (json!([]), 1),
            (json!({}), 1),
            (json!([[], {}]), 2),
            (wrapped(61, json!(1)), 61),
            (wrapped(61, json!([])), 62),
            (wrapped(61, json!({})), 62),
            (wrapped(64, json!("x")), 64),
            (json!([1, {"a": [2, {"b": []}]}, "c"]), 5),
            (json!({"a": 1, "b": [[[]]], "c": {}}), 4),
        ] {
            assert!(!deeper_than(&value, levels), "{value} is {levels} deep");
            if levels > 0 {
                assert!(deeper_than(&value, levels - 1), "{value} is deeper than {}", levels - 1);
            }
        }
    }

    #[test]
    fn the_walk_stops_at_the_first_container_beyond_the_limit_and_takes_wide_values() {
        let wide = Value::Array((0..10_000).map(|n| json!({"n": n})).collect());
        assert!(!deeper_than(&wide, 2) && deeper_than(&wide, 1));
        let mut deep = json!(1);
        for _ in 0..1_000 {
            deep = Value::Array(vec![deep]);
        }
        assert!(deeper_than(&deep, DEEPEST_VALUE));
    }
}
