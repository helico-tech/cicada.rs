//! The batcher: an actor that collects items in handling order and sends them to its target as one message once the
//! batch is full, at its longest wait, or on an explicit `Flush`.

use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::limits::MAX_TYPE_LENGTH;
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp, format_duration, serde_duration};
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::EncodeError;
use crate::supervisor::retry_policy;

/// The most items a batch may hold.
const MAX_ITEMS: u32 = 1_000;

/// A batcher, `cicada.batcher/<name>`: one for each stream or group whose items it batches.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Batcher {
    /// The stream or group: the key of its address.
    pub name: String,
}

impl Batcher {
    /// The batcher named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for Batcher {
    const TYPE: &'static str = "cicada.batcher";
    type Message = BatcherMsg;
}

/// The messages of a [`Batcher`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum BatcherMsg {
    /// Appends one item, any JSON value, `null` included. The first item of a batch arms a `Flush` at the longest wait;
    /// an item that fills the batch, or that comes at or after that deadline, sends the batch with it in this turn.
    Add {
        /// The item.
        item: Value,
    },
    /// Sends the items held to the target, cancels the armed flush and clears the batch, in one commit; with nothing
    /// held it does nothing. Anyone may send it: the armed timer, or an operator who delivers early or recovers a
    /// flush that failed. It sends what is held when it runs, not what was held when it was sent.
    Flush,
}

impl BatcherMsg {
    /// An `Add` of `item`. Fails for a value that does not serialize to JSON.
    ///
    /// ```
    /// use cicada_actors::BatcherMsg;
    ///
    /// let add = BatcherMsg::add(serde_json::json!({"row": 7}))?;
    /// assert_eq!(serde_json::to_value(&add)?, serde_json::json!({"type": "Add", "item": {"row": 7}}));
    /// assert_eq!(BatcherMsg::add(())?, BatcherMsg::Add { item: serde_json::Value::Null }, "null is an item");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn add(item: impl Serialize) -> Result<Self, EncodeError> {
        let item = serde_json::to_value(item).map_err(|e| EncodeError(e.to_string()))?;
        Ok(BatcherMsg::Add { item })
    }
}

/// Settings or a state of a batcher out of their bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidBatch(String);

/// The state of a [`Batcher`]: its target, message type, count limit, longest wait and target retry policy, fixed at
/// its create, and the batch it holds. Build it with `BatcherState::new`; a decoded state is checked the same way.
///
/// On the wire: `{"target":"indexer/_","messageType":"Index","maxItems":100,"maxWait":"PT5S"}`, with `retry`, and
/// `items`, `firstAt` and `armed` while it holds a batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct BatcherState {
    /// The target of each batch.
    pub target: ActorAddress,
    /// The message type of each batch, whose payload is `{"items": [...]}`.
    pub message_type: String,
    /// The items that make a batch full; a full batch is sent at once.
    pub max_items: u32,
    /// The longest time from the first item of a batch to its scheduled flush.
    #[serde(serialize_with = "serde_duration::serialize")]
    pub max_wait: Duration,
    /// The retry policy of each batch at the target; none: one attempt.
    pub retry: Option<RetryPolicy>,
    /// The items held, in handling order; always fewer than `max_items`.
    pub items: Vec<Value>,
    /// The store time the first item of the batch was handled; set exactly while items are held.
    pub first_at: Option<Timestamp>,
    /// The id of the armed `Flush`, which the flush of the batch cancels; none between batches. It stays set when the
    /// flush goes without running.
    pub armed: Option<MessageId>,
}

/// The wire form of a [`BatcherState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RawState {
    /// The target of each batch.
    target: ActorAddress,
    /// The message type of each batch, whose payload is `{"items": [...]}`; not empty, at most `MAX_TYPE_LENGTH`
    /// characters, without NUL. The target's registry is checked only when a batch is sent.
    message_type: String,
    /// The items that make a batch full; 1 through 1000. A full batch is sent at once, so a state holds fewer.
    #[schemars(range(min = 1, max = 1000))]
    max_items: u32,
    /// The longest time from the first item of a batch to its scheduled flush, as an ISO-8601 duration; positive and
    /// at most `MAX_DELAY`.
    #[serde(deserialize_with = "serde_duration::deserialize")]
    #[schemars(with = "String")]
    max_wait: Duration,
    /// The retry policy of each batch at the target; absent or null: one attempt.
    #[serde(default)]
    retry: Option<RetryPolicy>,
    /// The items held, in handling order; fewer than `maxItems`; absent: none.
    #[serde(default)]
    items: Vec<Value>,
    /// The store time the first item of the batch was handled; present exactly while items are held.
    #[serde(default)]
    first_at: Option<Timestamp>,
    /// The id of the armed `Flush`; absent or null between batches, and may be absent while items are held.
    #[serde(default)]
    armed: Option<MessageId>,
}

