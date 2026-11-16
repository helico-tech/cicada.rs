//! Completing a leased turn with the report of its handler (ADR 0038), and the policy that completion shares with the
//! local executor: how a handler's run becomes a report, how a report becomes the decision a commit applies (the
//! retry of a failure, the check of a new own state, the identities of the sends), how a decision becomes the commit
//! request, and which failure replaces a commit the store rejected or refused.

use std::collections::BTreeMap;

use serde_json::Value;

use super::limits;
use crate::actor::RunResult;
use crate::address::ActorAddress;
use crate::message;
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::store::{
    ActorCreate, CommitRequest, Control, Delivery, Message, MessageId, OutgoingMessage, StateChange, TurnOutcome,
};
use crate::watch::{FailureCause, WatchChange};

/// How the handler of a leased turn ended, as its executor reports it to
/// [`ActorSystem::complete_turn`](super::ActorSystem::complete_turn). The serving system derives the outcome from it:
/// a report names no outcome, retry policy, sender or event id.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnReport {
    /// The handler returned: its state and its finished buffer of effects, applied together or not at all.
    Returned {
        /// What happens to the actor's own state.
        state: StateReport,
        /// The effects the handler buffered.
        effects: TurnEffects,
    },
    /// The handler failed, or the turn failed around it: the message is retried under its own policy or ends `failed`,
    /// without effects.
    Failed(TurnFailure),
    /// An operator's cancel request came before the handler ended (the executor decides which came first): the message
    /// ends `cancelled`, without effects.
    Cancelled,
}

/// What a returned handler did to its actor's own state.
#[derive(Debug, Clone, PartialEq)]
pub enum StateReport {
    /// State and version stay as they are.
    Unchanged,
    /// The new state, written with the version increased by 1. It is checked as a turn's new state is: `null`, a NUL
    /// character or a value nested deeper than [`MAX_DEPTH`](super::limits::MAX_DEPTH) fails the message with an error
    /// that starts with `State cannot be stored:` (`Set(Value::Null)` is such a state, not [`Self::Unchanged`]).
    Set(Value),
    /// The actor stops, as with [`Turn::stop`](crate::Turn::stop): it and its descendants go.
    Stop,
}

/// The finished buffer of a returned handler's effects, applied by the commit in the order of
/// [`Turn`](crate::Turn)'s documentation: watches, deletes, creates, purges, pauses and resumes, then sends. Not a log
/// of calls: an executor applies a turn's buffering rules before it reports (a cancel of a message sent in the same
/// turn drops the send, of a pause and a resume of one actor only the last stays). A report that holds a send and a
/// cancel of its id gets the store's order: the cancel finds nothing, and the send is sent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnEffects {
    /// The watches set and removed, with the actor as watcher, in the order of the calls.
    pub watches: Vec<WatchChange>,
    /// The actors to create.
    pub creates: Vec<ActorCreate>,
    /// The messages to send, in order.
    pub sends: Vec<TurnSend>,
    /// Pending messages of the actor to cancel; never the turn's own message.
    pub cancels: Vec<MessageId>,
    /// The control effects: each effect once per actor, and of a pause and a resume of one actor only one.
    pub controls: Vec<Control>,
}

/// A message a returned handler sends. The serving system sets its sender to the leased actor and its `caused_by` to the
/// leased message; the id is the executor's, so that a repeated report sends the same message.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSend {
    /// The target.
    pub to: ActorAddress,
    /// The message type name.
    pub message_type: String,
    /// The payload.
    pub payload: Value,
    /// The message id, drawn once by the executor and kept when it repeats the report.
    pub id: MessageId,
    /// When it becomes due.
    pub delivery: Delivery,
    /// Opaque string passed through.
    pub correlation_id: Option<String>,
    /// Headers passed through.
    pub headers: BTreeMap<String, String>,
    /// The new message's retry policy.
    pub retry: Option<RetryPolicy>,
    /// How long its record is kept once it finished.
    pub retention: Option<Retention>,
}

impl TurnSend {
    /// A message of `message_type` with `payload` for `to`, due now, with a fresh id and nothing else set.
    pub fn new(to: ActorAddress, message_type: impl Into<String>, payload: Value) -> Self {
        Self {
            to,
            message_type: message_type.into(),
            payload,
            id: MessageId::random(),
            delivery: Delivery::Now,
            correlation_id: None,
            headers: BTreeMap::new(),
            retry: None,
            retention: None,
        }
    }
}

