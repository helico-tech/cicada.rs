//! The debouncer: an actor that keeps the latest submission of a burst and sends it to its target once the burst is
//! quiet, at the longest wait, or on an explicit `Flush`.

use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp, format_duration, serde_duration};
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};

use crate::supervisor::retry_policy;
use crate::{EncodeError, StoredMessage};

/// A debouncer, `cicada.debouncer/<name>`: one for each key whose bursts it debounces.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Debouncer {
    /// The key: the key of its address.
    pub name: String,
}

impl Debouncer {
    /// The debouncer named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for Debouncer {
    const TYPE: &'static str = "cicada.debouncer";
    type Message = DebouncerMsg;
}

/// The messages of a [`Debouncer`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum DebouncerMsg {
    /// Replaces the kept submission with this one, its target, message, retry policy and correlation id together, and
    /// arms a `Flush` at the quiet period from now, capped by the longest wait from the first submission of the burst.
    /// The target is not checked until the flush.
    Submit {
        /// The target.
        to: ActorAddress,
        /// The message for the target.
        message: StoredMessage,
        /// The retry policy of the message at the target; none: one attempt. A retry there does not pass the debouncer
        /// again.
        #[serde(default)]
        retry: Option<RetryPolicy>,
    },
    /// Sends the kept submission to its target, cancels the armed flush and clears the burst, in one commit; with
    /// nothing kept it does nothing. Anyone may send it: the armed timer, or an operator who delivers early or
    /// recovers a flush that failed. It sends what is kept when it runs, not what was kept when it was sent.
    Flush,
}

impl DebouncerMsg {
    /// A `Submit` of `message` for `to`, with `retry` as the retry policy at the target: the compiler checks that `to`
    /// takes the message, as for a send. Fails for a ref without an address, and for a value that is not a message
    /// family.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Indexer;
    /// # impl ActorRef for Indexer { const TYPE: &'static str = "indexer"; type Message = IndexerMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum IndexerMsg { Reindex { doc: String } }
    /// use cicada_actors::DebouncerMsg;
    ///
    /// let DebouncerMsg::Submit { to, message, retry } =
    ///     DebouncerMsg::submit(&Indexer, IndexerMsg::Reindex { doc: "d1".into() }, None)?
    /// else {
    ///     unreachable!()
    /// };
    /// assert_eq!((to.as_str(), message.message_type.as_str(), retry), ("indexer/_", "Reindex", None));
    /// # Ok::<(), cicada_actors::EncodeError>(())
    /// ```
    ///
    /// A message of another family does not compile:
    ///
    /// ```compile_fail
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Indexer;
    /// # impl ActorRef for Indexer { const TYPE: &'static str = "indexer"; type Message = IndexerMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum IndexerMsg { Reindex { doc: String } }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum MailerMsg { Send { to: String } }
    /// use cicada_actors::DebouncerMsg;
    ///
    /// let wrong = DebouncerMsg::submit(&Indexer, MailerMsg::Send { to: "a@b.c".into() }, None);
    /// ```
    pub fn submit<R: ActorRef>(
        to: &R,
        message: impl Into<R::Message>,
        retry: Option<RetryPolicy>,
    ) -> Result<Self, EncodeError> {
        let address = to.address().map_err(|e| EncodeError(e.to_string()))?;
        Ok(DebouncerMsg::Submit { to: address, message: StoredMessage::for_ref(to, message)?, retry })
    }
}

/// Settings or a state of a debouncer out of their bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidDebounce(String);

/// The submission a debouncer keeps: what a flush sends, where, and with which retry policy and correlation id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DebouncedMessage {
    /// The target.
    pub to: ActorAddress,
    /// The message for the target.
    pub message: StoredMessage,
    /// The retry policy of the message at the target; none: one attempt.
    #[serde(default)]
    pub retry: Option<RetryPolicy>,
    /// The correlation id of the submission, which the target's message carries.
    #[serde(default)]
    pub correlation_id: Option<String>,
}

/// The state of a [`Debouncer`]: its quiet period and longest wait, fixed at its create, and the burst it keeps. Build
/// it with `DebouncerState::new`; a decoded state is checked the same way.
///
/// On the wire: `{"quiet":"PT2S","maxWait":"PT5S"}`, with `pending`, `firstAt` and `armed` while it keeps a burst.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct DebouncerState {
    /// How long a burst must be quiet before its kept submission is sent.
    #[serde(serialize_with = "serde_duration::serialize")]
    pub quiet: Duration,
    /// The longest time from the first submission of a burst to its scheduled flush.
    #[serde(serialize_with = "serde_duration::serialize")]
    pub max_wait: Duration,
    /// The latest submission of the burst; none between bursts.
    pub pending: Option<DebouncedMessage>,
    /// The store time the first submission of the burst was handled; set exactly while a submission is kept.
    pub first_at: Option<Timestamp>,
    /// The id of the armed `Flush`, which the next `Submit` or `Flush` cancels; none between bursts. It stays set
    /// when the flush goes without running.
    pub armed: Option<MessageId>,
}

