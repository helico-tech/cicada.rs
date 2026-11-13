//! Durable flows, on virtual time with `TestActors`: one new step per commit, replay and changed replay, retries and
//! terminal failures with their watch notifications, interruption before commit with stable keys, public starts,
//! pokes and recovery, done runs, swallowed suspensions and failures, abandoned, unpolled and panicking step futures,
//! timeouts inside closures, result limits and the journal's count limit. A probe counts every closure invocation.

mod common;

use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::FailureCause;
use cicada::prelude::*;
use cicada::runtime::RetryResult;
use cicada::store::{Message, OutcomeStatus};
use cicada_actors::{Flow, FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, JournalEntry, Step};
use cicada_test::TestActors;
use common::*;
use futures::FutureExt;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};

const SECOND: Duration = Duration::from_secs(1);
/// The backoff of the test flow's policy.
const BACKOFF: Duration = Duration::from_secs(10);
/// Longer than the node's default maximum turn of 15 minutes.
const SLOW: Duration = Duration::from_secs(3600);

/// The test flow's policy: three attempts, ten seconds apart.
fn three_attempts() -> RetryPolicy {
    RetryPolicy::new(3, Backoff::fixed(BACKOFF).unwrap()).unwrap()
}

/// The crate's shared retry policy.
fn shared_policy() -> RetryPolicy {
    RetryPolicy::new(7, Backoff::exponential(SECOND, 2.0, Duration::from_secs(32)).unwrap()).unwrap()
}

/// What the fake service does on its next call for a step id; with nothing planned it succeeds with `r-<id>`.
#[derive(Debug, Clone, Copy)]
enum Act {
    /// An ordinary error.
    Fail,
    /// A permanent error.
    Refuse,
    /// Succeeds after an hour, past the node's maximum turn.
    Slow,
    /// Panics.
    Panic,
}

/// The fake external service: it notes the key of every call and does what is planned for its step id.
#[derive(Clone, Default)]
struct Probe {
    keys: Arc<Mutex<Vec<String>>>,
    plans: Arc<Mutex<HashMap<String, VecDeque<Act>>>>,
}

impl Probe {
    fn plan(&self, id: &str, acts: &[Act]) {
        self.plans.lock().unwrap().insert(id.to_owned(), acts.iter().copied().collect());
    }

    fn record(&self, key: &str) -> Option<Act> {
        self.keys.lock().unwrap().push(key.to_owned());
        let id = key.rsplit_once('/').unwrap().1;
        self.plans.lock().unwrap().get_mut(id).and_then(VecDeque::pop_front)
    }

    async fn call(&self, key: String) -> Result<String, HandlerError> {
        let id = key.rsplit_once('/').unwrap().1.to_owned();
        match self.record(&key) {
            None => Ok(format!("r-{id}")),
            Some(Act::Fail) => Err(HandlerError::msg("the service failed")),
            Some(Act::Refuse) => Err(HandlerError::permanent("the service refused")),
            Some(Act::Slow) => {
                tokio::time::sleep(SLOW).await;
                Ok(format!("r-{id}"))
            }
            Some(Act::Panic) => panic!("the service panicked"),
        }
    }

    /// The keys of every call, in order.
    fn keys(&self) -> Vec<String> {
        self.keys.lock().unwrap().clone()
    }

    /// The step ids of every call, in order.
    fn ids(&self) -> Vec<String> {
        self.keys().iter().map(|k| k.rsplit_once('/').unwrap().1.to_owned()).collect()
    }
}

/// One thing the test flow does.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    /// A step whose result is the service's text.
    Step { id: String },
    /// The steps `<prefix>0` up to `<prefix><count - 1>`.
    Repeat { prefix: String, count: u32 },
    /// A step asked for as a number; a new one saves 7.
    Number { id: String },
    /// A step whose error the flow catches and ignores.
    Swallow { id: String },
    /// A step that the flow times out after a second.
    Abandon { id: String },
    /// A step that loses a `select!` against a one-second timer.
    Select { id: String },
    /// A step future that the flow drops without polling it.
    Unpolled { id: String },
    /// A step whose closure times out its own call after a second.
    InnerTimeout { id: String },
    /// A step whose panic the flow catches.
    Caught { id: String },
    /// A step whose closure panics when it is invoked, caught by the flow.
    PanicInvoke { id: String },
    /// A step whose result panics when it is serialized, caught by the flow.
    PanicSerialize { id: String },
    /// A step whose result is `depth` arrays deep.
    Deep { id: String, depth: u32 },
    /// A step whose result holds a NUL.
    Nul { id: String },
    /// A step whose result does not encode as JSON.
    Unencodable { id: String },
    /// A charge that is declined, as data; the flow then calls `notify`.
    Charge { id: String },
}

