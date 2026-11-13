//! The scheduler: an actor that sends a stored message to a target on each slot of a [`Schedule`].

use cicada::prelude::*;
use cicada::time::Timestamp;
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};

use crate::supervisor::retry_policy;
use crate::{EncodeError, Schedule, StoredMessage};

/// The header of a delivery for a slot: the slot in UTC to the second, as `2026-11-13T02:00:00Z`. Stable: a target keys
/// its work by it.
const SLOT_HEADER: &str = "cicada-slot";

/// A scheduler, `cicada.scheduler/<name>`: one for each schedule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scheduler {
    /// The name of the schedule: the key of its address.
    pub name: String,
}

impl Scheduler {
    /// The scheduler named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for Scheduler {
    const TYPE: &'static str = "cicada.scheduler";
    type Message = SchedulerMsg;
}

/// The messages of a [`Scheduler`]. `Tick` is its own: it counts only when the scheduler sent it itself and it is the
/// armed tick; otherwise it ends `done` and changes nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum SchedulerMsg {
    /// Cancels the armed tick, if there is one, and arms a `Tick` at the next slot of the schedule. Safe to repeat: one
    /// tick stays armed. It also arms a chain again that ended.
    Start,
    /// Sends the stored message to the target, with the retry policy of the state, and keeps the armed tick. A tick sends
    /// it with its slot, which the delivery carries in the header `cicada-slot`; a client sends it without one, to run
    /// the schedule now, also on a scheduler that was never started.
    Run {
        /// The slot of the tick that sent it; none for a run now.
        #[serde(default)]
        slot: Option<Timestamp>,
    },
    /// Replaces each field it gives and keeps the counts. A new schedule on a started scheduler cancels the armed tick
    /// and arms one at the new schedule's next slot; a new target, message or retry policy counts from the next `Run`,
    /// also one that is already waiting. With no field it changes nothing.
    Update {
        /// The new schedule.
        schedule: Option<Schedule>,
        /// The new target.
        target: Option<ActorAddress>,
        /// The new message.
        message: Option<StoredMessage>,
        /// The new retry policy of the delivered message; a policy of one attempt turns retries off.
        retry: Option<RetryPolicy>,
    },
    /// One slot: arms the next tick and sends the scheduler a `Run` with the slot. Sent by the scheduler to itself, with
    /// a retry policy.
    Tick {
        /// The slot.
        slot: Timestamp,
    },
}

/// The state of a [`Scheduler`]: what it sends, where and when, and what it did. Build it with `SchedulerState::new`.
/// A scheduler is started when `armed_tick` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerState {
    /// When it sends.
    pub schedule: Schedule,
    /// Where it sends.
    pub target: ActorAddress,
    /// What it sends.
    pub message: StoredMessage,
    /// The retry policy of the delivered message; none: one attempt.
    #[serde(default)]
    pub retry: Option<RetryPolicy>,
    /// The slot of the armed tick.
    #[serde(default)]
    pub next_run: Option<Timestamp>,
    /// The id of the armed tick, which `Start` and `Update` cancel. It stays set when the tick goes without running.
    #[serde(default)]
    pub armed_tick: Option<MessageId>,
    /// The deliveries that reached the mailbox of the target.
    #[serde(default)]
    pub runs: u64,
    /// The store's time of the run that sent the last of them.
    #[serde(default)]
    pub last_run: Option<Timestamp>,
}

impl SchedulerState {
    /// A scheduler that sends `message` to `target` on each slot of `schedule`, with no retry policy for it: the
    /// compiler checks that `target` takes the message, as for a send. It is not started: a `Start` arms its first
    /// tick. Fails for a ref without an address, and for a value that is not a message family. Set the retry policy of
    /// the delivered message with `state.retry = Some(policy)`.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Reports;
    /// # impl ActorRef for Reports { const TYPE: &'static str = "reports"; type Message = ReportsMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum ReportsMsg { Build { full: bool } }
    /// use cicada_actors::{Schedule, SchedulerState};
    ///
    /// let state = SchedulerState::new(Schedule::daily(2, 0)?, &Reports, ReportsMsg::Build { full: true })?;
    /// assert_eq!(state.target.as_str(), "reports/_");
    /// assert_eq!((state.message.message_type.as_str(), &state.message.payload), ("Build", &serde_json::json!({"full": true})));
    /// assert_eq!((state.armed_tick, state.runs), (None, 0));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// A message of another family does not compile:
    ///
    /// ```compile_fail
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Reports;
    /// # impl ActorRef for Reports { const TYPE: &'static str = "reports"; type Message = ReportsMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum ReportsMsg { Build { full: bool } }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum MailerMsg { Send { to: String } }
    /// use cicada_actors::{Schedule, SchedulerState};
    ///
    /// let wrong = SchedulerState::new(Schedule::daily(2, 0).unwrap(), &Reports, MailerMsg::Send { to: "a@b.c".into() });
    /// ```
    pub fn new<R: ActorRef>(
        schedule: Schedule,
        target: &R,
        message: impl Into<R::Message>,
    ) -> Result<Self, EncodeError> {
        let address = target.address().map_err(|e| EncodeError(e.to_string()))?;
        Ok(Self {
            schedule,
            target: address,
            message: StoredMessage::for_ref(target, message)?,
            retry: None,
            next_run: None,
            armed_tick: None,
            runs: 0,
            last_run: None,
        })
    }
}

