//! Actor types for end-to-end and fault tests. They record *which node* ran a turn and with which `message.attempt`, so
//! a test driving the HTTP API only can observe recovery after a node was killed or paused:
//!
//! - [`Slow`]: a handler that takes as long as the payload says, so a test can kill or pause its node mid-turn.
//! - [`Counter`]: serial-turn (G1) and order (G5) checks: records every turn's seq, node and time span, flags overlaps.
//! - [`Recorder`]: an external side effect (the per-node [`EffectLog`], served at `GET /e2e/effects`) before a sleep,
//!   and the same record in state; comparing both shows at-least-once execution vs exactly-once effects (G4).
//! - [`Flaky`]: fails a given number of times; senders attach a retry policy ([`flaky_retry`]: 3 attempts, 1 s apart)
//!   or none, for `/outcomes` and retry checks.
//!
//! The node id is a field of each actor value, which does not change the contract: contracts come from the types
//! only, so every node registers the same hash.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::Utc;
use cicada::prelude::*;
use cicada::time::Timestamp;
use serde::{Deserialize, Serialize};

/// A node's in-memory log of external side effects; what a crashed node did is lost with it, as in reality. Cheap to
/// clone: clones share the log.
#[derive(Debug, Clone, Default)]
pub struct EffectLog {
    effects: Arc<Mutex<Vec<Effect>>>,
}

/// One side effect, recorded when it happened (before the turn committed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    /// The node that ran the turn.
    pub node: String,
    /// The actor's URI.
    pub actor: String,
    /// The message's id.
    pub message_id: String,
    /// The message's `attempt`.
    pub attempt: u32,
    /// The label the message carried.
    pub label: Option<String>,
    /// When it happened (the node's clock).
    pub at: Timestamp,
}

impl EffectLog {
    /// A new, empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an effect.
    pub fn record(&self, effect: Effect) {
        self.effects.lock().unwrap_or_else(PoisonError::into_inner).push(effect);
    }

    /// All effects, or only those of `actor`, in the order they happened.
    pub fn list(&self, actor: Option<&str>) -> Vec<Effect> {
        let effects = self.effects.lock().unwrap_or_else(PoisonError::into_inner);
        effects.iter().filter(|e| actor.is_none_or(|a| e.actor == a)).cloned().collect()
    }

    fn record_turn<A: Actor>(&self, node: &str, turn: &Turn<A>, label: Option<String>) {
        self.record(Effect {
            node: node.to_owned(),
            actor: turn.self_address().to_string(),
            message_id: turn.message().id.as_str().to_owned(),
            attempt: turn.message().attempt,
            label,
            at: Utc::now(),
        });
    }
}

/// Sleeps `millis` (nothing when not positive). Cancellable: a lost lease or a node stop interrupts it.
async fn sleep_millis(millis: i64) {
    if millis > 0 {
        tokio::time::sleep(Duration::from_millis(millis as u64)).await;
    }
}

// ---- Slow ----

/// A turn that takes as long as asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slow {
    /// The instance id.
    pub id: String,
}

impl ActorRef for Slow {
    const TYPE: &'static str = "e2e.slow";
    type Message = SlowMsg;
}

/// Messages of [`Slow`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum SlowMsg {
    /// Sleeps `millis` (cancellable: a lost lease interrupts it) and records the run.
    Sleep {
        /// How long the turn takes.
        millis: i64,
        /// A label for the run.
        #[serde(default)]
        label: Option<String>,
    },
}

/// One completed run of a [`Slow`] turn: who ran it, which claim it was, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlowRun {
    /// The message's id.
    pub message_id: String,
    /// The message's label.
    pub label: Option<String>,
    /// The message's `attempt`.
    pub attempt: u32,
    /// The node that ran it.
    pub node: String,
    /// The claim time (store time).
    pub claimed_at: Timestamp,
    /// When the sleep ended (the node's clock).
    pub finished_at: Timestamp,
}

/// State of a [`Slow`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SlowState {
    /// Every completed run.
    pub runs: Vec<SlowRun>,
}

/// The slow type.
pub struct SlowActor {
    node: String,
    effects: EffectLog,
}

