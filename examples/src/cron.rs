//! Example 3 — cron job.
//!
//! A `Tick` handler does the work and sends the next `Tick` at the next scheduled time. The work's completion, the
//! state update and the next tick commit atomically, so the chain neither breaks (a crash before the commit re-runs
//! the tick) nor doubles (a tick that committed is never handled again). `Start` is safe to repeat: a tick that is not
//! the one recorded in state is ignored, so a second `Start` cannot create a second chain.
//!
//! The task's own failure must not end the chain: a `Tick` that ends `failed` sends no successor, and the job would
//! stop for good. So the handler takes the task's error, records it in state and still arms the next tick. A turn
//! that is cut off (it ran past `MAX_TURN`) or panics records nothing; the tick's retry policy re-runs it instead.
//!
//! `RunNow` runs the job at once and re-arms the schedule. It cancels the armed tick in the same commit (the state
//! keeps its id), so exactly one tick stays armed: without the cancel, the old tick and the newly armed one could be
//! due at the same slot.

use std::error::Error;
use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use cicada::time::Timestamp;
use cicada_extras::Schedule;
use serde::{Deserialize, Serialize};

/// A named recurring job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronJob {
    /// The job's name.
    pub name: String,
}

impl ActorRef for CronJob {
    const TYPE: &'static str = "cron";
    type Message = CronMsg;
}

/// Messages of [`CronJob`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum CronMsg {
    /// Arms the schedule, unless it is already armed.
    Start,
    /// One scheduled run, due at `scheduled_for`.
    Tick {
        /// The slot.
        scheduled_for: Timestamp,
    },
    /// Runs the job now, outside the schedule, and re-arms the schedule from now on.
    RunNow,
    /// Ends the job: the instance is deleted and its pending tick purged.
    StopJob,
}

/// State of a [`CronJob`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CronState {
    /// When the job runs.
    pub schedule: Schedule,
    /// The tick currently armed; `None` until `Start`.
    #[serde(default)]
    pub next_run: Option<Timestamp>,
    /// Ticks handled, also those whose task failed.
    #[serde(default)]
    pub runs: i32,
    /// The slot of the most recent run.
    #[serde(default)]
    pub last_run: Option<Timestamp>,
    /// Ticks whose task failed.
    #[serde(default)]
    pub failed_runs: i32,
    /// The error of the most recent failed task, or `None` when the last task succeeded.
    #[serde(default)]
    pub last_error: Option<String>,
    /// The id of the armed tick, which `RunNow` cancels when it re-arms.
    #[serde(default)]
    pub armed_tick: Option<MessageId>,
}

impl CronState {
    /// A job on `schedule` that is not armed yet.
    pub fn new(schedule: Schedule) -> Self {
        Self { schedule, next_run: None, runs: 0, last_run: None, failed_runs: 0, last_error: None, armed_tick: None }
    }
}

/// A tick cut off at `MAX_TURN` (or failing outside the task) is tried again this often before the chain gives up:
/// 3 runs, 1 minute apart.
pub fn tick_retry() -> RetryPolicy {
    let backoff = Backoff::fixed(Duration::from_secs(60)).expect("a valid backoff");
    RetryPolicy::new(3, backoff).expect("a valid retry policy")
}

/// The error of a [`CronTask`] run.
pub type TaskError = Box<dyn Error + Send + Sync>;

/// The work a [`CronJob`] does on each tick.
pub trait CronTask: Send + Sync + 'static {
    /// Runs `job` for the slot `scheduled_for`; at least once per slot (G4).
    fn run(&self, job: &str, scheduled_for: Timestamp) -> impl Future<Output = Result<(), TaskError>> + Send;
}

/// The cron type, running `task` on every tick.
pub struct CronJobActor<T> {
    /// The work.
    pub task: T,
}

impl<T: CronTask> Actor for CronJobActor<T> {
    type Ref = CronJob;
    type State = CronState;
    type Message = CronMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(30 * 60));

    async fn handle(&self, turn: &mut Turn<Self>, message: CronMsg) -> HandlerResult {
        match message {
            CronMsg::Start => {
                if turn.state().next_run.is_some() {
                    return Ok(()); // already armed
                }
                let next = turn.state().schedule.next(turn.now());
                let tick = send_tick(turn, next)?;
                let state = turn.state_mut();
                state.next_run = Some(next);
                state.armed_tick = Some(tick);
            }
            CronMsg::Tick { scheduled_for } => {
                if Some(scheduled_for) != turn.state().next_run {
                    return Ok(()); // not the armed tick: a leftover or a duplicate chain
                }
                self.run_and_arm(turn, scheduled_for).await?;
            }
            CronMsg::RunNow => {
                if let Some(armed) = turn.state().armed_tick.clone() {
                    turn.cancel(&armed)?; // replaced by the tick run_and_arm arms
                }
                let now = turn.now();
                self.run_and_arm(turn, now).await?;
            }
            CronMsg::StopJob => turn.stop(),
        }
        Ok(())
    }
}

impl<T: CronTask> CronJobActor<T> {
    /// Runs the task for `slot`, records the run in state and arms the next tick. The task's own failure is recorded,
    /// not returned, so the chain goes on.
    async fn run_and_arm(&self, turn: &mut Turn<Self>, slot: Timestamp) -> HandlerResult {
        let error = self.task.run(&turn.self_ref().name, slot).await.err().map(|e| e.to_string());
        // The next slot after the work, so slots missed while it ran (or while no node was up) are skipped instead of
        // run in a burst. current_time(), not now: now is the claim time and does not move during a long task.
        let next = turn.state().schedule.next(turn.current_time().max(slot));
        let tick = send_tick(turn, next)?;
        let state = turn.state_mut();
        state.next_run = Some(next);
        state.runs += 1;
        state.last_run = Some(slot);
        if error.is_some() {
            state.failed_runs += 1;
        }
        state.last_error = error;
        state.armed_tick = Some(tick);
        Ok(())
    }
}

fn send_tick<T: CronTask>(turn: &mut Turn<CronJobActor<T>>, at: Timestamp) -> Result<MessageId, HandlerError> {
    let options = SendOptions::new().at(at).retry(tick_retry());
    Ok(turn.send_self_with(CronMsg::Tick { scheduled_for: at }, options)?)
}
