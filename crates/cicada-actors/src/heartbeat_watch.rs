//! The heartbeat watch: an actor that sends a stored alarm to its target once when a watched thing stays silent for
//! longer than an allowed period, counted from the last `Beat` it handled.

use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp, format_duration, serde_duration};
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};

use crate::StoredMessage;
use crate::supervisor::retry_policy;

/// A heartbeat watch, `cicada.heartbeat-watch/<name>`: one for each watched thing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HeartbeatWatch {
    /// The watched thing: the key of its address.
    pub name: String,
}

impl HeartbeatWatch {
    /// The heartbeat watch named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for HeartbeatWatch {
    const TYPE: &'static str = "cicada.heartbeat-watch";
    type Message = HeartbeatWatchMsg;
}

/// The messages of a [`HeartbeatWatch`]. `Expire` is its own: it counts only when the watch sent it itself, it is the
/// armed timer and the watch is `alive`; otherwise it ends `done` and changes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum HeartbeatWatchMsg {
    /// A sign of life: sets `lastBeatAt` to the store time of this turn and the status to `alive`, and replaces the
    /// armed timer with an `Expire` due after the allowed silence, in one commit. It starts a round in `waiting` and
    /// in `silent`. A host may send one with the create, to detect a thing that never beats.
    Beat,
    /// Arms the timer of the `alive` round again at its unchanged deadline, due at once when that has passed, and
    /// keeps `lastBeatAt`: the recovery of a timer that was purged, cancelled or failed for good. In `waiting` and
    /// `silent` it does nothing.
    Rearm,
    /// The timer: sent by the watch to itself, with a retry policy. Before the deadline it arms itself again at the
    /// deadline; at or after it, it sends the alarm and sets the status to `silent`.
    Expire,
}

/// Where a heartbeat watch is in its round.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HeartbeatStatus {
    /// Created, with no `Beat` handled yet: no timer and no `lastBeatAt`.
    #[default]
    Waiting,
    /// A `Beat` was handled and its silence round runs: `lastBeatAt` and `armed` are set.
    Alive,
    /// The round's silence passed and its alarm was sent: `lastBeatAt` stays, `armed` is cleared.
    Silent,
}

/// Settings or a state of a heartbeat watch out of their bounds, or a target or alarm that cannot be kept; it prints
/// why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidHeartbeat(String);

/// The state of a [`HeartbeatWatch`]: its allowed silence, target, alarm and retry policy, fixed at its create, and its
/// round. Build it with `HeartbeatWatchState::new`; a decoded state is checked the same way.
///
/// On the wire: `{"silence":"P1D","target":"ops/_","alarm":{"type":…,"payload":…}}`, with `retry`, `status`,
/// `lastBeatAt` and `armed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct HeartbeatWatchState {
    /// How long the watched thing may be silent, from the last handled `Beat`, before the alarm is sent.
    #[serde(serialize_with = "serde_duration::serialize")]
    pub silence: Duration,
    /// Where the alarm goes.
    pub target: ActorAddress,
    /// The alarm.
    pub alarm: StoredMessage,
    /// The retry policy of the alarm at the target; none: one attempt.
    pub retry: Option<RetryPolicy>,
    /// Where the watch is in its round.
    pub status: HeartbeatStatus,
    /// The store time of the last handled `Beat`; none while `waiting`.
    pub last_beat_at: Option<Timestamp>,
    /// The id of the armed `Expire`, set exactly while `alive`. It stays set when the timer goes without running.
    pub armed: Option<MessageId>,
}

/// The wire form of a [`HeartbeatWatchState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RawState {
    /// How long the watched thing may be silent before the alarm, as an ISO-8601 duration; positive and at most
    /// `MAX_DELAY`.
    #[serde(deserialize_with = "serde_duration::deserialize")]
    #[schemars(with = "String")]
    silence: Duration,
    /// Where the alarm goes.
    target: ActorAddress,
    /// The alarm.
    alarm: StoredMessage,
    /// The retry policy of the alarm at the target; absent or null: one attempt.
    #[serde(default)]
    retry: Option<RetryPolicy>,
    /// `waiting`, `alive` or `silent`; absent: `waiting`.
    #[serde(default)]
    status: HeartbeatStatus,
    /// The store time of the last handled `Beat`: absent or null while `waiting`, present otherwise.
    #[serde(default)]
    last_beat_at: Option<Timestamp>,
    /// The id of the armed `Expire`: present exactly while `alive`.
    #[serde(default)]
    armed: Option<MessageId>,
}