/// The wire form of a [`DebouncerState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RawState {
    /// How long a burst must be quiet before its kept submission is sent, as an ISO-8601 duration; positive and at
    /// most `maxWait`.
    #[serde(deserialize_with = "serde_duration::deserialize")]
    #[schemars(with = "String")]
    quiet: Duration,
    /// The longest time from the first submission of a burst to its scheduled flush, as an ISO-8601 duration; at
    /// least `quiet` and at most `MAX_DELAY`.
    #[serde(deserialize_with = "serde_duration::deserialize")]
    #[schemars(with = "String")]
    max_wait: Duration,
    /// The latest submission of the burst; absent or null between bursts.
    #[serde(default)]
    pending: Option<DebouncedMessage>,
    /// The store time the first submission of the burst was handled; present exactly with `pending`.
    #[serde(default)]
    first_at: Option<Timestamp>,
    /// The id of the armed `Flush`; absent or null between bursts, and may be absent with `pending`.
    #[serde(default)]
    armed: Option<MessageId>,
}

impl TryFrom<RawState> for DebouncerState {
    type Error = InvalidDebounce;

    fn try_from(raw: RawState) -> Result<Self, InvalidDebounce> {
        let state = Self {
            quiet: raw.quiet,
            max_wait: raw.max_wait,
            pending: raw.pending,
            first_at: raw.first_at,
            armed: raw.armed,
        };
        state.check()?;
        Ok(state)
    }
}

impl DebouncerState {
    /// A debouncer that sends the latest submission of a burst once it was quiet for `quiet`, and at the latest
    /// `max_wait` after the burst's first submission; it keeps nothing. Fails unless `0 < quiet <= max_wait <=
    /// MAX_DELAY`.
    ///
    /// ```
    /// use std::time::Duration;
    /// use cicada_actors::DebouncerState;
    ///
    /// let state = DebouncerState::new(Duration::from_secs(2), Duration::from_secs(5))?;
    /// assert_eq!(serde_json::to_value(&state)?, serde_json::json!(
    ///     {"quiet": "PT2S", "maxWait": "PT5S", "pending": null, "firstAt": null, "armed": null}));
    /// assert!(DebouncerState::new(Duration::from_secs(5), Duration::from_secs(2)).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(quiet: Duration, max_wait: Duration) -> Result<Self, InvalidDebounce> {
        let state = Self { quiet, max_wait, pending: None, first_at: None, armed: None };
        state.check()?;
        Ok(state)
    }

    fn check(&self) -> Result<(), InvalidDebounce> {
        if self.quiet.is_zero() {
            return Err(InvalidDebounce("quiet must be positive".to_owned()));
        }
        if self.max_wait > MAX_DELAY {
            return Err(InvalidDebounce(format!(
                "maxWait must not exceed {}, was {}",
                format_duration(MAX_DELAY),
                format_duration(self.max_wait)
            )));
        }
        if self.quiet > self.max_wait {
            return Err(InvalidDebounce(format!(
                "quiet must not exceed maxWait, was {} with maxWait {}",
                format_duration(self.quiet),
                format_duration(self.max_wait)
            )));
        }
        match (&self.pending, &self.first_at, &self.armed) {
            (Some(_), None, _) => return Err(InvalidDebounce("a pending submission needs firstAt".to_owned())),
            (None, Some(_), _) | (None, _, Some(_)) => {
                return Err(InvalidDebounce("a debouncer with nothing pending has no firstAt or armed".to_owned()));
            }
            _ => {}
        }
        if self.first_at.is_some_and(|t| t > MAX_INSTANT) {
            return Err(InvalidDebounce(format!("firstAt must not be after {}", time::format_instant(MAX_INSTANT))));
        }
        Ok(())
    }

    /// The due time of the flush for a submission handled at `now`: the quiet period from `now`, capped by the longest
    /// wait from the first submission of the burst, which is `now` for a new burst. Refused past `MAX_INSTANT`.
    fn due(&self, now: Timestamp) -> Result<(Timestamp, Timestamp), String> {
        let first_at = self.first_at.unwrap_or(now);
        let quiet = time::add(now, self.quiet).ok();
        let cap = time::add(first_at, self.max_wait).ok();
        let due = match (quiet, cap) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match due {
            Some(due) if due <= MAX_INSTANT => Ok((first_at, due)),
            _ => Err(format!("the flush would be due after {}", time::format_instant(MAX_INSTANT))),
        }
    }
}