/// A failed turn: why (the [`FailureCause`] its watchers are told), whether its message may be retried, and the reason
/// it is recorded with. Only `handler`, `node` and `timeout` may be retryable; `refused`, `message`, `state` and
/// `commit` may be reported as not retryable, and `claim-limit` never.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnFailure {
    /// Why the message fails.
    pub cause: FailureCause,
    /// True when the message's own retry policy may reschedule it.
    pub retryable: bool,
    /// The reason, recorded as the error of a `failed` outcome or as the `lastError` of a reschedule.
    pub reason: String,
}

/// Result of [`ActorSystem::complete_turn`](super::ActorSystem::complete_turn).
#[derive(Debug, Clone, PartialEq)]
pub enum CompleteTurnResult {
    /// The commit applied: the lease is gone and the message has its outcome, or was rescheduled.
    Completed(TurnCompletion),
    /// The store rejected or refused the commit: nothing was applied, the lease is still held, and no failure was
    /// committed. Submit `Failed` with this failure next, and from then on only that report: never the original effects
    /// again.
    Rejected(TurnFailure),
    /// The epoch is not current, its lease was released or completed, or the actor is gone: nothing was applied. A
    /// report repeated after its commit applied gets this answer too, so it is no proof that it did not apply.
    LeaseLost,
}

/// What a completed commit applied.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnCompletion {
    /// How the message ended, or `Rescheduled` when its retry policy put it back.
    pub outcome: TurnOutcome,
    /// Why it failed, for a `Failed` outcome only.
    pub cause: Option<FailureCause>,
    /// The actor's version after the commit; for a stop, the last version before it went.
    pub version: i64,
    /// The sends as enqueued, with their `seq` and effective `deliver_at`.
    pub sent: Vec<Message>,
    /// The cancels that found a pending message, which is now `cancelled`.
    pub cancelled: Vec<MessageId>,
}

/// What a turn decided, before it is committed.
pub(crate) struct Decision {
    pub outcome: TurnOutcome,
    /// Why the message fails, by the table of ADR 0028: read when the outcome is `failed`, and for a reschedule when
    /// the store refuses it and the message fails after all.
    pub cause: FailureCause,
    pub state: StateChange,
    pub creates: Vec<ActorCreate>,
    pub sends: Vec<OutgoingMessage>,
    pub cancels: Vec<MessageId>,
    pub watches: Vec<WatchChange>,
    pub controls: Vec<Control>,
    /// The failure reason when the turn failed: the fallback if the retry's reschedule is refused.
    pub handler_failure: Option<String>,
}

impl Decision {
    /// The message fails for `cause` with `reason`, without effects and without retry.
    pub fn failed(cause: FailureCause, reason: impl Into<String>) -> Self {
        failure_decision(TurnFailure { cause, retryable: false, reason: reason.into() }, None)
    }

    /// A decision without effects: `outcome` is not `failed` (see [`Self::failed`]).
    pub fn of(outcome: TurnOutcome) -> Self {
        Self {
            outcome,
            cause: FailureCause::Handler,
            state: StateChange::Unchanged,
            creates: Vec::new(),
            sends: Vec::new(),
            cancels: Vec::new(),
            watches: Vec::new(),
            controls: Vec::new(),
            handler_failure: None,
        }
    }
}

/// The retry policy and the failures so far of the turn's message, when it has a policy.
pub(crate) type RetrySnapshot<'a> = Option<(&'a RetryPolicy, u32)>;

/// A failed turn without effects. A retryable failure with a policy that allows another run is a reschedule after the
/// policy's backoff for `failures + 1` runs, capped at [`MAX_DELAY`](crate::time::MAX_DELAY); else `failed`.
pub(crate) fn failure_decision(failure: TurnFailure, retry: RetrySnapshot<'_>) -> Decision {
    let TurnFailure { cause, retryable, reason } = failure;
    let retry_delay = match retry {
        Some((policy, failures)) if retryable && policy.retries_after(failures + 1) => {
            Some(policy.backoff().delay(failures + 1).min(crate::time::MAX_DELAY))
        }
        _ => None,
    };
    let outcome = match retry_delay {
        Some(delay) => TurnOutcome::Rescheduled { delivery: Delivery::After(delay), error: reason.clone() },
        None => TurnOutcome::Failed(reason.clone()),
    };
    Decision { cause, handler_failure: Some(reason), ..Decision::of(outcome) }
}