impl TryFrom<RawState> for HeartbeatWatchState {
    type Error = InvalidHeartbeat;

    fn try_from(raw: RawState) -> Result<Self, InvalidHeartbeat> {
        let state = Self {
            silence: raw.silence,
            target: raw.target,
            alarm: raw.alarm,
            retry: raw.retry,
            status: raw.status,
            last_beat_at: raw.last_beat_at,
            armed: raw.armed,
        };
        state.check()?;
        Ok(state)
    }
}

impl HeartbeatWatchState {
    /// A `waiting` watch that sends `alarm` to `target` once the watched thing was silent for `silence` after a handled
    /// `Beat`, with no retry policy for it: the compiler checks that `target` takes the alarm, as for a send. Fails
    /// unless `0 < silence <= MAX_DELAY`, for a ref without an address, and for a value that is not a message family.
    /// Set the retry policy of the alarm with `state.retry = Some(policy)`.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Ops;
    /// # impl ActorRef for Ops { const TYPE: &'static str = "ops"; type Message = OpsMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum OpsMsg { Page { what: String } }
    /// use std::time::Duration;
    /// use cicada_actors::{HeartbeatStatus, HeartbeatWatchState};
    ///
    /// let state = HeartbeatWatchState::new(Duration::from_secs(3600), &Ops, OpsMsg::Page { what: "import".into() })?;
    /// assert_eq!((state.target.as_str(), state.alarm.message_type.as_str()), ("ops/_", "Page"));
    /// assert_eq!((state.status, state.last_beat_at, state.armed), (HeartbeatStatus::Waiting, None, None));
    /// assert!(HeartbeatWatchState::new(Duration::ZERO, &Ops, OpsMsg::Page { what: "x".into() }).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// An alarm of another family does not compile:
    ///
    /// ```compile_fail
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Ops;
    /// # impl ActorRef for Ops { const TYPE: &'static str = "ops"; type Message = OpsMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum OpsMsg { Page { what: String } }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum MailerMsg { Send { to: String } }
    /// use std::time::Duration;
    /// use cicada_actors::HeartbeatWatchState;
    ///
    /// let wrong = HeartbeatWatchState::new(Duration::from_secs(60), &Ops, MailerMsg::Send { to: "a@b.c".into() });
    /// ```
    pub fn new<R: ActorRef>(
        silence: Duration,
        target: &R,
        alarm: impl Into<R::Message>,
    ) -> Result<Self, InvalidHeartbeat> {
        let address = target.address().map_err(|e| InvalidHeartbeat(e.to_string()))?;
        let alarm = StoredMessage::for_ref(target, alarm).map_err(|e| InvalidHeartbeat(e.to_string()))?;
        let state = Self {
            silence,
            target: address,
            alarm,
            retry: None,
            status: HeartbeatStatus::Waiting,
            last_beat_at: None,
            armed: None,
        };
        state.check()?;
        Ok(state)
    }

    fn check(&self) -> Result<(), InvalidHeartbeat> {
        if self.silence.is_zero() {
            return Err(InvalidHeartbeat("silence must be positive".to_owned()));
        }
        if self.silence > MAX_DELAY {
            return Err(InvalidHeartbeat(format!(
                "silence must not exceed {}, was {}",
                format_duration(MAX_DELAY),
                format_duration(self.silence)
            )));
        }
        match (self.status, &self.last_beat_at, &self.armed) {
            (HeartbeatStatus::Waiting, None, None)
            | (HeartbeatStatus::Alive, Some(_), Some(_))
            | (HeartbeatStatus::Silent, Some(_), None) => {}
            (HeartbeatStatus::Waiting, _, _) => {
                return Err(InvalidHeartbeat("a waiting watch has no lastBeatAt or armed".to_owned()));
            }
            (HeartbeatStatus::Alive, _, _) => {
                return Err(InvalidHeartbeat("an alive watch needs lastBeatAt and armed".to_owned()));
            }
            (HeartbeatStatus::Silent, _, _) => {
                return Err(InvalidHeartbeat("a silent watch needs lastBeatAt and has no armed".to_owned()));
            }
        }
        if let Some(at) = self.last_beat_at {
            self.deadline(at).map_err(InvalidHeartbeat)?;
        }
        Ok(())
    }