/// The debouncer type. It has no settings: the quiet period and the longest wait are in the state of each debouncer,
/// fixed at its create; a create of a debouncer that exists changes nothing. It never stops itself.
///
/// Each `Submit` replaces the kept submission whole, and cancels the armed `Flush` and arms a new one at the quiet
/// period from the store time of the turn, capped by the longest wait from the burst's first submission; the timer has
/// the crate's shared retry policy. A superseded submission ends `done` and nobody is told. A `Flush` sends the kept
/// message with the debouncer as its sender, a new id, the kept correlation id and retry policy, and clears the burst
/// in the same commit. So a flush that fails clears nothing: a message the target does not take, or a target type that
/// is not registered, is refused for good (the cause `refused`); a target that is gone rejects the commit (`commit`);
/// a read of the store that fails is tried by the flush's own retry policy. A manual `Flush`, an operator's retry or a
/// redrive sends what is kept then; a later `Submit` replaces it and arms a flush from the old first submission, due at
/// once when the longest wait has passed. A failure at the target after the flush does not restore the kept message.
///
/// Due times are not run times: the longest wait bounds the scheduled flush from the first handled submission, not a
/// flush delayed by a pause, an outage or a slow commit. The kept payload sits three levels deep in the state, so it
/// may be at most 61 levels deep; a deeper one fails its `Submit` (the cause `handler`) and keeps the earlier burst.
#[derive(Debug, Clone, Copy, Default)]
pub struct DebouncerActor;

impl Actor for DebouncerActor {
    type Ref = Debouncer;
    type State = DebouncerState;

    async fn handle(&self, turn: &mut Turn<Self>, message: DebouncerMsg) -> HandlerResult {
        match message {
            DebouncerMsg::Submit { to, message, retry } => submit(turn, to, message, retry),
            DebouncerMsg::Flush => flush(turn).await,
        }
    }
}

/// Keeps the submission and arms a flush for it, replacing what was kept and armed.
fn submit(
    turn: &mut Turn<DebouncerActor>,
    to: ActorAddress,
    message: StoredMessage,
    retry: Option<RetryPolicy>,
) -> HandlerResult {
    let (first_at, due) = turn.state().due(turn.now()).map_err(HandlerError::permanent)?;
    cancel_armed(turn)?;
    let id = turn
        .send_self_with(DebouncerMsg::Flush, SendOptions::new().at(due).retry(retry_policy()))
        .map_err(HandlerError::permanent_from)?;
    let correlation_id = turn.message().correlation_id.clone();
    let state = turn.state_mut();
    state.pending = Some(DebouncedMessage { to, message, retry, correlation_id });
    state.first_at = Some(first_at);
    state.armed = Some(id);
    Ok(())
}

/// Sends the kept submission to its target and clears the burst; nothing when nothing is kept.
async fn flush(turn: &mut Turn<DebouncerActor>) -> HandlerResult {
    let Some(pending) = turn.state().pending.clone() else { return Ok(()) };
    let mut options = SendOptions::new();
    options.retry = pending.retry;
    options.correlation_id = pending.correlation_id;
    match turn.send_raw(&pending.to, &pending.message.message_type, pending.message.payload, options).await {
        Ok(_) => {}
        // Tried again by the retry policy of the flush.
        Err(e @ TurnError::Store(_)) => return Err(e.into()),
        // No registered type, a type that does not take the message, or a payload out of bounds.
        Err(e) => return Err(HandlerError::permanent_from(e)),
    }
    cancel_armed(turn)?;
    let state = turn.state_mut();
    state.pending = None;
    state.first_at = None;
    state.armed = None;
    Ok(())
}