/// How the test flow ends after its steps.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum End {
    #[default]
    Ok,
    Fail,
    Hang,
    Panic,
}

/// The input of the test flow: what it does.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Script {
    ops: Vec<Op>,
    #[serde(default)]
    end: End,
}

fn script(ops: Vec<Op>) -> Script {
    Script { ops, end: End::Ok }
}

fn ending(ops: Vec<Op>, end: End) -> Script {
    Script { ops, end }
}

fn steps(ids: &[&str]) -> Vec<Op> {
    ids.iter().map(|id| Op::Step { id: (*id).to_owned() }).collect()
}

fn op(make: fn(String) -> Op, id: &str) -> Op {
    make(id.to_owned())
}

/// A result whose serialization panics.
#[derive(Deserialize)]
struct Exploding;

impl Serialize for Exploding {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        panic!("the result panicked")
    }
}

fn nested(depth: u32) -> Value {
    (0..depth).fold(Value::Null, |inner, _| Value::Array(vec![inner]))
}

fn depth(value: &Value) -> u32 {
    match value {
        Value::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// The test flow, `test.flow`: runs its script against the probe.
struct ScriptFlow {
    probe: Probe,
}

impl Flow for ScriptFlow {
    const TYPE: &'static str = "test.flow";
    type Input = Script;

    fn retry() -> RetryPolicy {
        three_attempts()
    }

    async fn run(&self, step: &mut Step, script: Script) -> HandlerResult {
        for op in script.ops {
            self.op(step, op).await?;
        }
        match script.end {
            End::Ok => Ok(()),
            End::Fail => Err(HandlerError::msg("the flow failed")),
            End::Hang => {
                tokio::time::sleep(SLOW).await;
                Ok(())
            }
            End::Panic => panic!("the flow panicked"),
        }
    }
}

impl ScriptFlow {
    async fn op(&self, step: &mut Step, op: Op) -> HandlerResult {
        let p = &self.probe;
        match op {
            Op::Step { id } => {
                step.run(&id, |key| p.call(key)).await?;
            }
            Op::Repeat { prefix, count } => {
                for i in 0..count {
                    step.run(&format!("{prefix}{i}"), |key| p.call(key)).await?;
                }
            }
            Op::Number { id } => {
                let _: u32 = step.run(&id, |key| async move { p.call(key).await.map(|_| 7) }).await?;
            }
            Op::Swallow { id } => {
                let _ = step.run(&id, |key| p.call(key)).await;
            }
            Op::Abandon { id } => {
                let _ = tokio::time::timeout(SECOND, step.run(&id, |key| p.call(key))).await;
            }
            Op::Select { id } => {
                tokio::select! {
                    biased;
                    _ = step.run(&id, |key| p.call(key)) => {}
                    _ = tokio::time::sleep(SECOND) => {}
                }
            }
            Op::Unpolled { id } => drop(step.run(&id, |key| p.call(key))),
            Op::InnerTimeout { id } => {
                step.run(&id, |key| async move {
                    match tokio::time::timeout(SECOND, p.call(key)).await {
                        Ok(result) => result,
                        Err(elapsed) => Err(HandlerError::new(elapsed)),
                    }
                })
                .await?;
            }
            Op::Caught { id } => {
                let _ = AssertUnwindSafe(step.run(&id, |key| p.call(key))).catch_unwind().await;
            }
            Op::PanicInvoke { id } => {
                let invoke = |key: String| -> std::future::Ready<Result<String, HandlerError>> {
                    p.record(&key);
                    panic!("the invocation panicked")
                };
                let _ = AssertUnwindSafe(step.run(&id, invoke)).catch_unwind().await;
            }
            Op::PanicSerialize { id } => {
                let call = |key: String| async move {
                    p.record(&key);
                    Ok(Exploding)
                };
                let _ = AssertUnwindSafe(step.run(&id, call)).catch_unwind().await;
            }
            Op::Deep { id, depth } => {
                step.run(&id, |key| async move { p.call(key).await.map(|_| nested(depth)) }).await?;
            }
            Op::Nul { id } => {
                step.run(&id, |key| async move { p.call(key).await.map(|_| "a\0b".to_owned()) }).await?;
            }
            Op::Unencodable { id } => {
                let call = |key: String| async move { p.call(key).await.map(|_| HashMap::from([((1u8, 2u8), 3u8)])) };
                step.run(&id, call).await?;
            }
            Op::Charge { id } => {
                let charged: Result<String, String> =
                    step.run(&id, |key| async move { p.call(key).await.map(|_| Err("declined".to_owned())) }).await?;
                if charged.is_err() {
                    step.run("notify", |key| p.call(key)).await?;
                }
            }
        }
        Ok(())
    }
}

/// `test.plain`: one step, with the default retry policy.
struct PlainFlow;

impl Flow for PlainFlow {
    const TYPE: &'static str = "test.plain";
    type Input = String;

    async fn run(&self, step: &mut Step, _input: String) -> HandlerResult {
        step.run("only", |_key| async { Ok(()) }).await
    }
}

/// `producer/_`: creates a run and sends it `Continue` in one turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Producer;

impl ActorRef for Producer {
    const TYPE: &'static str = "producer";
    type Message = ProducerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ProducerMsg {
    Go { key: String, script: Script },
}

struct ProducerActor;

impl Actor for ProducerActor {
    type Ref = Producer;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProducerMsg) -> HandlerResult {
        let ProducerMsg::Go { key, script } = message;
        turn.create(&run(&key), &FlowState::new(script))?;
        turn.send(&run(&key), FlowMsg::Continue)?;
        Ok(())
    }
}

fn run(key: &str) -> FlowRun<ScriptFlow> {
    FlowRun::named(key)
}

/// A node with the test flow on `probe`, the plain flow, the producer and the witness, which watches `test.flow`.
async fn node(probe: &Probe) -> TestActors {
    let probe = probe.clone();
    let test = TestActors::builder()
        .with(move |node| {
            node.host(FlowActor::new(ScriptFlow { probe }))
                .host(FlowActor::new(PlainFlow))
                .host(ProducerActor)
                .host(WitnessActor)
        })
        .build()
        .await
        .unwrap();
    test.create(&Producer, &NoState {}).await;
    witness(&test, "test.flow").await;
    test
}

/// Creates the run `key` with `state` and sends it a public `Continue`.
async fn start_with(test: &TestActors, key: &str, state: FlowState<Script>) -> Message {
    test.create(&run(key), &state).await;
    test.send(&run(key), FlowMsg::Continue).await
}

async fn start(test: &TestActors, key: &str, script: Script) -> Message {
    start_with(test, key, FlowState::new(script)).await
}

async fn state(test: &TestActors, key: &str) -> FlowState<Script> {
    test.state(&run(key)).await
}

/// The armed `Continue` of `key`, which must be pending.
async fn armed(test: &TestActors, key: &str) -> Message {
    let id = state(test, key).await.armed.expect("an armed continuation");
    test.pending(&run(key)).await.into_iter().find(|m| m.id == id).expect("the armed continuation is pending")
}

async fn outcome(test: &TestActors, id: &MessageId) -> Option<cicada::store::OutcomeRecord> {
    test.system().outcome(id).await.unwrap()
}

/// The ids of the journal of `key`.
async fn journal(test: &TestActors, key: &str) -> Vec<String> {
    state(test, key).await.journal.iter().map(|e| e.id().to_owned()).collect()
}

fn saved(id: &str, result: Value) -> JournalEntry {
    JournalEntry::Step { id: id.to_owned(), result }
}

/// A running state of `script` that has saved `journal`.
fn restored(script: Script, journal: Vec<JournalEntry>) -> FlowState<Script> {
    FlowState { journal, ..FlowState::new(script) }
}

/// Asserts that the armed `Continue` of `key` ended `failed` for good, at its first attempt, with `why` in its error,
/// that the witness was told with the cause `refused`, and that nothing is pending.
async fn refused(test: &TestActors, key: &str, why: &str) {
    let id = state(test, key).await.armed.expect("the failed continuation stays armed");
    let record = outcome(test, &id).await.expect("an outcome");
    assert_eq!(record.status, OutcomeStatus::Failed, "{key}");
    let error = record.error.unwrap();
    assert!(error.contains(why), "{key}: {error} lacks {why}");
    assert_eq!(record.envelope.failures, 0, "{key}: no retry");
    let told = told(test).await;
    assert!(told.iter().any(|f| f.message_id == id && f.cause == FailureCause::Refused), "{key}: {told:?}");
    assert!(test.pending(&run(key)).await.is_empty(), "{key}: no continuation");
}

// One new step per commit.

/// Scenario: Two steps and completion.
#[tokio::test(start_paused = true)]
async fn two_steps_commit_one_at_a_time_and_a_replay_marks_the_run_done() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let poke = start(&test, "k", script(steps(&["a", "b"]))).await;
    let mut ids = Vec::new();
    for (turn, (saved, calls)) in
        [(vec![], vec![]), (vec!["a"], vec!["a"]), (vec!["a", "b"], vec!["a", "b"])].into_iter().enumerate()
    {
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
        let current = state(&test, "k").await;
        assert_eq!(journal(&test, "k").await, saved, "turn {turn}");
        assert_eq!(probe.ids(), calls, "turn {turn}: no second closure in a turn");
        assert_eq!(current.status, FlowStatus::Running);
        assert_eq!(test.version(&run("k")).await, turn as i64 + 1, "one commit per turn");
        let next = armed(&test, "k").await;
        assert_eq!(test.pending(&run("k")).await.len(), 1, "one continuation");
        ids.push(next.id);
    }
    assert_eq!(outcome(&test, &poke.id).await.unwrap().status, OutcomeStatus::Done);
    assert!(ids[0] != ids[1] && ids[1] != ids[2], "each suspension arms a new continuation");

    assert_eq!(test.run_until_idle().await, 1, "the last execution replays both and completes");
    let done = state(&test, "k").await;
    assert_eq!((done.status, done.armed.clone()), (FlowStatus::Done, None));
    assert_eq!(probe.ids(), ["a", "b"]);
    assert_eq!(done.result::<String>("a").unwrap().unwrap(), "r-a");
    assert_eq!(done.result::<String>("b").unwrap().unwrap(), "r-b");
    assert_eq!(test.version(&run("k")).await, 4);
    for id in &ids {
        assert_eq!(outcome(&test, id).await.unwrap().status, OutcomeStatus::Done);
    }
}

/// Scenario: Host start, when create and send occur in one producer turn.
#[tokio::test(start_paused = true)]
async fn a_producer_creates_and_starts_a_run_in_one_turn() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.send(&Producer, ProducerMsg::Go { key: "p".into(), script: script(steps(&["a"])) }).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "p").await.status, FlowStatus::Done);
    assert_eq!(probe.keys(), ["test.flow/p/a"]);
}