    /// The deadline of a round whose last `Beat` was handled at `at`: `at` plus the silence, refused when that is not
    /// representable or after `MAX_INSTANT`.
    fn deadline(&self, at: Timestamp) -> Result<Timestamp, String> {
        match time::add(at, self.silence) {
            Ok(due) if due <= MAX_INSTANT => Ok(due),
            _ => Err(format!("the silence would end after {}", time::format_instant(MAX_INSTANT))),
        }
    }
}

/// The heartbeat watch type. It has no settings: the silence, the target, the alarm and its retry policy are in the
/// state of each watch, fixed at its create; a create of a watch that exists changes nothing. It never stops itself.
///
/// A create leaves it `waiting`. Each `Beat` sets `lastBeatAt` to the store time of its turn, cancels the armed timer
/// and arms an `Expire` at `lastBeatAt` plus the silence, with the crate's shared retry policy; the status is `alive`.
/// The armed `Expire` sends the alarm with the watch as its sender, a new id, the stored retry policy and no
/// correlation id, sets the status to `silent` and clears `armed`, in one commit; it arms nothing more. The next `Beat`
/// starts a new round, and nobody is told of the recovery. An `Expire` that runs early, made due by an operator, arms
/// itself again at the deadline; any other `Expire` (from a client, superseded, retried by an operator or redriven
/// under a new id) changes nothing. A watch whose target is itself refuses `Beat`, an active `Rearm` and its timer.
///
/// An alarm that fails commits nothing: a message the target does not take, or a target type that is not registered,
/// is refused for good (the cause `refused`); a target that is gone rejects the commit (`commit`); a read of the store
/// that fails is tried by the timer's own retry policy. A timer that is purged, cancelled or failed for good leaves the
/// watch `alive` with a stale `armed`; `Rearm` arms it again at the old deadline, without inventing a `Beat`.
///
/// Silence is counted from the store time a `Beat` was handled, not from when the thing outside was active or the
/// `Beat` was sent; a `Beat` and the timer race in mailbox order. Due times are not run times: a paused or slow watch
/// can send its alarm late, never before the deadline. The alarm payload sits two levels deep in the state, so it may
/// be at most 62 levels deep.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeartbeatWatchActor;

impl Actor for HeartbeatWatchActor {
    type Ref = HeartbeatWatch;
    type State = HeartbeatWatchState;

    async fn handle(&self, turn: &mut Turn<Self>, message: HeartbeatWatchMsg) -> HandlerResult {
        match message {
            HeartbeatWatchMsg::Beat => beat(turn),
            HeartbeatWatchMsg::Rearm => rearm(turn),
            HeartbeatWatchMsg::Expire => expire(turn).await,
        }
    }
}

/// Starts or replaces the round from the store time of this turn.
fn beat(turn: &mut Turn<HeartbeatWatchActor>) -> HandlerResult {
    refuse_self_target(turn)?;
    let now = turn.now();
    let due = turn.state().deadline(now).map_err(HandlerError::permanent)?;
    arm(turn, due)?;
    let state = turn.state_mut();
    state.last_beat_at = Some(now);
    state.status = HeartbeatStatus::Alive;
    Ok(())
}

/// Arms the timer of the alive round again at its deadline; nothing in any other status.
fn rearm(turn: &mut Turn<HeartbeatWatchActor>) -> HandlerResult {
    let Some(last) = alive_since(turn) else { return Ok(()) };
    refuse_self_target(turn)?;
    let due = turn.state().deadline(last).map_err(HandlerError::permanent)?;
    arm(turn, due)?;
    Ok(())
}

/// Sends the alarm when the armed timer is due, arms it again when it runs early, and ignores every other `Expire`.
async fn expire(turn: &mut Turn<HeartbeatWatchActor>) -> HandlerResult {
    let Some(last) = alive_since(turn) else { return Ok(()) };
    let message = turn.message();
    let own = message.from.as_ref() == Some(turn.self_address());
    if !own || turn.state().armed.as_ref() != Some(&message.id) {
        return Ok(());
    }
    refuse_self_target(turn)?;
    let due = turn.state().deadline(last).map_err(HandlerError::permanent)?;
    if turn.now() < due {
        arm(turn, due)?;
        return Ok(());
    }
    let state = turn.state();
    let (target, alarm) = (state.target.clone(), state.alarm.clone());
    let mut options = SendOptions::new();
    options.retry = state.retry.clone();
    match turn.send_raw(&target, &alarm.message_type, alarm.payload, options).await {
        Ok(_) => {}
        // Tried again by the retry policy of the timer.
        Err(e @ TurnError::Store(_)) => return Err(e.into()),
        // No registered type, a type that does not take the message, or a payload out of bounds.
        Err(e) => return Err(HandlerError::permanent_from(e)),
    }
    let state = turn.state_mut();
    state.status = HeartbeatStatus::Silent;
    state.armed = None;
    Ok(())
}