/// Cancels the armed flush, unless it is the message of this turn.
fn cancel_armed(turn: &mut Turn<DebouncerActor>) -> Result<(), TurnError> {
    if let Some(armed) = turn.state().armed.clone()
        && armed != turn.message().id
    {
        turn.cancel(&armed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn t(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn kept() -> serde_json::Value {
        serde_json::json!({"to": "worker/a", "message": {"type": "Note", "payload": {"text": "x"}}})
    }

    #[test]
    fn invalid_settings_are_refused_by_the_constructor_and_the_decoder() {
        let ok = DebouncerState::new(2 * SECOND, 5 * SECOND).unwrap();
        assert_eq!((ok.pending.clone(), ok.first_at, ok.armed.clone()), (None, None, None));
        assert!(DebouncerState::new(SECOND, SECOND).is_ok(), "quiet may equal maxWait");
        assert!(DebouncerState::new(SECOND, MAX_DELAY).is_ok());
        let past = MAX_DELAY + Duration::from_nanos(1);
        for (quiet, max_wait, why) in [
            (Duration::ZERO, SECOND, "quiet must be positive"),
            (5 * SECOND, 2 * SECOND, "quiet must not exceed maxWait"),
            (SECOND, past, "maxWait must not exceed"),
            (past, past, "maxWait must not exceed"),
        ] {
            let built = DebouncerState::new(quiet, max_wait);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
            let json = serde_json::json!({"quiet": format_duration(quiet), "maxWait": format_duration(max_wait)});
            let decoded = serde_json::from_value::<DebouncerState>(json);
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {why}");
        }
        let decoded: DebouncerState =
            serde_json::from_value(serde_json::json!({"quiet": "PT2S", "maxWait": "PT5S", "other": 1})).unwrap();
        assert_eq!(decoded, ok, "the minimal state, unknown fields ignored");
        assert!(serde_json::from_value::<DebouncerState>(serde_json::json!({"quiet": "PT2S"})).is_err());
    }

    #[test]
    fn an_inconsistent_state_is_refused() {
        let at = "2026-01-01T00:00:00Z";
        for (state, why) in [
            (serde_json::json!({"pending": kept()}), "needs firstAt"),
            (serde_json::json!({"pending": kept(), "armed": "f1"}), "needs firstAt"),
            (serde_json::json!({"firstAt": at}), "nothing pending"),
            (serde_json::json!({"armed": "f1"}), "nothing pending"),
            (
                serde_json::json!({"pending": kept(), "firstAt": time::format_instant(
                    time::add(MAX_INSTANT, Duration::from_micros(1)).unwrap())}),
                "firstAt must not be after",
            ),
        ] {
            let mut json = serde_json::json!({"quiet": "PT2S", "maxWait": "PT5S"});
            json.as_object_mut().unwrap().extend(state.as_object().unwrap().clone());
            let decoded = serde_json::from_value::<DebouncerState>(json.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {json}");
        }
        for state in [
            serde_json::json!({"pending": kept(), "firstAt": at}),
            serde_json::json!({"pending": kept(), "firstAt": at, "armed": "f1"}),
            serde_json::json!({"pending": null, "firstAt": null, "armed": null}),
        ] {
            let mut json = serde_json::json!({"quiet": "PT2S", "maxWait": "PT5S"});
            json.as_object_mut().unwrap().extend(state.as_object().unwrap().clone());
            assert!(serde_json::from_value::<DebouncerState>(json.clone()).is_ok(), "{json}");
        }
    }

    #[test]
    fn the_due_time_is_the_quiet_period_capped_by_the_longest_wait() {
        let start = t("2026-01-01T00:00:00Z");
        let at = |s: u32| time::add(start, s * SECOND).unwrap();
        let mut state = DebouncerState::new(2 * SECOND, 5 * SECOND).unwrap();
        assert_eq!(state.due(start), Ok((start, at(2))), "a new burst starts now");
        state.first_at = Some(start);
        assert_eq!(state.due(at(1)), Ok((start, at(3))));
        assert_eq!(state.due(at(4)), Ok((start, at(5))), "capped");
        assert_eq!(state.due(at(9)), Ok((start, at(5))), "past the cap: due at once, clamped by the core");
    }

    #[test]
    fn a_due_time_past_max_instant_is_refused() {
        let state = DebouncerState::new(SECOND, SECOND).unwrap();
        assert!(state.due(MAX_INSTANT).unwrap_err().contains("after 9999-12-31"));
        let last = time::sub(MAX_INSTANT, SECOND);
        assert_eq!(state.due(last), Ok((last, MAX_INSTANT)), "the last instant that fits");
        // The quiet period does not fit, but the cap from an earlier burst start does.
        let mut state = DebouncerState::new(SECOND, MAX_DELAY).unwrap();
        state.first_at = Some(t("2026-01-01T00:00:00Z"));
        let now = Timestamp::MAX_UTC;
        assert!(state.due(now).is_ok_and(|(_, due)| due < MAX_INSTANT));
    }
}