impl TryFrom<RawState> for BatcherState {
    type Error = InvalidBatch;

    fn try_from(raw: RawState) -> Result<Self, InvalidBatch> {
        let state = Self {
            target: raw.target,
            message_type: raw.message_type,
            max_items: raw.max_items,
            max_wait: raw.max_wait,
            retry: raw.retry,
            items: raw.items,
            first_at: raw.first_at,
            armed: raw.armed,
        };
        state.check()?;
        Ok(state)
    }
}

impl BatcherState {
    /// A batcher that sends `message_type` with `{"items": [...]}` to `target` once it holds `max_items` items, and at
    /// the latest `max_wait` after a batch's first item; it holds nothing and sends with no retry policy. Fails for a
    /// ref without an address, unless `1 <= max_items <= 1000` and `0 < max_wait <= MAX_DELAY`, and for a message type
    /// that is empty, longer than `MAX_TYPE_LENGTH` characters or holds a NUL.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Indexer;
    /// # impl ActorRef for Indexer { const TYPE: &'static str = "indexer"; type Message = IndexerMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum IndexerMsg { Index { items: Vec<String> } }
    /// use std::time::Duration;
    /// use cicada_actors::BatcherState;
    ///
    /// let state = BatcherState::new(&Indexer, "Index", 100, Duration::from_secs(5))?;
    /// assert_eq!(serde_json::to_value(&state)?, serde_json::json!({
    ///     "target": "indexer/_", "messageType": "Index", "maxItems": 100, "maxWait": "PT5S",
    ///     "retry": null, "items": [], "firstAt": null, "armed": null}));
    /// assert!(BatcherState::new(&Indexer, "Index", 0, Duration::from_secs(5)).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new<R: ActorRef>(
        target: &R,
        message_type: impl Into<String>,
        max_items: u32,
        max_wait: Duration,
    ) -> Result<Self, InvalidBatch> {
        let target = target.address().map_err(|e| InvalidBatch(e.to_string()))?;
        let state = Self {
            target,
            message_type: message_type.into(),
            max_items,
            max_wait,
            retry: None,
            items: Vec::new(),
            first_at: None,
            armed: None,
        };
        state.check()?;
        Ok(state)
    }

    fn check(&self) -> Result<(), InvalidBatch> {
        if !(1..=MAX_ITEMS).contains(&self.max_items) {
            return Err(InvalidBatch(format!("maxItems must be 1 through {MAX_ITEMS}, was {}", self.max_items)));
        }
        if self.max_wait.is_zero() || self.max_wait > MAX_DELAY {
            return Err(InvalidBatch(format!(
                "maxWait must be positive and at most {}, was {}",
                format_duration(MAX_DELAY),
                format_duration(self.max_wait)
            )));
        }
        let length = self.message_type.chars().count();
        if length == 0 || length > MAX_TYPE_LENGTH || self.message_type.contains('\0') {
            return Err(InvalidBatch(format!(
                "messageType must be 1 through {MAX_TYPE_LENGTH} characters without NUL, had {length}"
            )));
        }
        if self.items.len() >= self.max_items as usize {
            return Err(InvalidBatch(format!(
                "a batcher holds fewer than maxItems ({}) items, had {}",
                self.max_items,
                self.items.len()
            )));
        }
        match (self.items.is_empty(), &self.first_at, &self.armed) {
            (false, None, _) => return Err(InvalidBatch("held items need firstAt".to_owned())),
            (true, Some(_), _) | (true, _, Some(_)) => {
                return Err(InvalidBatch("a batcher with no items has no firstAt or armed".to_owned()));
            }
            _ => {}
        }
        if self.first_at.is_some_and(|t| t > MAX_INSTANT) {
            return Err(InvalidBatch(format!("firstAt must not be after {}", time::format_instant(MAX_INSTANT))));
        }
        Ok(())
    }

    /// The deadline of the batch whose first item was handled at `first_at`. Refused past `MAX_INSTANT`.
    fn deadline(&self, first_at: Timestamp) -> Result<Timestamp, String> {
        match time::add(first_at, self.max_wait) {
            Ok(due) if due <= MAX_INSTANT => Ok(due),
            _ => Err(format!("the flush would be due after {}", time::format_instant(MAX_INSTANT))),
        }
    }
}