/// The decision of `report` for the turn of `actor` on `message_id`: a failure under `retry`, `cancelled`, or `done`
/// with the new own state checked (one that cannot be stored fails the message, cause `handler`, without effects) and
/// every send from `actor`, caused by `message_id`.
pub(crate) fn report_decision(
    report: TurnReport,
    actor: &ActorAddress,
    message_id: &MessageId,
    retry: RetrySnapshot<'_>,
) -> Decision {
    let (state, effects) = match report {
        TurnReport::Failed(failure) => return failure_decision(failure, retry),
        TurnReport::Cancelled => return Decision::of(TurnOutcome::Cancelled),
        TurnReport::Returned { state, effects } => (state, effects),
    };
    let state = match state {
        StateReport::Unchanged => StateChange::Unchanged,
        StateReport::Stop => StateChange::Stop,
        StateReport::Set(state) => {
            if let Err(e) =
                message::check_not_null("state", &state).and_then(|()| limits::check_json("state", Some(&state)))
            {
                return Decision::failed(FailureCause::Handler, format!("State cannot be stored: {e}"));
            }
            StateChange::NewState(state)
        }
    };
    let sends = effects
        .sends
        .into_iter()
        .map(|s| OutgoingMessage {
            to: s.to,
            message_type: s.message_type,
            payload: s.payload,
            id: s.id,
            from: Some(actor.clone()),
            delivery: s.delivery,
            correlation_id: s.correlation_id,
            headers: s.headers,
            retry: s.retry,
            caused_by: Some(message_id.clone()),
            retention: s.retention,
        })
        .collect();
    Decision {
        outcome: TurnOutcome::Done,
        cause: FailureCause::Handler,
        state,
        creates: effects.creates,
        sends,
        cancels: effects.cancels,
        watches: effects.watches,
        controls: effects.controls,
        handler_failure: None,
    }
}

/// The report of a handler that returned: a stop wins over a new state, and a new state that could not be encoded
/// fails the message, cause `handler`, without retry. The sends lose the sender and cause that the turn gave them;
/// [`report_decision`] gives them back.
pub(crate) fn run_report(run: RunResult) -> TurnReport {
    let effects = run.effects;
    let state = if effects.stop {
        StateReport::Stop
    } else {
        match run.new_state {
            None => StateReport::Unchanged,
            Some(Err(e)) => {
                let reason = format!("State cannot be encoded: {e}");
                return TurnReport::Failed(TurnFailure { cause: FailureCause::Handler, retryable: false, reason });
            }
            Some(Ok(state)) => StateReport::Set(state),
        }
    };
    let sends = effects
        .sends
        .into_iter()
        .map(|s| TurnSend {
            to: s.to,
            message_type: s.message_type,
            payload: s.payload,
            id: s.id,
            delivery: s.delivery,
            correlation_id: s.correlation_id,
            headers: s.headers,
            retry: s.retry,
            retention: s.retention,
        })
        .collect();
    let effects = TurnEffects {
        watches: effects.watches,
        creates: effects.creates,
        sends,
        cancels: effects.cancels,
        controls: effects.controls,
    };
    TurnReport::Returned { state, effects }
}

/// Checks a failure a caller reports: a retryable one only for `handler`, `node` and `timeout`, and `claim-limit`
/// never.
pub(crate) fn check_failure(failure: &TurnFailure) -> Result<(), String> {
    let allowed = match failure.cause {
        FailureCause::Handler | FailureCause::Node | FailureCause::Timeout => true,
        FailureCause::Refused | FailureCause::Message | FailureCause::State | FailureCause::Commit => {
            !failure.retryable
        }
        FailureCause::ClaimLimit => false,
    };
    if allowed {
        return Ok(());
    }
    Err(if failure.cause == FailureCause::ClaimLimit {
        "A reported failure cannot have the cause claim-limit".to_owned()
    } else {
        format!("A failure with the cause {} cannot be retryable", failure.cause)
    })
}

/// Checks the bounds of the pin of each create of a returned report (ADR 0052), naming the first problem.
pub(crate) fn check_pins(report: &TurnReport) -> Result<(), String> {
    let TurnReport::Returned { effects, .. } = report else {
        return Ok(());
    };
    effects.creates.iter().filter_map(|c| c.pinned_to.as_ref()).try_for_each(limits::check_pin)
}

/// Checks the storage limits of a returned report's effects, as a turn checks them when they are called: each send's
/// delivery and envelope, and each create's state for `null`, NUL characters and depth. The first problem.
pub(crate) fn check_effect_limits(report: &TurnReport) -> Result<(), String> {
    let TurnReport::Returned { effects, .. } = report else {
        return Ok(());
    };
    for send in &effects.sends {
        crate::turn::check_delivery(&send.delivery).map_err(|e| e.to_string())?;
        limits::check_envelope(
            &send.id,
            &send.message_type,
            &send.payload,
            send.correlation_id.as_deref(),
            &send.headers,
        )?;
    }
    for create in &effects.creates {
        message::check_not_null("initial state", &create.state)?;
        limits::check_json("initial state", Some(&create.state))?;
    }
    Ok(())
}