/// The scheduler type. It has no settings: the schedule, the target, the message and its retry policy are in the state
/// of each scheduler, so an operator creates and edits a schedule at runtime. Pause, resume and delete it with the
/// core's control.
///
/// Each slot takes two turns. The `Tick` arms the next tick, at the first slot after the later of the store's time and
/// its own slot, and sends the scheduler a `Run` with its slot; the `Run` sends the stored message to the target. So a
/// target that is gone fails that run alone (the cause `commit`), a message the target does not take is refused for good
/// (`refused`), and the next slot delivers as usual; a `Run` whose read of the store fails is tried again. A failed run
/// is a failed message of the scheduler, which its watchers are told of. Slots missed while no node ran, or while the
/// scheduler was paused, are skipped: a late tick delivers once.
///
/// A chain ends when its armed tick goes without running: a purge of the mailbox, a cancel of the tick, or a tick that
/// fails for good. A `Start`, or an `Update` with a schedule, arms it again.
#[derive(Debug, Clone, Copy, Default)]
pub struct SchedulerActor;

impl Actor for SchedulerActor {
    type Ref = Scheduler;
    type State = SchedulerState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SchedulerMsg) -> HandlerResult {
        match message {
            SchedulerMsg::Start => {
                let next = turn.state().schedule.next(turn.now());
                rearm(turn, next)?;
            }
            SchedulerMsg::Run { slot } => run(turn, slot).await?,
            SchedulerMsg::Update { schedule, target, message, retry } => {
                update(turn, schedule, target, message, retry)?;
            }
            SchedulerMsg::Tick { slot } => {
                let own = turn.message().from.as_ref() == Some(turn.self_address());
                if own && turn.state().armed_tick.as_ref() == Some(&turn.message().id) {
                    // From the slot when the tick runs early, so that it does not arm its own slot again.
                    let next = turn.state().schedule.next(turn.now().max(slot));
                    arm(turn, next)?;
                    turn.send_self_with(
                        SchedulerMsg::Run { slot: Some(slot) },
                        SendOptions::new().retry(retry_policy()),
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Sends the stored message to the target; when that commits, the run is counted.
async fn run(turn: &mut Turn<SchedulerActor>, slot: Option<Timestamp>) -> HandlerResult {
    let state = turn.state();
    let (target, message) = (state.target.clone(), state.message.clone());
    let mut options = SendOptions::new();
    options.retry = state.retry.clone();
    if let Some(slot) = slot {
        options = options.header(SLOT_HEADER, slot.format("%Y-%m-%dT%H:%M:%SZ").to_string());
    }
    match turn.send_raw(&target, &message.message_type, message.payload, options).await {
        Ok(_) => {}
        // Tried again by the retry policy of the run.
        Err(e @ TurnError::Store(_)) => return Err(e.into()),
        // No registered type, a type that does not take the message, or a payload out of bounds.
        Err(e) => return Err(HandlerError::permanent_from(e)),
    }
    let now = turn.now();
    let state = turn.state_mut();
    state.runs = state.runs.saturating_add(1);
    state.last_run = Some(now);
    Ok(())
}

fn update(
    turn: &mut Turn<SchedulerActor>,
    schedule: Option<Schedule>,
    target: Option<ActorAddress>,
    message: Option<StoredMessage>,
    retry: Option<RetryPolicy>,
) -> Result<(), TurnError> {
    if schedule.is_none() && target.is_none() && message.is_none() && retry.is_none() {
        return Ok(());
    }
    let started = turn.state().armed_tick.is_some();
    let state = turn.state_mut();
    if let Some(target) = target {
        state.target = target;
    }
    if let Some(message) = message {
        state.message = message;
    }
    if let Some(retry) = retry {
        state.retry = Some(retry);
    }
    if let Some(schedule) = schedule {
        state.schedule = schedule;
        if started {
            let next = schedule.next(turn.now());
            rearm(turn, next)?;
        }
    }
    Ok(())
}

/// Cancels the armed tick, if there is one, and arms a tick at `slot`.
fn rearm(turn: &mut Turn<SchedulerActor>, slot: Timestamp) -> Result<(), TurnError> {
    if let Some(armed) = turn.state().armed_tick.clone()
        && armed != turn.message().id
    {
        turn.cancel(&armed)?;
    }
    arm(turn, slot)
}

/// Sends the scheduler a `Tick` at `slot`, with the shared retry policy, and keeps it as the armed tick.
fn arm(turn: &mut Turn<SchedulerActor>, slot: Timestamp) -> Result<(), TurnError> {
    let id = turn.send_self_with(SchedulerMsg::Tick { slot }, SendOptions::new().at(slot).retry(retry_policy()))?;
    let state = turn.state_mut();
    state.next_run = Some(slot);
    state.armed_tick = Some(id);
    Ok(())
}