/// The batcher type. It has no settings: the target, message type, count limit, longest wait and target retry policy
/// are in the state of each batcher, fixed at its create; a create of a batcher that exists changes nothing. It never
/// stops itself.
///
/// Each `Add` appends its item in handling order. An item that brings the batch to `maxItems`, or that is handled at or
/// after the batch's deadline (its first item's store time plus `maxWait`), sends the batch with that item in the same
/// turn; so a stored batch always holds fewer than `maxItems` items. Otherwise the first item of a batch arms a `Flush`
/// at the deadline, with the crate's shared retry policy, and later items keep it. A flush sends the message type with
/// `{"items": [...]}`, the batcher as its sender, a new id, the state's retry policy and no correlation id, and cancels
/// the armed flush and clears the batch in the same commit. So a flush that fails clears nothing, and an `Add` whose
/// flush fails keeps no item: a target type that is not registered or does not take the message, or a payload out of
/// bounds, is refused for good (the cause `refused`); a target that is gone rejects the commit (`commit`); a read of
/// the store that fails is tried by the incoming message's own retry policy. A manual `Flush`, an operator's retry or a
/// redrive sends what is held then, even a later batch. A failure at the target after the flush restores nothing.
///
/// A deadline is not a run time: it bounds the scheduled flush from the first handled item, not a flush delayed by a
/// pause, an outage or a slow commit. The items sit two levels deep in the state and the batch, so an item may be at
/// most 62 levels deep; a deeper one fails its `Add` (the cause `handler`, or `refused` when it would be sent at once)
/// and keeps the earlier batch. `maxItems` bounds the count of items, not their bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct BatcherActor;

impl Actor for BatcherActor {
    type Ref = Batcher;
    type State = BatcherState;

    async fn handle(&self, turn: &mut Turn<Self>, message: BatcherMsg) -> HandlerResult {
        match message {
            BatcherMsg::Add { item } => add(turn, item).await,
            BatcherMsg::Flush => {
                let items = std::mem::take(&mut turn.state_mut().items);
                if items.is_empty() {
                    return Ok(());
                }
                forward(turn, items).await
            }
        }
    }
}

/// Appends the item; sends the batch with it when that fills the batch or its deadline has passed, and arms a flush
/// for the first item of a batch otherwise.
async fn add(turn: &mut Turn<BatcherActor>, item: Value) -> HandlerResult {
    let now = turn.now();
    let state = turn.state();
    let full = state.items.len() + 1 >= state.max_items as usize;
    if full {
        let mut items = std::mem::take(&mut turn.state_mut().items);
        items.push(item);
        return forward(turn, items).await;
    }
    let first = state.items.is_empty();
    let deadline = state.deadline(state.first_at.unwrap_or(now)).map_err(HandlerError::permanent)?;
    if now >= deadline {
        let mut items = std::mem::take(&mut turn.state_mut().items);
        items.push(item);
        return forward(turn, items).await;
    }
    if first {
        let id = turn
            .send_self_with(BatcherMsg::Flush, SendOptions::new().at(deadline).retry(retry_policy()))
            .map_err(HandlerError::permanent_from)?;
        let state = turn.state_mut();
        state.first_at = Some(now);
        state.armed = Some(id);
    }
    turn.state_mut().items.push(item);
    Ok(())
}