/// What the caller of a commit [`prepare`] made needs once the store answered.
pub(crate) struct Prepared {
    /// The outcome as committed: reasons every backend can store, a reschedule's error cut to
    /// [`TurnOutcome::MAX_ERROR_LENGTH`].
    pub outcome: TurnOutcome,
    pub cause: FailureCause,
    pub handler_failure: Option<String>,
    /// True when the commit may write notifications or make messages due beyond its sends: a failure, a stop or a
    /// control effect.
    pub notifies: bool,
    pub controls: Vec<Control>,
}

/// The commit of `decision` for the turn of `actor` under `epoch` on `message_id`, with a new event id.
pub(crate) fn prepare(
    actor: &ActorAddress,
    epoch: i64,
    message_id: &MessageId,
    decision: Decision,
) -> (CommitRequest, Prepared) {
    // Every backend can store the reason (PostgreSQL refuses NUL characters in text). A rescheduled message carries its
    // `lastError` along, so that one is also bounded.
    let outcome = match decision.outcome {
        TurnOutcome::Failed(e) => TurnOutcome::Failed(sanitize(&e)),
        TurnOutcome::Rescheduled { delivery, error } => TurnOutcome::Rescheduled {
            delivery,
            error: sanitize(&error).chars().take(TurnOutcome::MAX_ERROR_LENGTH).collect(),
        },
        outcome @ (TurnOutcome::Done | TurnOutcome::Cancelled) => outcome,
    };
    // A stop removes actors, and a failure is an event too: either may write notifications to watchers here. A control
    // effect may too, and a resume makes messages due.
    let notifies = matches!(outcome, TurnOutcome::Failed(_))
        || matches!(decision.state, StateChange::Stop)
        || !decision.controls.is_empty();
    let controls = decision.controls.clone();
    let request = CommitRequest {
        event: MessageId::random(),
        address: actor.clone(),
        epoch,
        message_id: message_id.clone(),
        outcome: outcome.clone(),
        cause: decision.cause,
        state: decision.state,
        creates: decision.creates,
        sends: decision.sends,
        cancels: decision.cancels,
        watches: decision.watches,
        controls: decision.controls,
    };
    let prepared =
        Prepared { outcome, cause: decision.cause, handler_failure: decision.handler_failure, notifies, controls };
    (request, prepared)
}

/// How the store turned a commit down, with nothing applied and the lease still held.
pub(crate) enum Refusal<'a> {
    /// It rejected the commit: an absent target, an id in use, a create of an unregistered type.
    Rejected(&'a str),
    /// It refused its data ([`StoreError::Invalid`](crate::store::StoreError::Invalid)).
    Invalid(&'a str),
}

/// The failure, without retry, that replaces a commit of `prepared` the store turned down: a refused reschedule keeps
/// the failure and its cause with the refusal appended; anything else fails with cause `commit`.
pub(crate) fn fallback(prepared: &Prepared, refusal: Refusal<'_>) -> TurnFailure {
    let (cause, reason) = match (refusal, &prepared.outcome, &prepared.handler_failure) {
        (Refusal::Invalid(reason), TurnOutcome::Rescheduled { .. }, Some(failure)) => {
            (prepared.cause, format!("{failure} (commit refused: {reason})"))
        }
        (Refusal::Invalid(reason), _, _) => (FailureCause::Commit, format!("Commit refused: {reason}")),
        (Refusal::Rejected(reason), _, _) => (FailureCause::Commit, reason.to_owned()),
    };
    TurnFailure { cause, retryable: false, reason: sanitize(&reason) }
}

/// True when a commit of `outcome` that sent `sent` should wake the dispatch loop: for the messages sent, a rescheduled
/// message (which may be due at once), and what `notifies` covers.
pub(crate) fn wakes(outcome: &TurnOutcome, sent: &[Message], notifies: bool) -> bool {
    !sent.is_empty() || !outcome.is_final() || notifies
}

/// Logs the control effects of a commit that applied: nothing in the store records who controlled an actor (ADR 0024);
/// whether the target was there, the commit does not say.
pub(crate) fn log_controls(actor: &ActorAddress, message_id: &MessageId, controls: &[Control]) {
    for control in controls {
        tracing::info!(
            actor = %actor,
            effect = %control.effect,
            target = %control.actor,
            "Turn of {actor} for {message_id}: {} {}",
            control.effect,
            control.actor
        );
    }
}

/// A failure reason every backend can store.
pub(crate) fn sanitize(reason: &str) -> String {
    reason.replace('\0', "\u{FFFD}")
}