// Replay.

/// Scenario: Retry a later step; and Retrying the armed execution.
#[tokio::test(start_paused = true)]
async fn a_later_step_retries_without_running_the_committed_ones() {
    let probe = Probe::default();
    probe.plan("c", &[Act::Fail]);
    let test = node(&probe).await;
    start(&test, "k", script(steps(&["a", "b", "c"]))).await;
    test.run_until_idle().await;
    assert_eq!(probe.ids(), ["a", "b", "c"]);
    assert_eq!(journal(&test, "k").await, ["a", "b"]);
    let failed = armed(&test, "k").await;
    assert_eq!((failed.failures, failed.last_error.as_deref()), (1, Some("the service failed")));

    assert_eq!(test.advance_time_by(BACKOFF).await, 2, "the retry and the completion; no normalization turn");
    assert_eq!(probe.ids(), ["a", "b", "c", "c"], "the committed steps are replayed, not run");
    let done = outcome(&test, &failed.id).await.unwrap();
    assert_eq!((done.status, done.envelope.attempt), (OutcomeStatus::Done, 2), "the same message, retried");
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// Scenario: Restored progress.
#[tokio::test(start_paused = true)]
async fn restored_progress_replays_saved_values_without_calling_their_closures() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let progress = restored(script(steps(&["a", "b"])), vec![saved("a", json!("from-a-host"))]);
    start_with(&test, "k", progress).await;
    test.run_until_idle().await;
    assert_eq!(probe.ids(), ["b"]);
    let done = state(&test, "k").await;
    assert_eq!(done.status, FlowStatus::Done);
    assert_eq!(done.result::<String>("a").unwrap().unwrap(), "from-a-host");
}