impl SlowActor {
    /// The slow type on node `node`, recording effects in `effects`.
    pub fn new(node: impl Into<String>, effects: EffectLog) -> Self {
        Self { node: node.into(), effects }
    }
}

impl Actor for SlowActor {
    type Ref = Slow;
    type State = SlowState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SlowMsg) -> HandlerResult {
        match message {
            SlowMsg::Sleep { millis, label } => {
                self.effects.record_turn(&self.node, turn, label.clone());
                sleep_millis(millis).await;
                let run = SlowRun {
                    message_id: turn.message().id.as_str().to_owned(),
                    label,
                    attempt: turn.message().attempt,
                    node: self.node.clone(),
                    claimed_at: turn.now(),
                    finished_at: Utc::now(),
                };
                turn.state_mut().runs.push(run);
            }
        }
        Ok(())
    }
}

// ---- Counter ----

/// Serial and order checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counter {
    /// The instance id.
    pub id: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "e2e.counter";
    type Message = CounterMsg;
}

/// Messages of [`Counter`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum CounterMsg {
    /// One step; a client sending `seq` 1, 2, 3, … one after another expects them handled in that order.
    Step {
        /// The client's sequence number.
        seq: i64,
        /// How long the turn takes, in milliseconds.
        #[serde(default)]
        hold_millis: i64,
    },
}

/// One handled `Step`. `claimedAt` is store time, `finishedAt` the node's clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StepEntry {
    /// The step's seq.
    pub seq: i64,
    /// The node that ran it.
    pub node: String,
    /// The message's `attempt`.
    pub attempt: u32,
    /// The claim time (store time).
    pub claimed_at: Timestamp,
    /// When the turn ended (the node's clock).
    pub finished_at: Timestamp,
}

/// State of a [`Counter`]. `outOfOrder` counts steps whose seq was not larger than the previous one's; `overlaps`
/// counts turns that started while another turn of this instance was running (on this node), or that were claimed
/// before the previous turn finished (any node). Both must stay 0 (G1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct CounterState {
    /// Steps handled.
    pub count: i64,
    /// The seq of the latest step.
    pub last_seq: i64,
    /// Steps that arrived out of order.
    pub out_of_order: i32,
    /// Overlapping turns.
    pub overlaps: i32,
    /// The latest steps, at most 5 000 ([`MAX_ENTRIES`]).
    pub entries: Vec<StepEntry>,
}

/// How many entries a [`CounterState`] keeps.
pub const MAX_ENTRIES: usize = 5_000;

/// The counter type.
pub struct CounterActor {
    node: String,
    /// Instances with a turn running on this node.
    running: Arc<Mutex<HashSet<String>>>,
}

impl CounterActor {
    /// The counter type on node `node`.
    pub fn new(node: impl Into<String>) -> Self {
        Self { node: node.into(), running: Arc::default() }
    }
}

/// Marks an instance as running on this node until dropped (also when the turn is cancelled), unless another turn of
/// the instance had already marked it.
struct Running {
    set: Arc<Mutex<HashSet<String>>>,
    key: String,
    owner: bool,
}

impl Running {
    fn enter(set: &Arc<Mutex<HashSet<String>>>, key: String) -> Self {
        let owner = set.lock().unwrap_or_else(PoisonError::into_inner).insert(key.clone());
        Self { set: set.clone(), key, owner }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.owner {
            self.set.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.key);
        }
    }
}

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        match message {
            CounterMsg::Step { seq, hold_millis } => {
                let running = Running::enter(&self.running, turn.self_address().to_string());
                let overlapping_here = !running.owner;
                sleep_millis(hold_millis).await;
                let now = turn.now();
                let attempt = turn.message().attempt;
                let state = turn.state_mut();
                let overlapping_anywhere = state.entries.last().is_some_and(|previous| now < previous.finished_at);
                state.count += 1;
                if seq <= state.last_seq {
                    state.out_of_order += 1;
                }
                state.last_seq = seq;
                if overlapping_here || overlapping_anywhere {
                    state.overlaps += 1;
                }
                state.entries.push(StepEntry {
                    seq,
                    node: self.node.clone(),
                    attempt,
                    claimed_at: now,
                    finished_at: Utc::now(),
                });
                let excess = state.entries.len().saturating_sub(MAX_ENTRIES);
                state.entries.drain(..excess);
            }
        }
        Ok(())
    }
}