/// Sends `items` as one batch to the target, cancels the armed flush unless it is this turn's message, and clears the
/// batch. On an error the turn rolls back, so the state stays as it was.
async fn forward(turn: &mut Turn<BatcherActor>, items: Vec<Value>) -> HandlerResult {
    let state = turn.state();
    let (target, message_type) = (state.target.clone(), state.message_type.clone());
    let mut options = SendOptions::new();
    options.retry = state.retry.clone();
    match turn.send_raw(&target, &message_type, serde_json::json!({ "items": items }), options).await {
        Ok(_) => {}
        // Tried again by the retry policy of the incoming message.
        Err(e @ TurnError::Store(_)) => return Err(e.into()),
        // No registered type, a type that does not take the message, or a payload out of bounds.
        Err(e) => return Err(HandlerError::permanent_from(e)),
    }
    if let Some(armed) = turn.state().armed.clone()
        && armed != turn.message().id
    {
        turn.cancel(&armed)?;
    }
    let state = turn.state_mut();
    state.items.clear();
    state.first_at = None;
    state.armed = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Indexer;

    impl ActorRef for Indexer {
        const TYPE: &'static str = "indexer";
        type Message = IndexerMsg;
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum IndexerMsg {
        Index { items: Vec<Value> },
    }

    fn base() -> serde_json::Value {
        serde_json::json!({"target": "indexer/_", "messageType": "Index", "maxItems": 3, "maxWait": "PT10S"})
    }

    fn with(extra: serde_json::Value) -> serde_json::Value {
        let mut json = base();
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    }

    #[test]
    fn invalid_settings_are_refused_by_the_constructor_and_the_decoder() {
        let ok = BatcherState::new(&Indexer, "Index", 3, 10 * SECOND).unwrap();
        assert_eq!((ok.items.len(), ok.first_at, ok.armed.clone(), ok.retry.clone()), (0, None, None, None));
        assert!(BatcherState::new(&Indexer, "Index", 1, SECOND).is_ok());
        assert!(BatcherState::new(&Indexer, "I".repeat(MAX_TYPE_LENGTH), 1000, MAX_DELAY).is_ok());
        let past = MAX_DELAY + Duration::from_nanos(1);
        let long = "I".repeat(MAX_TYPE_LENGTH + 1);
        for (message_type, max_items, max_wait, why) in [
            ("Index", 0, SECOND, "maxItems must be 1 through 1000"),
            ("Index", 1001, SECOND, "maxItems must be 1 through 1000"),
            ("Index", 3, Duration::ZERO, "maxWait must be positive"),
            ("Index", 3, past, "maxWait must be positive"),
            ("", 3, SECOND, "messageType must be"),
            (long.as_str(), 3, SECOND, "messageType must be"),
            ("In\0dex", 3, SECOND, "messageType must be"),
        ] {
            let built = BatcherState::new(&Indexer, message_type, max_items, max_wait);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
            let json = serde_json::json!({"target": "indexer/_", "messageType": message_type, "maxItems": max_items,
                "maxWait": format_duration(max_wait)});
            let decoded = serde_json::from_value::<BatcherState>(json);
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {why}");
        }
        let decoded: BatcherState = serde_json::from_value(with(serde_json::json!({"other": 1}))).unwrap();
        assert_eq!(decoded, ok, "the minimal state, unknown fields ignored");
        for missing in ["target", "messageType", "maxItems", "maxWait"] {
            let mut json = base();
            json.as_object_mut().unwrap().remove(missing);
            assert!(serde_json::from_value::<BatcherState>(json).is_err(), "{missing} is required");
        }
    }

    #[test]
    fn a_ref_without_an_address_is_refused() {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct Bad;
        impl ActorRef for Bad {
            const TYPE: &'static str = "no type name";
            type Message = IndexerMsg;
        }
        assert!(BatcherState::new(&Bad, "Index", 3, SECOND).is_err());
    }

    #[test]
    fn an_inconsistent_state_is_refused() {
        let at = "2026-01-01T00:00:00Z";
        let too_late = time::format_instant(time::add(MAX_INSTANT, Duration::from_micros(1)).unwrap());
        for (state, why) in [
            (serde_json::json!({"items": [1, 2, 3], "firstAt": at}), "fewer than maxItems"),
            (serde_json::json!({"items": [1]}), "need firstAt"),
            (serde_json::json!({"items": [1], "armed": "f1"}), "need firstAt"),
            (serde_json::json!({"firstAt": at}), "no items"),
            (serde_json::json!({"items": [], "armed": "f1"}), "no items"),
            (serde_json::json!({"items": [1], "firstAt": too_late}), "firstAt must not be after"),
        ] {
            let json = with(state);
            let decoded = serde_json::from_value::<BatcherState>(json.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {json}");
        }
        for state in [
            serde_json::json!({"items": [1, null], "firstAt": at}),
            serde_json::json!({"items": [null], "firstAt": at, "armed": "f1"}),
            serde_json::json!({"items": [], "firstAt": null, "armed": null, "retry": null}),
        ] {
            let json = with(state);
            assert!(serde_json::from_value::<BatcherState>(json.clone()).is_ok(), "{json}");
        }
    }

    #[test]
    fn the_deadline_is_checked_against_max_instant() {
        let state = BatcherState::new(&Indexer, "Index", 3, 10 * SECOND).unwrap();
        let start: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        assert_eq!(state.deadline(start), time::add(start, 10 * SECOND).map_err(|e| e.to_string()));
        let last = time::sub(MAX_INSTANT, 10 * SECOND);
        assert_eq!(state.deadline(last), Ok(MAX_INSTANT), "the last instant that fits");
        let after = time::add(last, Duration::from_micros(1)).unwrap();
        assert!(state.deadline(after).unwrap_err().contains("after 9999-12-31"));
        assert!(state.deadline(Timestamp::MAX_UTC).is_err(), "overflow");
    }
}