/// Scenario: Changed replay.
#[tokio::test(start_paused = true)]
async fn changed_flow_code_fails_for_good_without_progress() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let sleep = JournalEntry::Sleep { id: "w".into(), until: "2026-11-14T00:00:00Z".parse().unwrap() };
    let cases = [
        (
            "order",
            steps(&["a", "b"]),
            vec![saved("b", json!("r-b"))],
            "calls step `a` where its journal holds step `b`",
        ),
        ("kind", steps(&["w"]), vec![sleep], "holds sleep `w`"),
        ("type", vec![op(|id| Op::Number { id }, "a")], vec![saved("a", json!("text"))], "does not decode"),
        ("repeat", steps(&["a", "a"]), vec![saved("a", json!("r-a"))], "calls the id `a` twice"),
        ("early", steps(&["a"]), vec![saved("a", json!("r-a")), saved("b", json!("r-b"))], "returned before reaching"),
    ];
    for (key, ops, saved, why) in cases {
        let journal_before = saved.clone();
        start_with(&test, key, restored(script(ops), saved)).await;
        test.run_until_idle().await;
        refused(&test, key, why).await;
        assert_eq!(state(&test, key).await.journal, journal_before, "{key}: no new progress");
        assert_eq!(state(&test, key).await.status, FlowStatus::Running, "{key}");
    }
    assert!(probe.keys().is_empty(), "no closure ran");
}