// ---- Recorder ----

/// External side effects vs committed effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorder {
    /// The instance id.
    pub id: String,
}

impl ActorRef for Recorder {
    const TYPE: &'static str = "e2e.recorder";
    type Message = RecorderMsg;
}

/// Messages of [`Recorder`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum RecorderMsg {
    /// Performs an external side effect (the node's [`EffectLog`]), then sleeps `sleepMillis`, then commits the
    /// record.
    Record {
        /// The record's label.
        label: String,
        /// How long to sleep between the effect and the commit, in milliseconds.
        #[serde(default)]
        sleep_millis: i64,
    },
}

/// One committed record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Recorded {
    /// The record's label.
    pub label: String,
    /// The message's id.
    pub message_id: String,
    /// The message's `attempt`.
    pub attempt: u32,
    /// The node that ran it.
    pub node: String,
}

/// State of a [`Recorder`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RecorderState {
    /// Every committed record.
    pub records: Vec<Recorded>,
}

/// The recorder type.
pub struct RecorderActor {
    node: String,
    effects: EffectLog,
}

impl RecorderActor {
    /// The recorder type on node `node`, recording effects in `effects`.
    pub fn new(node: impl Into<String>, effects: EffectLog) -> Self {
        Self { node: node.into(), effects }
    }
}

impl Actor for RecorderActor {
    type Ref = Recorder;
    type State = RecorderState;

    async fn handle(&self, turn: &mut Turn<Self>, message: RecorderMsg) -> HandlerResult {
        match message {
            RecorderMsg::Record { label, sleep_millis: millis } => {
                self.effects.record_turn(&self.node, turn, Some(label.clone()));
                sleep_millis(millis).await;
                let record = Recorded {
                    label,
                    message_id: turn.message().id.as_str().to_owned(),
                    attempt: turn.message().attempt,
                    node: self.node.clone(),
                };
                turn.state_mut().records.push(record);
            }
        }
        Ok(())
    }
}

// ---- Flaky ----

/// Fails on purpose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flaky {
    /// The instance id.
    pub id: String,
}

impl ActorRef for Flaky {
    const TYPE: &'static str = "e2e.flaky";
    type Message = FlakyMsg;
}

/// Messages of [`Flaky`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum FlakyMsg {
    /// Fails the first `times` attempts, then succeeds. Only a message sent with a retry policy is run again: with
    /// [`flaky_retry`] (3 attempts, 1 s apart, so the pending message with its `failures` is easy to observe in
    /// between) `times >= 3` ends `failed`; without a policy any `times >= 1` ends `failed` at once.
    Attempt {
        /// How many attempts fail.
        times: i32,
    },
}

/// State of a [`Flaky`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct FlakyState {
    /// Messages that succeeded.
    pub succeeded: i32,
    /// The attempt that succeeded last.
    pub last_attempt: u32,
}

/// The retry policy the tests attach to `Attempt` messages: 3 attempts, 1 s apart.
pub fn flaky_retry() -> RetryPolicy {
    let backoff = Backoff::fixed(Duration::from_secs(1)).expect("a valid backoff");
    RetryPolicy::new(3, backoff).expect("a valid retry policy")
}

/// The flaky type.
pub struct FlakyActor;

impl Actor for FlakyActor {
    type Ref = Flaky;
    type State = FlakyState;

    async fn handle(&self, turn: &mut Turn<Self>, message: FlakyMsg) -> HandlerResult {
        match message {
            FlakyMsg::Attempt { times } => {
                // A retry runs the same message again; failures counts the earlier runs.
                let attempt = turn.message().failures + 1;
                if i64::from(attempt) <= i64::from(times) {
                    return Err(HandlerError::msg(format!(
                        "failing on purpose (attempt {attempt} of {times} failures)"
                    )));
                }
                let state = turn.state_mut();
                state.succeeded += 1;
                state.last_attempt = attempt;
            }
        }
        Ok(())
    }
}