/// The `lastBeatAt` of an alive watch; none in any other status.
fn alive_since(turn: &Turn<HeartbeatWatchActor>) -> Option<Timestamp> {
    let state = turn.state();
    if state.status == HeartbeatStatus::Alive { state.last_beat_at } else { None }
}

/// Refuses for good a watch whose alarm would go to itself, where it could drive its own round.
fn refuse_self_target(turn: &Turn<HeartbeatWatchActor>) -> HandlerResult {
    if turn.state().target == *turn.self_address() {
        return Err(HandlerError::permanent("a heartbeat watch must not send its alarm to itself"));
    }
    Ok(())
}

/// Cancels the armed timer, unless it is the message of this turn, and arms an `Expire` at `due` with the shared retry
/// policy, keeping its id.
fn arm(turn: &mut Turn<HeartbeatWatchActor>, due: Timestamp) -> Result<(), TurnError> {
    if let Some(armed) = turn.state().armed.clone()
        && armed != turn.message().id
    {
        turn.cancel(&armed)?;
    }
    let id = turn.send_self_with(HeartbeatWatchMsg::Expire, SendOptions::new().at(due).retry(retry_policy()))?;
    turn.state_mut().armed = Some(id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);
    const AT: &str = "2026-01-01T00:00:00Z";

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Ops;

    impl ActorRef for Ops {
        const TYPE: &'static str = "ops";
        type Message = OpsMsg;
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum OpsMsg {
        Page { what: String },
    }

    /// A ref of two fields, which makes no address.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Pair {
        a: String,
        b: String,
    }

    impl ActorRef for Pair {
        const TYPE: &'static str = "pair";
        type Message = OpsMsg;
    }

    /// A ref whose messages are a struct, which is no message family.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Plain;

    impl ActorRef for Plain {
        const TYPE: &'static str = "plain";
        type Message = PlainMsg;
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    struct PlainMsg {
        what: String,
    }

    fn page() -> OpsMsg {
        OpsMsg::Page { what: "import".into() }
    }

    /// The settings of a watch on the wire, with `extra` fields.
    fn with(extra: serde_json::Value) -> serde_json::Value {
        let mut json = serde_json::json!({
            "silence": "PT1H",
            "target": "ops/_",
            "alarm": {"type": "Page", "payload": {"what": "import"}}
        });
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    }

    fn decode(json: serde_json::Value) -> Result<HeartbeatWatchState, serde_json::Error> {
        serde_json::from_value(json)
    }

    #[test]
    fn the_minimal_json_is_a_waiting_watch_and_unknown_fields_are_ignored() {
        let built = HeartbeatWatchState::new(3600 * SECOND, &Ops, page()).unwrap();
        let decoded = decode(with(serde_json::json!({"other": 1}))).unwrap();
        assert_eq!(decoded, built);
        assert_eq!(
            serde_json::to_value(&built).unwrap(),
            with(serde_json::json!({"retry": null, "status": "waiting", "lastBeatAt": null, "armed": null}))
        );
        for missing in ["silence", "target", "alarm"] {
            let mut json = with(serde_json::json!({}));
            json.as_object_mut().unwrap().remove(missing);
            assert!(decode(json).is_err(), "{missing} is required");
        }
    }

    #[test]
    fn every_inconsistent_status_is_refused_and_the_consistent_ones_are_taken() {
        let ok = [
            serde_json::json!({"status": "waiting"}),
            serde_json::json!({"status": "waiting", "lastBeatAt": null, "armed": null}),
            serde_json::json!({"status": "alive", "lastBeatAt": AT, "armed": "e1"}),
            serde_json::json!({"status": "silent", "lastBeatAt": AT}),
            serde_json::json!({"status": "silent", "lastBeatAt": AT, "armed": null}),
        ];
        for extra in ok {
            assert!(decode(with(extra.clone())).is_ok(), "{extra}");
        }
        let refused = [
            (serde_json::json!({"lastBeatAt": AT}), "a waiting watch"),
            (serde_json::json!({"armed": "e1"}), "a waiting watch"),
            (serde_json::json!({"status": "waiting", "lastBeatAt": AT, "armed": "e1"}), "a waiting watch"),
            (serde_json::json!({"status": "alive"}), "an alive watch"),
            (serde_json::json!({"status": "alive", "lastBeatAt": AT}), "an alive watch"),
            (serde_json::json!({"status": "alive", "armed": "e1"}), "an alive watch"),
            (serde_json::json!({"status": "silent"}), "a silent watch"),
            (serde_json::json!({"status": "silent", "armed": "e1"}), "a silent watch"),
            (serde_json::json!({"status": "silent", "lastBeatAt": AT, "armed": "e1"}), "a silent watch"),
        ];
        for (extra, why) in refused {
            let decoded = decode(with(extra.clone()));
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {extra}");
        }
        for extra in [serde_json::json!({"status": "dead"}), serde_json::json!({"status": "Alive"})] {
            assert!(decode(with(extra.clone())).is_err(), "{extra}");
        }
    }

    #[test]
    fn the_silence_is_positive_and_at_most_max_delay() {
        assert!(HeartbeatWatchState::new(Duration::from_nanos(1), &Ops, page()).is_ok());
        assert!(HeartbeatWatchState::new(MAX_DELAY, &Ops, page()).is_ok());
        let past = MAX_DELAY + Duration::from_nanos(1);
        for (silence, why) in [(Duration::ZERO, "silence must be positive"), (past, "silence must not exceed")] {
            let built = HeartbeatWatchState::new(silence, &Ops, page());
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
            let decoded = decode(with(serde_json::json!({"silence": format_duration(silence)})));
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {why}");
        }
        assert!(decode(with(serde_json::json!({"silence": 3600}))).is_err(), "a duration is a text");
        let fraction = decode(with(serde_json::json!({"silence": "PT1.0000005S"}))).unwrap();
        assert_eq!(fraction.silence, Duration::new(1, 500), "fractions below a microsecond are kept");
    }

    #[test]
    fn the_deadline_of_a_timestamp_must_be_representable_and_at_most_max_instant() {
        let last = time::sub(MAX_INSTANT, 3600 * SECOND);
        let state =
            |at: Timestamp| with(serde_json::json!({"status": "silent", "lastBeatAt": time::format_instant(at)}));
        assert!(decode(state(last)).is_ok(), "the deadline is MAX_INSTANT");
        let late = time::add(last, Duration::from_micros(1)).unwrap();
        let decoded = decode(state(late));
        assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains("after 9999-12-31")), "{decoded:?}");
        let built = HeartbeatWatchState::new(3600 * SECOND, &Ops, page()).unwrap();
        assert_eq!(built.deadline(last), Ok(MAX_INSTANT));
        assert!(built.deadline(late).is_err());
        assert!(built.deadline(Timestamp::MAX_UTC).is_err(), "not representable");
        let long = HeartbeatWatchState::new(MAX_DELAY, &Ops, page()).unwrap();
        let at: Timestamp = AT.parse().unwrap();
        assert!(long.deadline(at).is_ok());
        let alive = with(serde_json::json!(
            {"silence": format_duration(MAX_DELAY), "status": "alive", "lastBeatAt": "9100-01-01T00:00:00Z", "armed": "e1"}
        ));
        assert!(decode(alive).is_err(), "an alive round past MAX_INSTANT");
    }

    #[test]
    fn the_constructor_refuses_a_ref_without_an_address_and_a_value_of_no_family() {
        let pair = Pair { a: "x".into(), b: "y".into() };
        let unaddressable = HeartbeatWatchState::new(SECOND, &pair, page());
        assert!(unaddressable.as_ref().is_err_and(|e| e.to_string().contains("no address")), "{unaddressable:?}");
        let plain = HeartbeatWatchState::new(SECOND, &Plain, PlainMsg { what: "x".into() });
        assert!(plain.as_ref().is_err_and(|e| e.to_string().contains("not a message family")), "{plain:?}");
        let nul = HeartbeatWatchState::new(SECOND, &Ops, OpsMsg::Page { what: "a\0b".into() });
        assert!(nul.is_ok(), "NUL is refused by the core at the create, not by the constructor");
    }
}