// Interruption before commit.

/// Scenario: Lost progress before commit; and Key stability and escaping.
#[tokio::test(start_paused = true)]
async fn a_success_whose_turn_does_not_commit_is_called_again_with_the_same_key() {
    let probe = Probe::default();
    probe.plan("a", &[Act::Slow]);
    let test = node(&probe).await;
    start(&test, "k/1", script(steps(&["a"]))).await;
    test.run_until_idle().await;
    assert_eq!(probe.keys(), ["test.flow/k%2F1/a"], "the call succeeded, but past the turn's maximum");
    assert!(journal(&test, "k/1").await.is_empty(), "nothing saved");
    let retried = armed(&test, "k/1").await;
    assert_eq!((retried.failures, retried.last_error.as_deref()), (1, Some("timeout")));

    test.advance_time_by(BACKOFF).await;
    assert_eq!(probe.keys(), ["test.flow/k%2F1/a", "test.flow/k%2F1/a"], "called again with the same key");
    assert_eq!(state(&test, "k/1").await.status, FlowStatus::Done);
}

/// Scenario: Key stability and escaping, through a redrive; and Reusing a deleted run address.
#[tokio::test(start_paused = true)]
async fn step_keys_are_stable_across_retries_redrives_and_recreation() {
    let probe = Probe::default();
    probe.plan("x", &[Act::Fail, Act::Fail, Act::Fail]);
    let test = node(&probe).await;
    start(&test, "a/b", script(steps(&["x"]))).await;
    test.run_until_idle().await;
    test.advance_time_by(BACKOFF * 2).await;
    let failed = state(&test, "a/b").await.armed.unwrap();
    assert_eq!(outcome(&test, &failed).await.unwrap().status, OutcomeStatus::Failed);
    let RetryResult::Accepted(_) = test.system().retry_message(&failed).await.unwrap() else { panic!("redriven") };
    test.run_until_idle().await;
    assert_eq!(state(&test, "a/b").await.status, FlowStatus::Done);

    test.system().delete_actor(&addr(&run("a/b"))).await.unwrap();
    start(&test, "a/b", script(steps(&["x"]))).await;
    test.run_until_idle().await;
    assert_eq!(probe.keys(), vec!["test.flow/a%2Fb/x"; 5], "three attempts, a redrive and a recreated run");
}

// Public Continue.

/// Scenario: Initial retry policy.
#[tokio::test(start_paused = true)]
async fn a_public_continue_arms_the_flow_policy_and_keeps_the_correlation_id() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&run("k"), &FlowState::new(script(steps(&["a", "b"])))).await;
    let poke = test.send_with(&run("k"), FlowMsg::Continue, SendOptions::new().correlation_id("c1")).await;
    assert_eq!(poke.retry, None);
    for turn in 0..3 {
        test.system().run_due_turns(1).await.unwrap();
        let next = armed(&test, "k").await;
        assert_eq!(next.retry, Some(three_attempts()), "turn {turn}");
        assert_eq!(next.correlation_id.as_deref(), Some("c1"), "turn {turn}");
        assert_eq!(next.from, Some(addr(&run("k"))), "turn {turn}");
    }
    test.run_until_idle().await;
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);

    let plain = FlowRun::<PlainFlow>::named("p");
    test.create(&plain, &FlowState::new("in".to_owned())).await;
    test.send(&plain, FlowMsg::Continue).await;
    test.system().run_due_turns(1).await.unwrap();
    let pending = test.pending(&plain).await;
    assert_eq!(pending[0].retry, Some(shared_policy()), "the default is the crate's shared policy");
    test.run_until_idle().await;
    let done: FlowState<String> = test.state(&plain).await;
    assert_eq!((done.status, done.result::<()>("only").map(|r| r.is_ok())), (FlowStatus::Done, Some(true)));
}

/// Scenario: Multiple public pokes.
#[tokio::test(start_paused = true)]
async fn queued_pokes_replace_the_armed_continuation_without_running_flow_code() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&run("k"), &FlowState::new(script(steps(&["a"])))).await;
    for _ in 0..3 {
        test.send(&run("k"), FlowMsg::Continue).await;
    }
    let mut armed_ids = Vec::new();
    for turn in 0..3 {
        test.system().run_due_turns(1).await.unwrap();
        armed_ids.push(state(&test, "k").await.armed.unwrap());
        assert!(probe.keys().is_empty(), "turn {turn}: no flow code");
        let continuations = test.pending(&run("k")).await;
        assert_eq!(continuations.len(), 3 - turn, "turn {turn}: the other pokes and one armed continuation");
    }
    for id in &armed_ids[..2] {
        assert_eq!(outcome(&test, id).await.unwrap().status, OutcomeStatus::Cancelled, "replaced");
    }
    test.run_until_idle().await;
    assert_eq!(probe.ids(), ["a"]);
    assert_eq!(outcome(&test, &armed_ids[2]).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

// Completed and stalled runs.

/// Scenario: Completed run.
#[tokio::test(start_paused = true)]
async fn a_done_run_ignores_continue_and_keeps_its_results() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(steps(&["a"]))).await;
    test.run_until_idle().await;
    let version = test.version(&run("k")).await;
    let poke = test.send(&run("k"), FlowMsg::Continue).await;
    assert_eq!(test.run_until_idle().await, 1);
    assert_eq!(outcome(&test, &poke.id).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(probe.ids(), ["a"]);
    assert_eq!(test.version(&run("k")).await, version, "nothing written");
    assert!(test.pending(&run("k")).await.is_empty());
    assert_eq!(state(&test, "k").await.result::<String>("a").unwrap().unwrap(), "r-a");
}

/// Scenario: Stalled continuation (cancelled, purged, failed for good) and Out of retries.
#[tokio::test(start_paused = true)]
async fn a_stalled_continuation_is_recovered_by_a_public_continue() {
    for how in ["cancel", "purge", "fail"] {
        let probe = Probe::default();
        if how == "fail" {
            probe.plan("y", &[Act::Fail, Act::Fail, Act::Fail]);
        }
        let test = node(&probe).await;
        start(&test, "k", script(steps(&["x", "y"]))).await;
        test.system().run_due_turns(1).await.unwrap();
        test.system().run_due_turns(1).await.unwrap();
        let stalled = armed(&test, "k").await;
        match how {
            "cancel" => drop(test.system().cancel(&stalled.id).await.unwrap()),
            "purge" => drop(test.system().purge_mailbox(&addr(&run("k"))).await.unwrap()),
            _ => {
                test.run_until_idle().await;
                test.advance_time_by(BACKOFF * 2).await;
                let record = outcome(&test, &stalled.id).await.unwrap();
                assert_eq!((record.status, record.envelope.attempt), (OutcomeStatus::Failed, 3));
                let told = told(&test).await;
                assert_eq!(told.len(), 1);
                assert_eq!(
                    (&told[0].message_id, told[0].cause, told[0].attempts),
                    (&stalled.id, FailureCause::Handler, 3)
                );
            }
        }
        test.run_until_idle().await;
        let held = state(&test, "k").await;
        assert_eq!((held.status, held.armed.as_ref()), (FlowStatus::Running, Some(&stalled.id)), "{how}: stalled");
        assert_eq!(journal(&test, "k").await, ["x"], "{how}: saved progress stays");
        assert!(test.pending(&run("k")).await.is_empty(), "{how}");

        test.send(&run("k"), FlowMsg::Continue).await;
        test.system().run_due_turns(1).await.unwrap();
        let fresh = armed(&test, "k").await;
        assert!(fresh.id != stalled.id && fresh.failures == 0, "{how}: a fresh armed message");
        assert_eq!(fresh.retry, Some(three_attempts()), "{how}");
        test.run_until_idle().await;
        assert_eq!(state(&test, "k").await.status, FlowStatus::Done, "{how}");
        let ys = if how == "fail" { 4 } else { 1 };
        let mut expected = vec!["x".to_owned()];
        expected.extend(std::iter::repeat_n("y".to_owned(), ys));
        assert_eq!(probe.ids(), expected, "{how}: x is replayed, not run");
    }
}

// The latch.

/// Scenario: Swallowed suspension.
#[tokio::test(start_paused = true)]
async fn a_swallowed_suspension_commits_only_the_first_new_step() {
    for end in [End::Ok, End::Fail] {
        let probe = Probe::default();
        let test = node(&probe).await;
        start(&test, "k", ending(vec![op(|id| Op::Swallow { id }, "a"), op(|id| Op::Step { id }, "b")], end)).await;
        test.system().run_due_turns(1).await.unwrap();
        let first = armed(&test, "k").await;
        test.system().run_due_turns(1).await.unwrap();
        assert_eq!(probe.ids(), ["a"], "{end:?}: only the first closure ran");
        assert_eq!(journal(&test, "k").await, ["a"], "{end:?}");
        assert_eq!(outcome(&test, &first.id).await.unwrap().status, OutcomeStatus::Done, "{end:?}");
        armed(&test, "k").await;
        test.system().run_due_turns(1).await.unwrap();
        assert_eq!(
            (probe.ids(), journal(&test, "k").await),
            (vec!["a".into(), "b".into()], vec!["a".into(), "b".into()])
        );
    }
}

/// Scenario: Swallowed step failure.
#[tokio::test(start_paused = true)]
async fn a_swallowed_step_failure_still_fails_the_turn_with_its_own_retryability() {
    let probe = Probe::default();
    probe.plan("a", &[Act::Fail]);
    probe.plan("b", &[Act::Refuse]);
    let test = node(&probe).await;
    start(&test, "transient", script(vec![op(|id| Op::Swallow { id }, "a")])).await;
    start(&test, "permanent", script(vec![op(|id| Op::Swallow { id }, "b")])).await;
    test.run_until_idle().await;
    let retried = armed(&test, "transient").await;
    assert_eq!((retried.failures, retried.last_error.as_deref()), (1, Some("the service failed")));
    assert!(journal(&test, "transient").await.is_empty());
    refused(&test, "permanent", "the service refused").await;
    assert!(journal(&test, "permanent").await.is_empty());
}

/// Scenario: Business error as data.
#[tokio::test(start_paused = true)]
async fn a_business_error_is_saved_as_data_and_branches_on_replay() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(vec![op(|id| Op::Charge { id }, "charge")])).await;
    test.run_until_idle().await;
    let done = state(&test, "k").await;
    assert_eq!(done.status, FlowStatus::Done);
    assert_eq!(done.journal[0], saved("charge", json!({"Err": "declined"})));
    assert_eq!(done.result::<Result<String, String>>("charge").unwrap().unwrap(), Err("declined".into()));
    assert_eq!(probe.ids(), ["charge", "notify"]);
}

/// Scenario: Misuse that never returns.
#[tokio::test(start_paused = true)]
async fn a_swallowed_suspension_then_a_hang_or_panic_follows_core_rules_and_commits_nothing() {
    for (end, error) in [(End::Hang, "timeout"), (End::Panic, "panic: the flow panicked")] {
        let probe = Probe::default();
        let test = node(&probe).await;
        start(&test, "k", ending(vec![op(|id| Op::Swallow { id }, "a")], end)).await;
        test.run_until_idle().await;
        assert_eq!(probe.ids(), ["a"], "{end:?}");
        assert!(journal(&test, "k").await.is_empty(), "{end:?}: the latched result does not commit");
        let retried = armed(&test, "k").await;
        assert_eq!((retried.failures, retried.last_error.as_deref()), (1, Some(error)), "{end:?}");
    }
}

// Abandoned step futures.

/// Scenarios: External timeout of a step; and Select cancellation followed by another step.
#[tokio::test(start_paused = true)]
async fn an_abandoned_step_future_fails_the_turn_for_good() {
    let probe = Probe::default();
    for key in ["timeout-ok", "timeout-err", "select"] {
        probe.plan(key, &[Act::Slow]);
    }
    let test = node(&probe).await;
    let abandon = |id: &str| op(|id| Op::Abandon { id }, id);
    start(&test, "timeout-ok", script(vec![abandon("timeout-ok")])).await;
    start(&test, "timeout-err", ending(vec![abandon("timeout-err")], End::Fail)).await;
    start(&test, "select", script(vec![op(|id| Op::Select { id }, "select"), op(|id| Op::Step { id }, "after")])).await;
    test.run_until_idle().await;
    for key in ["timeout-ok", "timeout-err", "select"] {
        refused(&test, key, &format!("step `{key}` was abandoned")).await;
        assert!(journal(&test, key).await.is_empty(), "{key}");
    }
    assert_eq!(probe.ids().len(), 3, "the second step after a select is not invoked: {:?}", probe.ids());
    assert!(!probe.ids().contains(&"after".to_owned()));
}

/// Scenario: A timeout inside the closure.
#[tokio::test(start_paused = true)]
async fn a_timeout_inside_the_closure_follows_the_flow_policy() {
    let probe = Probe::default();
    probe.plan("a", &[Act::Slow]);
    let test = node(&probe).await;
    start(&test, "k", script(vec![op(|id| Op::InnerTimeout { id }, "a")])).await;
    test.run_until_idle().await;
    let retried = armed(&test, "k").await;
    assert_eq!((retried.failures, retried.last_error.as_deref()), (1, Some("deadline has elapsed")));
    test.advance_time_by(BACKOFF).await;
    assert_eq!(probe.ids(), ["a", "a"]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// Scenario: An unpolled future.
#[tokio::test(start_paused = true)]
async fn an_unpolled_step_future_changes_nothing() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(vec![op(|id| Op::Unpolled { id }, "a"), op(|id| Op::Step { id }, "b")])).await;
    test.run_until_idle().await;
    assert_eq!(probe.ids(), ["b"]);
    assert_eq!(journal(&test, "k").await, ["b"]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// Scenario: A caught step panic, in the closure's invocation, its future and its result's serialization.
#[tokio::test(start_paused = true)]
async fn a_caught_step_panic_fails_the_turn_for_good() {
    let probe = Probe::default();
    probe.plan("future", &[Act::Panic]);
    let test = node(&probe).await;
    start(&test, "invoke", script(vec![op(|id| Op::PanicInvoke { id }, "invoke")])).await;
    start(&test, "future", script(vec![op(|id| Op::Caught { id }, "future")])).await;
    start(&test, "serialize", script(vec![op(|id| Op::PanicSerialize { id }, "serialize")])).await;
    test.run_until_idle().await;
    for key in ["invoke", "future", "serialize"] {
        refused(&test, key, &format!("step `{key}` was abandoned")).await;
        assert!(journal(&test, key).await.is_empty(), "{key}");
    }
    let mut called = probe.ids();
    called.sort();
    assert_eq!(called, ["future", "invoke", "serialize"], "each closure once");
}

// Limits.

/// Scenario: Result boundaries.
#[tokio::test(start_paused = true)]
async fn result_depth_and_encoding_are_checked_before_saving() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let x = || op(|id| Op::Step { id }, "x");
    start(&test, "deep", script(vec![x(), Op::Deep { id: "d".into(), depth: 61 }])).await;
    let cases = [
        ("too-deep", Op::Deep { id: "d".into(), depth: 62 }, "more than 61 arrays and objects deep"),
        ("nul", op(|id| Op::Nul { id }, "s"), "a string holds a NUL character"),
        ("unencodable", op(|id| Op::Unencodable { id }, "u"), "does not encode"),
    ];
    for (key, bad, _) in &cases {
        start(&test, key, script(vec![x(), bad.clone()])).await;
    }
    test.run_until_idle().await;

    let deep = state(&test, "deep").await;
    assert_eq!(deep.status, FlowStatus::Done, "61 levels are saved and replayed");
    assert_eq!(depth(&deep.result::<Value>("d").unwrap().unwrap()), 61);
    for (key, _, why) in cases {
        refused(&test, key, why).await;
        assert_eq!(journal(&test, key).await, ["x"], "{key}: the earlier entry stays");
    }
}

/// Scenario: Step count limit.
#[tokio::test(start_paused = true)]
async fn a_full_journal_refuses_a_new_step_without_invoking_it() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let full: Vec<JournalEntry> = (0..1000).map(|i| saved(&format!("s{i}"), json!(format!("r-s{i}")))).collect();
    let repeat = || Op::Repeat { prefix: "s".into(), count: 1000 };
    start_with(&test, "replay", restored(script(vec![repeat()]), full.clone())).await;
    start_with(&test, "over", restored(script(vec![repeat(), op(|id| Op::Step { id }, "next")]), full)).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "replay").await.status, FlowStatus::Done, "1000 saved entries replay");
    refused(&test, "over", "would be entry 1001 of a journal that holds at most 1000").await;
    assert!(probe.keys().is_empty());
}

/// A unit result is saved as `null`.
#[tokio::test(start_paused = true)]
async fn a_unit_result_is_saved_as_null() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let plain = FlowRun::<PlainFlow>::named("u");
    test.create(&plain, &FlowState::new("in".to_owned())).await;
    test.send(&plain, FlowMsg::Continue).await;
    test.run_until_idle().await;
    let done = test.state_json(&plain).await.unwrap();
    assert_eq!(done["journal"], json!([{"kind": "step", "id": "only", "result": null}]));
    assert_eq!((done["status"].clone(), done["armed"].clone()), (json!("done"), Value::Null));
}
