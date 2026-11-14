//! Durable flows, on virtual time with `TestActors`: one new step per commit, replay and changed replay, retries and
//! terminal failures with their watch notifications, interruption before commit with stable keys, public starts,
//! pokes and recovery, done runs, swallowed suspensions and failures, abandoned, unpolled and panicking step futures,
//! timeouts inside closures, result limits and the journal's count limit; and durable sleeps: the saved wake time,
//! changed-duration replay, early pokes, deliver-now, retries and redrives, lost timers, pause, delete, failures after
//! a sleep and the limits of a new sleep; and sends (ADR 0044): the create, message, entry and continuation of one
//! commit, replay without another message, core create rules, rejected commits and their recovery, refusals, observer
//! headers, and the messages and phases this release refuses; and requests, replies and waits: parked and closed waits,
//! early, late, ignored, duplicate and invalid replies, replies refused at a reserved wait, the sender check (a remote
//! worker's leased target passes it, a dead-letter redrive does not), correlations, pokes, pauses, lost
//! continuations, typed decoding and its recovery, rejected request commits and replies to a deleted run. A probe
//! counts every closure invocation and notes what each wait returned.

mod common;

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::FailureCause;
use cicada::prelude::*;
use cicada::runtime::RetryResult;
use cicada::store::{DeliverNowResult, Message, OutcomeStatus};
use cicada::time::{self, MAX_DELAY, Timestamp};
use cicada::turn::TurnError;
use cicada_actors::{
    CreateKind, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, Flow, FlowActor, FlowMsg, FlowReply,
    FlowRun, FlowState, FlowStatus, JournalEntry, ReplyTo, Request, Step, Target, WaitMode, WaitOutcome,
};
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
const MINUTE: Duration = Duration::from_secs(60);
const MICROSECOND: Duration = Duration::from_micros(1);

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
    /// The values the flow code got from its waits, by wait id, in order.
    got: Arc<Mutex<Vec<(String, Value)>>>,
    /// Whether the flow's code is fixed: a typed wait then decodes any JSON.
    fixed: Arc<AtomicBool>,
}

impl Probe {
    /// What the flow code got from its waits.
    fn got(&self) -> Vec<(String, Value)> {
        self.got.lock().unwrap().clone()
    }

    /// Notes what a wait returned, once: every replay returns the same.
    fn note(&self, wait: &str, value: Value) {
        let mut got = self.got.lock().unwrap();
        let entry = (wait.to_owned(), value);
        if !got.contains(&entry) {
            got.push(entry);
        }
    }

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

    /// Whether the flow code planned to glitch at `id` does so this time: it fails outside any step.
    fn glitch(&self, id: &str) -> bool {
        self.plans.lock().unwrap().get_mut(id).and_then(VecDeque::pop_front).is_some()
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
    /// A durable sleep of `secs` seconds and `nanos` nanoseconds.
    Sleep {
        id: String,
        secs: u64,
        #[serde(default)]
        nanos: u32,
    },
    /// A one-minute sleep whose suspension the flow catches.
    SwallowSleep { id: String },
    /// An error of the flow code itself, outside any step, when the probe plans one for `id`.
    Glitch { id: String },
    /// A send of `message` to `worker/<to>`, with a create of `create` and the initial state `state` first.
    Send {
        id: String,
        to: String,
        message: WorkerMsg,
        #[serde(default)]
        create: Option<CreateKind>,
        #[serde(default)]
        state: Option<Shape>,
    },
    /// A send to `ghost/_`, a type no node registers.
    SendGhost { id: String },
    /// A send to `worker/<to>` of a message type the worker does not accept.
    SendForeign { id: String, to: String },
    /// A request to `responder/<to>` that asks it to `answer`; created first, of `create`, when given.
    Request {
        id: String,
        to: String,
        answer: Answer,
        #[serde(default)]
        create: Option<CreateKind>,
    },
    /// The wait `id` for the request `request` of this execution; the probe notes what it returns.
    Wait { id: String, request: String },
    /// A request `id` to `responder/<to>` asking for a number, and its wait `wait`: typed `u32` until the probe's code
    /// is fixed, any JSON after.
    Typed { id: String, wait: String, to: String, answer: Answer },
}

/// How a responder answers a request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "how", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Answer {
    /// Replies `value` in the turn of the request, with the helper.
    Now { value: Value },
    /// Keeps the route; [`ResponderMsg::Answer`] replies later.
    Later,
    /// Sends `FlowMsg::Reply` itself, past the helper's checks, naming `request` in place of the route's when given;
    /// the value is `depth` arrays deep when given, made by the responder: what an input may not hold.
    Raw {
        value: Value,
        #[serde(default)]
        depth: Option<u32>,
        #[serde(default)]
        request: Option<MessageId>,
    },
    /// Hands the route to `responder/<to>`, a delegate, which replies `value`.
    Delegate { to: String, value: Value },
    /// Hands the route to a new child of the responder, which replies `value`.
    Child { value: Value },
}

/// `responder/<id>`: the target of the test flow's requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct Responder(String);

impl ActorRef for Responder {
    const TYPE: &'static str = "responder";
    type Message = ResponderMsg;
}

fn responder(id: &str) -> Responder {
    Responder(id.to_owned())
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ResponderMsg {
    /// A request of a flow for any JSON.
    Ask { reply: ReplyTo<Value>, answer: Answer },
    /// A request of a flow for a number.
    AskNumber { reply: ReplyTo<u32>, answer: Answer },
    /// Replies `value` to every kept route, with the helper.
    Answer { value: Value },
    /// Replies `value` to `reply`, with the helper.
    Forward { reply: ReplyTo<Value>, value: Value },
    /// Sends `FlowMsg::Reply` with `request` and a value `depth` arrays deep to `run` itself, for a run created with
    /// its progress.
    Raw { run: ActorAddress, request: MessageId, depth: u32 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
struct ResponderState {
    /// The routes kept for a later answer.
    #[serde(default)]
    routes: Vec<ReplyTo<Value>>,
    /// The requests it got.
    #[serde(default)]
    asked: u32,
}

struct ResponderActor;

impl Actor for ResponderActor {
    type Ref = Responder;
    type State = ResponderState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ResponderMsg) -> HandlerResult {
        match message {
            ResponderMsg::Ask { reply, answer } => answer_with(turn, reply, answer).await?,
            ResponderMsg::AskNumber { reply, answer: Answer::Now { value: Value::Number(n) } } => {
                turn.state_mut().asked += 1;
                let n = u32::try_from(n.as_u64().unwrap()).unwrap();
                reply_cell(turn, retype(&reply), Cell::new(n)).await?;
            }
            ResponderMsg::AskNumber { reply, answer } => answer_with(turn, retype(&reply), answer).await?,
            ResponderMsg::Answer { value } => {
                for route in turn.state().routes.clone() {
                    route.send(turn, &value).await?;
                }
            }
            ResponderMsg::Forward { reply, value } => {
                reply.send(turn, &value).await?;
            }
            ResponderMsg::Raw { run, request, depth } => raw_reply(turn, &run, request, nested(depth)).await?,
        }
        Ok(())
    }
}

/// The same route for another reply type: the wire form does not depend on it.
fn retype<T, U>(route: &ReplyTo<T>) -> ReplyTo<U> {
    serde_json::from_value(serde_json::to_value(route).unwrap()).unwrap()
}

/// Scenario: Owned encoded value. The helper's future outlives the route and the value, both dropped here, and is
/// `Send` for a reply type that is not `Sync`.
fn reply_cell<'t>(
    turn: &'t mut Turn<ResponderActor>,
    reply: ReplyTo<Cell<u32>>,
    value: Cell<u32>,
) -> impl Future<Output = Result<MessageId, TurnError>> + Send + 't {
    reply.send(turn, &value)
}

async fn answer_with(turn: &mut Turn<ResponderActor>, reply: ReplyTo<Value>, answer: Answer) -> HandlerResult {
    turn.state_mut().asked += 1;
    match answer {
        Answer::Now { value } => {
            reply.send(turn, &value).await?;
        }
        Answer::Later => turn.state_mut().routes.push(reply),
        Answer::Raw { value, depth, request } => {
            let value = depth.map_or(value, nested);
            raw_reply(turn, &reply.run, request.unwrap_or(reply.request.clone()), value).await?;
        }
        Answer::Delegate { to, value } => {
            turn.send(&responder(&to), ResponderMsg::Forward { reply, value })?;
        }
        Answer::Child { value } => {
            let child = responder(&format!("{}-child", turn.self_ref().0));
            turn.create_child(&child, &ResponderState::default())?;
            turn.send(&child, ResponderMsg::Forward { reply, value })?;
        }
    }
    Ok(())
}

/// Sends a `Reply` past the helper, with the turn's correlation id.
async fn raw_reply(
    turn: &mut Turn<ResponderActor>,
    run: &ActorAddress,
    request: MessageId,
    value: Value,
) -> HandlerResult {
    let encoded = cicada::message::encode(&FlowMsg::Reply { request, value }).unwrap();
    let mut options = SendOptions::new();
    options.correlation_id = turn.message().correlation_id.clone();
    turn.send_raw(run, &encoded.message_type, encoded.payload, options).await?;
    Ok(())
}

/// The initial state of a send's create, made when the flow runs: what a state may not be cannot be in the input.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "shape", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Shape {
    Value {
        value: Value,
    },
    Null,
    /// A worker state with a NUL in its log.
    Nul,
    /// A worker state `depth` levels deep.
    Deep {
        depth: u32,
    },
}

impl Shape {
    fn make(self) -> Value {
        match self {
            Shape::Value { value } => value,
            Shape::Null => Value::Null,
            Shape::Nul => json!({"n": 0, "log": ["a\0b"]}),
            Shape::Deep { depth } => json!({"n": 0, "extra": nested(depth - 1)}),
        }
    }
}

/// `ghost/_`: a ref of a type that no node registers.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ghost;

impl ActorRef for Ghost {
    const TYPE: &'static str = "ghost";
    type Message = WorkerMsg;
}

/// `worker/{id}` with a family the worker does not accept.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct Foreign(String);

impl ActorRef for Foreign {
    const TYPE: &'static str = "worker";
    type Message = ForeignMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ForeignMsg {
    Unknown,
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
        let mut requests = HashMap::new();
        for op in script.ops {
            self.op(step, op, &mut requests).await?;
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
    async fn op(&self, step: &mut Step, op: Op, requests: &mut HashMap<String, Request<Value>>) -> HandlerResult {
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
            Op::Sleep { id, secs, nanos } => step.sleep(&id, Duration::new(secs, nanos)).await?,
            Op::SwallowSleep { id } => {
                let _ = step.sleep(&id, MINUTE).await;
            }
            Op::Glitch { id } => {
                if p.glitch(&id) {
                    return Err(HandlerError::msg("the flow glitched"));
                }
            }
            Op::Send { id, to, message, create, state } => {
                let target = worker(&to);
                let state = state.map(Shape::make);
                match (create, &state) {
                    (None, _) => step.send(&id, &target, message).await?,
                    (Some(CreateKind::Independent), Some(s)) => {
                        step.send(&id, Target::create(&target, s), message).await?
                    }
                    (Some(CreateKind::Child), Some(s)) => step.send(&id, Target::child(&target, s), message).await?,
                    (Some(_), None) => unreachable!("a create has a state"),
                }
            }
            Op::SendGhost { id } => step.send(&id, &Ghost, WorkerMsg::Work).await?,
            Op::SendForeign { id, to } => step.send(&id, Target::existing(&Foreign(to)), ForeignMsg::Unknown).await?,
            Op::Request { id, to, answer, create } => {
                let target = responder(&to);
                let initial = ResponderState::default();
                let build = |reply| ResponderMsg::Ask { reply, answer };
                let request = match create {
                    None => step.request(&id, &target, build).await?,
                    Some(CreateKind::Independent) => {
                        step.request(&id, Target::create(&target, &initial), build).await?
                    }
                    Some(CreateKind::Child) => step.request(&id, Target::child(&target, &initial), build).await?,
                };
                requests.insert(id, request);
            }
            Op::Wait { id, request } => {
                let value = step.wait(&id, &requests[&request]).await?;
                p.note(&id, value);
            }
            Op::Typed { id, wait, to, answer } => {
                let target = responder(&to);
                if p.fixed.load(Ordering::SeqCst) {
                    let request = step.request(&id, &target, |reply| ResponderMsg::Ask { reply, answer }).await?;
                    let value: Value = step.wait(&wait, &request).await?;
                    p.note(&wait, value);
                } else {
                    let request = step.request(&id, &target, |reply| ResponderMsg::AskNumber { reply, answer }).await?;
                    let value: u32 = step.wait(&wait, &request).await?;
                    p.note(&wait, json!(value));
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
                .host(WorkerActor)
                .host(ResponderActor)
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

/// Scenario: Lost progress before commit; and Key stability and escaping. The probe notes its call, an external effect,
/// before it would respond; the turn's timeout drops the call before the response, so the effect is lost to the run.
#[tokio::test(start_paused = true)]
async fn a_step_effect_lost_to_a_turn_timeout_is_called_again_with_the_same_key() {
    let probe = Probe::default();
    probe.plan("a", &[Act::Slow]);
    let test = node(&probe).await;
    start(&test, "k/1", script(steps(&["a"]))).await;
    test.run_until_idle().await;
    assert_eq!(probe.keys(), ["test.flow/k%2F1/a"], "the call was made, then the turn's maximum dropped it unanswered");
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

// Durable sleep.

fn sleep(id: &str, duration: Duration) -> Op {
    Op::Sleep { id: id.to_owned(), secs: duration.as_secs(), nanos: duration.subsec_nanos() }
}

fn saved_sleep(id: &str, until: Timestamp) -> JournalEntry {
    JournalEntry::Sleep { id: id.to_owned(), until }
}

fn after(instant: Timestamp, duration: Duration) -> Timestamp {
    time::add(instant, duration).unwrap()
}

/// Asserts that `key` holds `journal` and one pending continuation, armed and due at `until`, and returns it.
async fn sleeping(test: &TestActors, key: &str, journal: &[JournalEntry], until: Timestamp) -> Message {
    assert_eq!(state(test, key).await.journal, journal, "{key}");
    let timer = armed(test, key).await;
    assert_eq!(timer.deliver_at, until, "{key}: due at the saved wake time");
    assert_eq!(test.pending(&run(key)).await.len(), 1, "{key}: one continuation");
    timer
}

/// Scenarios: First durable wait; and Before and at the deadline. Every later turn replays the elapsed sleep.
#[tokio::test(start_paused = true)]
async fn a_sleep_commits_its_wake_time_with_a_continuation_due_then() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(vec![op(|id| Op::Step { id }, "a"), sleep("w", MINUTE), op(|id| Op::Step { id }, "b")]))
        .await;
    let t0 = test.now();
    assert_eq!(test.run_until_idle().await, 3, "the start, step a, and the sleep");
    let until = after(t0, MINUTE);
    let journal = [saved("a", json!("r-a")), saved_sleep("w", until)];
    let timer = sleeping(&test, "k", &journal, until).await;
    assert_eq!((timer.retry, timer.from), (Some(three_attempts()), Some(addr(&run("k")))));
    assert_eq!(probe.ids(), ["a"], "no closure after the sleep in its turn");

    // Tokio's virtual timer counts whole milliseconds.
    let ms = Duration::from_millis(1);
    assert_eq!(test.advance_time_by(MINUTE - ms).await, 0, "no handler runs before the wake time");
    assert_eq!(test.advance_time_by(ms).await, 2, "step b at the wake time, then the completion");
    assert_eq!(probe.ids(), ["a", "b"]);
    let done = state(&test, "k").await;
    assert_eq!(done.status, FlowStatus::Done);
    assert_eq!(done.journal, [journal[0].clone(), journal[1].clone(), saved("b", json!("r-b"))]);
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Done);
}

/// Scenarios: Zero sleep; and Fractional microsecond.
#[tokio::test(start_paused = true)]
async fn a_zero_or_sub_microsecond_sleep_still_ends_its_turn_with_a_commit() {
    for (duration, wait) in [(Duration::ZERO, Duration::ZERO), (Duration::from_nanos(1), MICROSECOND)] {
        let probe = Probe::default();
        let test = node(&probe).await;
        start(&test, "k", script(vec![sleep("w", duration), op(|id| Op::Step { id }, "b")])).await;
        test.system().run_due_turns(1).await.unwrap();
        let (t0, version) = (test.now(), test.version(&run("k")).await);
        test.system().run_due_turns(1).await.unwrap();
        assert_eq!(test.version(&run("k")).await, version + 1, "{duration:?}: the sleep commits");
        sleeping(&test, "k", &[saved_sleep("w", after(t0, wait))], after(t0, wait)).await;
        assert!(probe.ids().is_empty(), "{duration:?}: b waits for the next turn");

        test.advance_time_by(wait).await;
        assert_eq!(probe.ids(), ["b"], "{duration:?}");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Done, "{duration:?}");
    }
}

/// Scenario: Duration changes in code, also to one that a new sleep may not have.
#[tokio::test(start_paused = true)]
async fn a_replayed_sleep_keeps_its_saved_wake_time_when_the_code_changes_its_duration() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let until = after(test.now(), SLOW);
    let too_long = MAX_DELAY + SECOND;
    for (key, duration) in [("shorter", Duration::ZERO), ("longer", SLOW * 24), ("invalid", too_long)] {
        let code = script(vec![sleep("w", duration), op(|id| Op::Step { id }, "b")]);
        start_with(&test, key, restored(code, vec![saved_sleep("w", until)])).await;
    }
    test.run_until_idle().await;
    for key in ["shorter", "longer", "invalid"] {
        sleeping(&test, key, &[saved_sleep("w", until)], until).await;
    }
    assert!(probe.ids().is_empty());
    test.advance_time_by(SLOW).await;
    for key in ["shorter", "longer", "invalid"] {
        assert_eq!(state(&test, key).await.status, FlowStatus::Done, "{key}");
    }
    assert_eq!(probe.ids(), ["b", "b", "b"]);
}

/// Scenario: Manual early Continue.
#[tokio::test(start_paused = true)]
async fn an_early_public_continue_rearms_the_sleep_at_its_saved_wake_time() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(vec![sleep("w", MINUTE), op(|id| Op::Step { id }, "b")])).await;
    test.run_until_idle().await;
    let timer = armed(&test, "k").await;
    let until = timer.deliver_at;
    test.advance_time_by(SECOND * 10).await;

    test.send(&run("k"), FlowMsg::Continue).await;
    test.system().run_due_turns(1).await.unwrap();
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled, "the timer is replaced");
    assert_eq!(armed(&test, "k").await.deliver_at, test.now(), "the normalization arms a continuation due now");
    assert_eq!(test.run_until_idle().await, 1, "it replays the sleep");
    let rearmed = sleeping(&test, "k", &[saved_sleep("w", until)], until).await;
    assert!(rearmed.id != timer.id);
    assert!(probe.ids().is_empty(), "nothing after the sleep runs early");

    test.advance_time_by(MINUTE).await;
    assert_eq!(probe.ids(), ["b"]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// Scenario: Deliver-now.
#[tokio::test(start_paused = true)]
async fn a_sleep_timer_delivered_now_rearms_at_the_same_time() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(vec![sleep("w", MINUTE), op(|id| Op::Step { id }, "b")])).await;
    test.run_until_idle().await;
    let timer = armed(&test, "k").await;
    let until = timer.deliver_at;
    let DeliverNowResult::Delivered(_) = test.system().deliver_now(&timer.id).await.unwrap() else {
        panic!("delivered now")
    };
    assert_eq!(test.run_until_idle().await, 1);
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Done, "not cancelled by itself");
    let rearmed = sleeping(&test, "k", &[saved_sleep("w", until)], until).await;
    assert!(rearmed.id != timer.id);
    assert!(probe.ids().is_empty());
    test.advance_time_by(MINUTE).await;
    assert_eq!(probe.ids(), ["b"]);
}

/// Scenario: Early redrive, of a cancelled timer under a new id, and a retry of the armed continuation.
#[tokio::test(start_paused = true)]
async fn a_redrive_or_retry_before_the_wake_time_rearms_at_the_same_time() {
    let probe = Probe::default();
    probe.plan("g", &[Act::Fail]);
    let test = node(&probe).await;
    start(&test, "redrive", script(vec![sleep("w", MINUTE), op(|id| Op::Step { id }, "b")])).await;
    let until = after(test.now(), MINUTE);
    let code = script(vec![op(|id| Op::Glitch { id }, "g"), sleep("w", MINUTE), op(|id| Op::Step { id }, "b")]);
    start_with(&test, "retry", restored(code, vec![saved_sleep("w", until)])).await;
    test.run_until_idle().await;

    let timer = armed(&test, "redrive").await;
    test.system().cancel(&timer.id).await.unwrap();
    let RetryResult::Accepted(redriven) = test.system().retry_message(&timer.id).await.unwrap() else {
        panic!("redriven")
    };
    assert_eq!(test.run_until_idle().await, 2, "the redrive normalizes, its continuation replays");
    assert_eq!(outcome(&test, &redriven.id).await.unwrap().status, OutcomeStatus::Done);
    sleeping(&test, "redrive", &[saved_sleep("w", until)], until).await;

    let failing = armed(&test, "retry").await;
    assert_eq!((failing.failures, failing.last_error.as_deref()), (1, Some("the flow glitched")));
    test.advance_time_by(BACKOFF).await;
    let record = outcome(&test, &failing.id).await.unwrap();
    assert_eq!((record.status, record.envelope.attempt), (OutcomeStatus::Done, 2), "retried before the wake time");
    sleeping(&test, "retry", &[saved_sleep("w", until)], until).await;
    assert!(probe.ids().is_empty());

    test.advance_time_by(MINUTE).await;
    assert_eq!(probe.ids(), ["b", "b"]);
}

/// Scenario: Recovery after a cancelled (or purged) timer.
#[tokio::test(start_paused = true)]
async fn a_lost_sleep_timer_stalls_the_run_until_a_public_continue() {
    for how in ["cancel", "purge"] {
        let probe = Probe::default();
        let test = node(&probe).await;
        start(&test, "k", script(vec![sleep("w", MINUTE), op(|id| Op::Step { id }, "b")])).await;
        test.run_until_idle().await;
        let timer = armed(&test, "k").await;
        match how {
            "cancel" => drop(test.system().cancel(&timer.id).await.unwrap()),
            _ => drop(test.system().purge_mailbox(&addr(&run("k"))).await.unwrap()),
        }
        assert_eq!(test.advance_time_by(MINUTE * 2).await, 0, "{how}: stalled past the wake time");
        let held = state(&test, "k").await;
        assert_eq!((held.journal, held.armed), (vec![saved_sleep("w", timer.deliver_at)], Some(timer.id)), "{how}");
        assert!(probe.ids().is_empty(), "{how}");

        test.send(&run("k"), FlowMsg::Continue).await;
        test.run_until_idle().await;
        assert_eq!(probe.ids(), ["b"], "{how}: the elapsed sleep is replayed, not slept again");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Done, "{how}");
    }
}

/// Scenario: Pause past wake time.
#[tokio::test(start_paused = true)]
async fn a_run_paused_past_its_wake_time_continues_when_resumed_without_sleeping_again() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(vec![sleep("w", MINUTE), op(|id| Op::Step { id }, "b")])).await;
    test.run_until_idle().await;
    let until = armed(&test, "k").await.deliver_at;
    assert!(test.system().pause_mailbox(&addr(&run("k"))).await.unwrap());
    assert_eq!(test.advance_time_by(MINUTE * 2).await, 0);
    assert!(probe.ids().is_empty());
    assert!(test.system().resume_mailbox(&addr(&run("k"))).await.unwrap());
    assert_eq!(test.run_until_idle().await, 2, "step b, then the completion, at once");
    let done = state(&test, "k").await;
    assert_eq!(done.journal, [saved_sleep("w", until), saved("b", json!("r-b"))]);
    assert_eq!(done.status, FlowStatus::Done);
}

/// Scenario: Deleted run.
#[tokio::test(start_paused = true)]
async fn a_deleted_sleeping_run_loses_its_timer_and_is_not_recreated() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(vec![sleep("w", MINUTE), op(|id| Op::Step { id }, "b")])).await;
    test.run_until_idle().await;
    test.system().delete_actor(&addr(&run("k"))).await.unwrap();
    assert!(test.pending(&run("k")).await.is_empty(), "the timer is purged");
    assert_eq!(test.advance_time_by(MINUTE * 2).await, 0);
    assert!(!test.exists(&run("k")).await);
    assert!(probe.ids().is_empty());
}

/// Scenario: Retry after sleep.
#[tokio::test(start_paused = true)]
async fn a_failing_step_after_a_sleep_retries_without_sleeping_again() {
    let probe = Probe::default();
    probe.plan("b", &[Act::Fail]);
    let test = node(&probe).await;
    start(&test, "k", script(vec![op(|id| Op::Step { id }, "a"), sleep("w", MINUTE), op(|id| Op::Step { id }, "b")]))
        .await;
    test.run_until_idle().await;
    let until = armed(&test, "k").await.deliver_at;
    test.advance_time_by(MINUTE).await;
    let failed = armed(&test, "k").await;
    assert_eq!((failed.failures, failed.deliver_at), (1, after(until, BACKOFF)), "the flow's backoff, no new sleep");
    assert_eq!(test.advance_time_by(BACKOFF).await, 2, "the retry and the completion");
    assert_eq!(probe.ids(), ["a", "b", "b"], "a is replayed, not run");
    let done = state(&test, "k").await;
    assert_eq!(done.journal, [saved("a", json!("r-a")), saved_sleep("w", until), saved("b", json!("r-b"))]);
}

/// Scenario: Sleep after an abandoned step.
#[tokio::test(start_paused = true)]
async fn a_sleep_after_an_abandoned_step_adds_nothing() {
    let probe = Probe::default();
    probe.plan("a", &[Act::Slow]);
    let test = node(&probe).await;
    start(&test, "k", script(vec![op(|id| Op::Abandon { id }, "a"), sleep("w", MINUTE)])).await;
    test.run_until_idle().await;
    refused(&test, "k", "step `a` was abandoned").await;
    assert!(journal(&test, "k").await.is_empty());
}

/// Scenario: Swallowed sleep suspension, of a new sleep and of a replayed one not yet due.
#[tokio::test(start_paused = true)]
async fn a_swallowed_sleep_suspension_still_controls_the_turn() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let code = |end| ending(vec![op(|id| Op::SwallowSleep { id }, "w"), op(|id| Op::Step { id }, "b")], end);
    let t0 = test.now();
    start(&test, "ok", code(End::Ok)).await;
    start(&test, "fail", code(End::Fail)).await;
    let until = after(t0, SLOW);
    start_with(&test, "replay", restored(code(End::Ok), vec![saved_sleep("w", until)])).await;
    test.run_until_idle().await;
    for key in ["ok", "fail"] {
        sleeping(&test, key, &[saved_sleep("w", after(t0, MINUTE))], after(t0, MINUTE)).await;
    }
    sleeping(&test, "replay", &[saved_sleep("w", until)], until).await;
    assert!(probe.ids().is_empty(), "b is not invoked after a caught suspension");
}

/// Scenarios: Sleep bounds; Shared id namespace and count; and Wrong operation kind.
#[tokio::test(start_paused = true)]
async fn an_invalid_new_sleep_fails_for_good_and_keeps_earlier_progress() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let x = || op(|id| Op::Step { id }, "x");
    let full: Vec<JournalEntry> = (0..1000).map(|i| saved(&format!("s{i}"), json!(format!("r-s{i}")))).collect();
    let cases = [
        (
            "too-long",
            vec![x(), sleep("w", MAX_DELAY + Duration::from_nanos(1))],
            vec![saved("x", json!("r-x"))],
            "longer than the most a sleep may last",
        ),
        ("repeated", vec![x(), sleep("x", MINUTE)], vec![saved("x", json!("r-x"))], "calls the id `x` twice"),
        (
            "kind",
            vec![x(), sleep("w", MINUTE)],
            vec![saved("x", json!("r-x")), saved("w", json!("r-w"))],
            "calls sleep `w` where its journal holds step `w`",
        ),
        (
            "full",
            vec![Op::Repeat { prefix: "s".into(), count: 1000 }, sleep("w", MINUTE)],
            full,
            "sleep `w` would be entry 1001 of a journal that holds at most 1000",
        ),
    ];
    for (key, ops, saved, why) in cases {
        let before = saved.clone();
        start_with(&test, key, restored(script(ops), saved)).await;
        test.run_until_idle().await;
        refused(&test, key, why).await;
        assert_eq!(state(&test, key).await.journal, before, "{key}: no new progress");
    }
    assert!(probe.keys().is_empty());
}

// Sends (ADR 0044).

/// A send of `message` to `worker/<to>`, with no create.
fn send_op(id: &str, to: &str, message: WorkerMsg) -> Op {
    Op::Send { id: id.into(), to: to.into(), message, create: None, state: None }
}

/// A send of `message` to `worker/<to>` that creates it first, of `kind`, with the initial state `shape`.
fn create_op(id: &str, to: &str, kind: CreateKind, shape: Shape, message: WorkerMsg) -> Op {
    Op::Send { id: id.into(), to: to.into(), message, create: Some(kind), state: Some(shape) }
}

fn initial(n: u32) -> Shape {
    Shape::Value { value: json!({"n": n}) }
}

async fn worker_state(test: &TestActors, id: &str) -> WorkerState {
    test.state(&worker(id)).await
}

/// The send entry `id` of the journal of `key`.
async fn send_entry(test: &TestActors, key: &str, id: &str) -> (ActorAddress, MessageId, Option<CreateKind>) {
    let journal = state(test, key).await.journal;
    match journal.iter().find(|e| e.id() == id) {
        Some(JournalEntry::Send { to, message, create, reply: false, .. }) => (to.clone(), message.clone(), *create),
        other => panic!("{key}: no send `{id}`: {other:?}"),
    }
}

/// Asserts that the armed `Continue` of `key` ended `failed` with the cause `commit` and that nothing is pending.
async fn rejected(test: &TestActors, key: &str) {
    let id = state(test, key).await.armed.expect("the failed continuation stays armed");
    let record = outcome(test, &id).await.expect("an outcome");
    assert_eq!(record.status, OutcomeStatus::Failed, "{key}");
    let told = told(test).await;
    assert!(told.iter().any(|f| f.message_id == id && f.cause == FailureCause::Commit), "{key}: {told:?}");
    assert!(test.pending(&run(key)).await.is_empty(), "{key}: no continuation");
}

/// Scenario: Create and send; and Metadata and target failure.
#[tokio::test(start_paused = true)]
async fn a_send_commits_its_create_message_entry_and_continuation_together() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let ops =
        vec![create_op("start", "w1", CreateKind::Child, initial(5), WorkerMsg::Work), op(|id| Op::Step { id }, "a")];
    test.create(&run("k"), &FlowState::new(script(ops))).await;
    test.send_with(&run("k"), FlowMsg::Continue, SendOptions::new().correlation_id("c1")).await;
    test.system().run_due_turns(1).await.unwrap();
    test.system().run_due_turns(1).await.unwrap();
    assert_eq!(test.version(&run("k")).await, 2, "the poke, then one commit of the send");
    let (to, message, create) = send_entry(&test, "k", "start").await;
    assert_eq!((to, create), (addr(&worker("w1")), Some(CreateKind::Child)));
    assert_eq!(worker_state(&test, "w1").await.n, 5, "created in the same commit");
    let record = test.system().get_actor(&addr(&worker("w1"))).await.unwrap().unwrap();
    assert_eq!(record.parent, Some(addr(&run("k"))), "a child of the run");
    let sent = test.pending(&worker("w1")).await;
    assert_eq!(sent.len(), 1);
    let work = &sent[0];
    assert_eq!((&work.id, work.message_type.as_str()), (&message, "Work"), "the entry's message id");
    assert_eq!(work.from, Some(addr(&run("k"))));
    assert_eq!(work.correlation_id.as_deref(), Some("c1"), "the executing Continue's correlation");
    assert_eq!((work.retry.clone(), work.retention.clone()), (None, None), "no retry policy, default retention");
    assert!(work.deliver_at <= test.now(), "due now");
    let next = armed(&test, "k").await;
    assert_eq!((next.retry, next.correlation_id.as_deref()), (Some(three_attempts()), Some("c1")));

    test.run_until_idle().await;
    assert_eq!(worker_state(&test, "w1").await.n, 6);
    let done = state(&test, "k").await;
    assert_eq!((done.status, done.journal.len()), (FlowStatus::Done, 2));
    assert_eq!(probe.ids(), ["a"]);
}

/// Scenario: Later failure; and the target's failure ends its message without undoing progress.
#[tokio::test(start_paused = true)]
async fn a_saved_send_replays_without_another_create_or_message() {
    let probe = Probe::default();
    probe.plan("after", &[Act::Fail]);
    let test = node(&probe).await;
    test.create(&worker("w1"), &WorkerState::default()).await;
    let ops = vec![
        send_op("go", "w1", WorkerMsg::Work),
        send_op("boom", "w1", WorkerMsg::Fail),
        op(|id| Op::Step { id }, "after"),
    ];
    start(&test, "k", script(ops)).await;
    test.run_until_idle().await;
    assert_eq!(journal(&test, "k").await, ["go", "boom"]);
    let (_, failing, _) = send_entry(&test, "k", "boom").await;
    let record = outcome(&test, &failing).await.unwrap();
    assert_eq!((record.status, record.envelope.attempt), (OutcomeStatus::Failed, 1), "no retry of its own");
    assert_eq!(state(&test, "k").await.status, FlowStatus::Running, "the target's failure undoes nothing");

    assert_eq!(test.advance_time_by(BACKOFF).await, 2, "the retry of `after` and the completion");
    assert_eq!(probe.ids(), ["after", "after"]);
    let done = state(&test, "k").await;
    assert_eq!(done.status, FlowStatus::Done);
    assert_eq!(worker_state(&test, "w1").await.n, 1, "one message for one entry, after two replays");
    assert!(test.pending(&worker("w1")).await.is_empty());
}

/// Scenario: Existing actor and child collision; and Missing destination and recovery.
#[tokio::test(start_paused = true)]
async fn creates_follow_core_rules_and_a_rejected_commit_stalls_until_a_public_continue() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&worker("kept"), &WorkerState { n: 5, ..WorkerState::default() }).await;
    start(&test, "reuse", script(vec![create_op("go", "kept", CreateKind::Independent, initial(0), WorkerMsg::Work)]))
        .await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "reuse").await.status, FlowStatus::Done);
    assert_eq!(worker_state(&test, "kept").await.n, 6, "the existing state, untouched, then the message");

    test.create(&worker("owned"), &WorkerState::default()).await;
    start(&test, "child", script(vec![create_op("go", "owned", CreateKind::Child, initial(0), WorkerMsg::Work)])).await;
    start(&test, "absent", script(vec![send_op("go", "later", WorkerMsg::Work)])).await;
    test.run_until_idle().await;
    for key in ["child", "absent"] {
        rejected(&test, key).await;
        assert!(journal(&test, key).await.is_empty(), "{key}: no entry");
    }
    assert_eq!(worker_state(&test, "owned").await.n, 0, "not taken over, no message");
    assert!(test.pending(&worker("owned")).await.is_empty());

    test.create(&worker("later"), &WorkerState::default()).await;
    test.system().delete_actor(&addr(&worker("owned"))).await.unwrap();
    test.run_until_idle().await;
    for key in ["child", "absent"] {
        test.send(&run(key), FlowMsg::Continue).await;
    }
    test.run_until_idle().await;
    for (key, target) in [("child", "owned"), ("absent", "later")] {
        assert_eq!(state(&test, key).await.status, FlowStatus::Done, "{key}: recovered");
        assert_eq!(worker_state(&test, target).await.n, 1, "{key}: sent afresh");
    }
    let record = test.system().get_actor(&addr(&worker("owned"))).await.unwrap().unwrap();
    assert_eq!(record.parent, Some(addr(&run("child"))));
}

/// Scenario: Registry failure and invalid target, the refusals; and Existing actor and child collision, the checks of
/// an initial state that apply when the target exists.
#[tokio::test(start_paused = true)]
async fn refused_targets_messages_and_initial_states_fail_for_good_without_effects() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&worker("w1"), &WorkerState::default()).await;
    let x = || op(|id| Op::Step { id }, "x");
    let invalid = |shape: Shape| create_op("go", "w1", CreateKind::Independent, shape, WorkerMsg::Work);
    let cases = [
        ("ghost", Op::SendGhost { id: "go".into() }, "send `go` to ghost/_ is refused"),
        ("foreign", Op::SendForeign { id: "go".into(), to: "w1".into() }, "is refused"),
        ("null", invalid(Shape::Null), "null"),
        ("nul", invalid(Shape::Nul), "NUL"),
        ("deep", invalid(Shape::Deep { depth: 65 }), "nested deeper than 64"),
        ("child-null", create_op("go", "w1", CreateKind::Child, Shape::Null, WorkerMsg::Work), "null"),
    ];
    for (key, send, why) in cases {
        let before = vec![saved("x", json!("r-x"))];
        start_with(&test, key, restored(script(vec![x(), send]), before.clone())).await;
        test.run_until_idle().await;
        refused(&test, key, why).await;
        refused(&test, key, "send `go`").await;
        assert_eq!(state(&test, key).await.journal, before, "{key}: no new progress");
    }
    assert_eq!(worker_state(&test, "w1").await, WorkerState::default(), "untouched");
    assert!(test.pending(&worker("w1")).await.is_empty(), "nothing sent");

    start(&test, "deepest", script(vec![invalid(Shape::Deep { depth: 64 })])).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "deepest").await.status, FlowStatus::Done, "64 levels is the core's limit");
}

/// Observer headers go on a flow's sends as on any send.
#[tokio::test(start_paused = true)]
async fn observer_headers_go_on_a_send() {
    struct Picks;
    impl cicada::observer::TurnObserver for Picks {
        fn turn_started(&self, turn: &mut cicada::observer::TurnStart<'_>) -> Box<dyn cicada::observer::ObservedTurn> {
            turn.add_header_for(cicada::observer::Sends::Others, "work", "w");
            Box::new(())
        }
    }
    let probe = Probe::default();
    let flow = ScriptFlow { probe: probe.clone() };
    let test = TestActors::builder()
        .with(move |node| node.host(FlowActor::new(flow)).host(WorkerActor).observe(Picks))
        .build()
        .await
        .unwrap();
    test.create(&worker("w1"), &WorkerState::default()).await;
    start(&test, "k", script(vec![send_op("go", "w1", WorkerMsg::Work)])).await;
    test.system().run_due_turns(1).await.unwrap();
    test.system().run_due_turns(1).await.unwrap();
    let sent = test.pending(&worker("w1")).await;
    assert_eq!(sent[0].headers, [("work".to_owned(), "w".to_owned())].into());
}

/// Scenario: Reserved message and phase.
#[tokio::test(start_paused = true)]
async fn cancel_and_cancelling_are_refused_and_terminal_runs_ignore_continue_and_reply() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&run("cancel"), &FlowState::new(script(steps(&["a"])))).await;
    let sent = test.send(&run("cancel"), FlowMsg::Cancel).await;
    test.run_until_idle().await;
    let record = outcome(&test, &sent.id).await.unwrap();
    assert_eq!((record.status, record.envelope.failures), (OutcomeStatus::Failed, 0));
    assert!(record.error.unwrap().contains("refuses FlowMsg::Cancel for good"));
    assert_eq!(test.version(&run("cancel")).await, 0, "no effects");
    assert!(test.pending(&run("cancel")).await.is_empty());

    let reply = || FlowMsg::Reply { request: MessageId::new("m1").unwrap(), value: Value::Null };
    for (key, message) in [("cancelling", FlowMsg::Continue), ("cancelling-reply", reply())] {
        let cancelling = FlowState { status: FlowStatus::Cancelling, ..FlowState::new(script(steps(&["a"]))) };
        test.create(&run(key), &cancelling).await;
        let sent = test.send(&run(key), message).await;
        test.run_until_idle().await;
        let record = outcome(&test, &sent.id).await.unwrap();
        assert_eq!(record.status, OutcomeStatus::Failed, "{key}");
        assert!(record.error.unwrap().contains("refuses the cleanup of a cancelling run for good"), "{key}");
        assert_eq!(test.version(&run(key)).await, 0, "{key}");
    }

    for status in [FlowStatus::Done, FlowStatus::Cancelled] {
        for message in [FlowMsg::Continue, reply()] {
            let key = format!("{status:?}-{message:?}");
            let terminal = FlowState { status, ..restored(script(steps(&["a"])), vec![saved("a", json!("r-a"))]) };
            test.create(&run(&key), &terminal).await;
            let sent = test.send(&run(&key), message).await;
            test.run_until_idle().await;
            assert_eq!(outcome(&test, &sent.id).await.unwrap().status, OutcomeStatus::Done, "{key}");
            let kept = test.state_json(&run(&key)).await.unwrap();
            assert_eq!(kept, serde_json::to_value(&terminal).unwrap(), "{key}: the state stays");
            assert_eq!(test.version(&run(&key)).await, 0, "{key}");
        }
    }
    assert!(probe.keys().is_empty(), "no flow code ran");
}

/// Scenario: Replay misuse, through the runner: a saved request does not replay as a send, a saved join cannot replay
/// in this release, and a saved wait with a deadline does not replay through `Step::wait`.
#[tokio::test(start_paused = true)]
async fn a_saved_request_or_a_reserved_wait_stalls_replay_for_good() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let request = saved_request("go", "w1", "m1");
    let ops = vec![send_op("go", "w1", WorkerMsg::Work)];
    start_with(&test, "request", restored(script(ops), vec![request.clone()])).await;
    test.run_until_idle().await;
    refused(&test, "request", "calls send `go` where its journal holds request `go` at entry 1").await;
    refused(&test, "request", "changed flow code cannot replay this run").await;

    let second = saved_request("q", "w1", "m2");
    let join = open_wait("x", &["m1", "m2"], WaitMode::All, None);
    let first = open_wait("x", &["m1"], WaitMode::First, None);
    let deadline = open_wait("x", &["m1"], WaitMode::All, Some("2026-01-01T00:00:00Z"));
    for (key, wait) in [("join", join), ("first", first), ("deadline", deadline)] {
        let ops = vec![ask("go", "w1", Answer::Later), ask("q", "w1", Answer::Later), wait_op("x", "go")];
        let saved = vec![request.clone(), second.clone(), wait];
        start_with(&test, key, restored(script(ops), saved.clone())).await;
        test.run_until_idle().await;
        let why = if key == "deadline" { "wait `x` is saved with a deadline, which Step::wait" } else { "a join" };
        refused(&test, key, why).await;
        assert_eq!(state(&test, key).await.journal, saved, "{key}");
    }
    let after = vec![op(|id| Op::Step { id }, "a")];
    let join = open_wait("x", &["m1", "m2"], WaitMode::All, None);
    let saved = vec![saved("a", json!("r-a")), request, second, join];
    start_with(&test, "early", restored(script(after), saved)).await;
    test.run_until_idle().await;
    refused(&test, "early", "returned before reaching its saved request `go`: changed flow code").await;
    assert!(probe.got().is_empty());
}

// Requests, replies and waits (flow-reply-waits).

/// A request to `responder/<to>` that asks it to `answer`.
fn ask(id: &str, to: &str, answer: Answer) -> Op {
    Op::Request { id: id.into(), to: to.into(), answer, create: None }
}

fn wait_op(id: &str, request: &str) -> Op {
    Op::Wait { id: id.into(), request: request.into() }
}

fn now(value: Value) -> Answer {
    Answer::Now { value }
}

/// A saved request entry to `responder/<to>` with the message id `message`.
fn saved_request(id: &str, to: &str, message: &str) -> JournalEntry {
    JournalEntry::Send {
        id: id.into(),
        to: addr(&responder(to)),
        message: MessageId::new(message).unwrap(),
        create: None,
        reply: true,
    }
}

fn open_wait(id: &str, requests: &[&str], mode: WaitMode, until: Option<&str>) -> JournalEntry {
    JournalEntry::Wait {
        id: id.into(),
        requests: requests.iter().map(|r| MessageId::new(*r).unwrap()).collect(),
        mode,
        until: until.map(|u| time::parse_instant(u).unwrap()),
        outcome: None,
        received: vec![],
    }
}

fn kept(request: &str, value: Value) -> FlowReply {
    FlowReply { request: MessageId::new(request).unwrap(), value }
}

/// The message id of the request `id` of the journal of `key`.
async fn request_id(test: &TestActors, key: &str, id: &str) -> MessageId {
    match state(test, key).await.journal.iter().find(|e| e.id() == id) {
        Some(JournalEntry::Send { message, reply: true, .. }) => message.clone(),
        other => panic!("{key}: no request `{id}`: {other:?}"),
    }
}

async fn responder_state(test: &TestActors, id: &str) -> ResponderState {
    test.state(&responder(id)).await
}

/// The wait entry `id` of the journal of `key`, as its outcome and its selection.
async fn wait_entry(test: &TestActors, key: &str, id: &str) -> (Option<WaitOutcome>, Vec<MessageId>) {
    match state(test, key).await.journal.iter().find(|e| e.id() == id) {
        Some(JournalEntry::Wait { outcome, received, .. }) => (*outcome, received.clone()),
        other => panic!("{key}: no wait `{id}`: {other:?}"),
    }
}

/// Starts `key` with `script`, runs until idle and asserts that it is parked at the open wait `wait`: no armed and no
/// pending continuation.
async fn parked(test: &TestActors, key: &str, script: Script, wait: &str) {
    start(test, key, script).await;
    test.run_until_idle().await;
    assert_parked(test, key, wait).await;
}

async fn assert_parked(test: &TestActors, key: &str, wait: &str) {
    let held = state(test, key).await;
    assert_eq!((held.status, held.armed.clone()), (FlowStatus::Running, None), "{key}: parked");
    assert_eq!(held.journal.last().map(JournalEntry::id), Some(wait), "{key}");
    assert_eq!(wait_entry(test, key, wait).await, (None, vec![]), "{key}: open");
    assert!(test.pending(&run(key)).await.is_empty(), "{key}: no continuation, no polling");
}

/// Scenario: Wait before reply; Reply closes an open wait; and Owned encoded value. A run parks at its open wait with
/// no continuation; the target's later reply commits the value, closes the wait and arms a `Continue` with the reply's
/// correlation id, without flow code; the next execution returns the value.
#[tokio::test(start_paused = true)]
async fn a_late_reply_closes_the_parked_wait_and_arms_with_its_correlation() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let ops = vec![
        Op::Request { id: "ask".into(), to: "r1".into(), answer: Answer::Later, create: Some(CreateKind::Child) },
        wait_op("done", "ask"),
        op(|id| Op::Step { id }, "after"),
    ];
    test.create(&run("k"), &FlowState::new(script(ops))).await;
    test.send_with(&run("k"), FlowMsg::Continue, SendOptions::new().correlation_id("c-start")).await;
    test.run_until_idle().await;
    assert_parked(&test, "k", "done").await;
    let asked = request_id(&test, "k", "ask").await;
    let routes = responder_state(&test, "r1").await.routes;
    assert_eq!(serde_json::to_value(&routes).unwrap(), json!([{"run": "test.flow/k", "request": asked.as_str()}]));
    let record = test.system().get_actor(&addr(&responder("r1"))).await.unwrap().unwrap();
    assert_eq!(record.parent, Some(addr(&run("k"))), "created as a child in the request's commit");
    let version = test.version(&run("k")).await;

    test.send_with(
        &responder("r1"),
        ResponderMsg::Answer { value: json!({"ok": 1}) },
        SendOptions::new().correlation_id("c-answer"),
    )
    .await;
    test.system().run_due_turns(1).await.unwrap();
    let replies = test.pending(&run("k")).await;
    assert_eq!(replies.len(), 1);
    let reply = &replies[0];
    assert_eq!(reply.message_type, "Reply");
    assert_eq!(reply.payload, json!({"request": asked.as_str(), "value": {"ok": 1}}));
    assert_eq!(reply.from, Some(addr(&responder("r1"))));
    assert_eq!(reply.correlation_id.as_deref(), Some("c-answer"), "the replier's turn's correlation");
    assert_eq!((reply.retry.clone(), reply.retention.clone()), (None, None), "no retry policy, default retention");
    assert!(reply.deliver_at <= test.now(), "due now");

    test.system().run_due_turns(1).await.unwrap();
    assert_eq!(test.version(&run("k")).await, version + 1, "one commit");
    let held = state(&test, "k").await;
    assert_eq!(held.replies, [kept(asked.as_str(), json!({"ok": 1}))]);
    assert_eq!(wait_entry(&test, "k", "done").await, (Some(WaitOutcome::Received), vec![asked.clone()]));
    let next = armed(&test, "k").await;
    assert_eq!((next.retry, next.correlation_id.as_deref()), (Some(three_attempts()), Some("c-answer")));
    assert!(probe.keys().is_empty() && probe.got().is_empty(), "the Reply turn runs no flow code");

    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), json!({"ok": 1}))]);
    assert_eq!(probe.ids(), ["after"]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
    assert_eq!(responder_state(&test, "r1").await.asked, 1, "asked once");
}

/// Scenario: Ignored reply; and Sender boundary. Every ignored reply ends `done` without a write, even when its value
/// would fail the depth check: unknown ids, a plain send's id, another sender (outside, delegate, child) and a
/// duplicate with another value.
#[tokio::test(start_paused = true)]
async fn ignored_replies_change_nothing() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&worker("w1"), &WorkerState::default()).await;
    test.create(&responder("r2"), &ResponderState::default()).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![send_op("go", "w1", WorkerMsg::Work), ask("ask", "r1", Answer::Later), wait_op("done", "ask")];
    parked(&test, "k", script(ops), "done").await;
    let asked = request_id(&test, "k", "ask").await;
    let (_, plain, _) = send_entry(&test, "k", "go").await;
    let deep = nested(63);

    let before = unchanged(&test, "k").await;
    let raw = |request: MessageId| Answer::Raw { value: Value::Null, depth: Some(63), request: Some(request) };
    let cases = [
        ("unknown id", raw(MessageId::random())),
        ("plain send", raw(plain.clone())),
        ("delegate", Answer::Delegate { to: "r2".into(), value: json!("delegated") }),
        ("child", Answer::Child { value: json!("child") }),
    ];
    for (case, answer) in cases {
        test.send(&responder("r1"), ResponderMsg::Ask { reply: route(&asked), answer }).await;
        test.run_until_idle().await;
        assert_eq!(unchanged(&test, "k").await, before, "{case}");
        assert!(test.failures().await.is_empty(), "{case}: {:?}", test.failures().await);
        assert!(test.pending(&run("k")).await.is_empty(), "{case}");
    }
    assert!(test.exists(&responder("r1-child")).await, "the child replied");
    let outside = test.send(&run("k"), FlowMsg::Reply { request: asked.clone(), value: deep.clone() }).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &outside.id).await.unwrap().status, OutcomeStatus::Done, "no sender: ignored");
    assert_eq!(unchanged(&test, "k").await, before);
    assert!(probe.got().is_empty() && probe.keys().is_empty(), "no flow code ran");

    // Duplicates, while the run sleeps before its wait: the first value stays.
    test.create(&responder("r3"), &ResponderState::default()).await;
    let ops = vec![ask("ask", "r3", Answer::Later), sleep("nap", MINUTE), wait_op("done", "ask")];
    start(&test, "dup", script(ops)).await;
    test.run_until_idle().await;
    test.send(&responder("r3"), ResponderMsg::Answer { value: json!("first") }).await;
    test.run_until_idle().await;
    let first = unchanged(&test, "dup").await;
    let asked = request_id(&test, "dup", "ask").await;
    assert_eq!(state(&test, "dup").await.replies, [kept(asked.as_str(), json!("first"))]);
    test.send(&responder("r3"), ResponderMsg::Answer { value: json!("second") }).await;
    let reply = serde_json::from_value(json!({"run": "test.flow/dup", "request": asked.as_str()})).unwrap();
    let answer = Answer::Raw { value: Value::Null, depth: Some(63), request: None };
    test.send(&responder("r3"), ResponderMsg::Ask { reply, answer }).await;
    test.run_until_idle().await;
    assert_eq!(unchanged(&test, "dup").await, first, "a duplicate replaces nothing");
    assert!(test.failures().await.is_empty(), "a deep duplicate is ignored before its value is checked");
    test.advance_time_by(MINUTE).await;
    assert_eq!(probe.got(), [("done".to_owned(), json!("first"))], "the first value wins");
}

/// The state of `key` as JSON, and its version.
async fn unchanged(test: &TestActors, key: &str) -> (Value, i64) {
    (test.state_json(&run(key)).await.unwrap(), test.version(&run(key)).await)
}

/// The route of the request `asked` of `test.flow/k`.
fn route<T>(asked: &MessageId) -> ReplyTo<T> {
    serde_json::from_value(json!({"run": "test.flow/k", "request": asked.as_str()})).unwrap()
}

/// Scenario: Ignored reply, of a request that a closed wait names; supplied progress with a timed-out wait.
#[tokio::test(start_paused = true)]
async fn a_reply_to_a_request_a_closed_wait_names_is_ignored() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let mut closed = open_wait("x", &["m1"], WaitMode::All, Some("2026-01-01T00:00:00Z"));
    if let JournalEntry::Wait { outcome, .. } = &mut closed {
        *outcome = Some(WaitOutcome::TimedOut);
    }
    let progress = restored(script(vec![]), vec![saved_request("ask", "r1", "m1"), closed]);
    test.create(&run("k"), &progress).await;
    let message = ResponderMsg::Raw { run: addr(&run("k")), request: MessageId::new("m1").unwrap(), depth: 63 };
    test.send(&responder("r1"), message).await;
    test.run_until_idle().await;
    assert!(test.failures().await.is_empty());
    assert_eq!(unchanged(&test, "k").await, (serde_json::to_value(&progress).unwrap(), 0));
}

/// Scenario: Missing and deep values; null retained. A new eligible value is checked: 61 levels and null are kept,
/// 62 and 63 fail for good and save nothing, and a missing value fails to decode before the runner sees it.
#[tokio::test(start_paused = true)]
async fn a_new_reply_value_is_checked_before_it_is_kept() {
    let probe = Probe::default();
    let test = node(&probe).await;
    for (key, depth) in [("deepest", Some(61)), ("null", None), ("deeper", Some(62)), ("deeper-still", Some(63))] {
        let answer = Answer::Raw { value: Value::Null, depth, request: None };
        let request = Op::Request { id: "ask".into(), to: key.into(), answer, create: Some(CreateKind::Independent) };
        start(&test, key, script(vec![request, wait_op("done", "ask")])).await;
    }
    test.run_until_idle().await;
    assert_eq!(state(&test, "deepest").await.status, FlowStatus::Done);
    assert_eq!(state(&test, "null").await.status, FlowStatus::Done);
    let got = probe.got();
    assert!(got.contains(&("done".to_owned(), Value::Null)), "null is present data: {got:?}");
    assert!(got.iter().any(|(_, v)| depth(v) == 61));
    for key in ["deeper", "deeper-still"] {
        let failed: Vec<_> = test.failures().await.into_iter().filter(|r| r.envelope.to == addr(&run(key))).collect();
        assert_eq!(failed.len(), 1, "{key}");
        assert_eq!(failed[0].envelope.message_type, "Reply", "{key}");
        assert_eq!(failed[0].envelope.failures, 0, "{key}: for good");
        assert!(failed[0].error.as_deref().unwrap().contains("cannot be kept: it is more than 61"), "{key}");
        assert!(state(&test, key).await.replies.is_empty(), "{key}: nothing saved");
        assert_parked(&test, key, "done").await;
    }

    let asked = request_id(&test, "deeper", "ask").await;
    let payload = json!({"request": asked.as_str()});
    match test.send_raw(&addr(&run("deeper")), "Reply", payload, SendOptions::new()).await {
        cicada::runtime::SendResult::InvalidPayload { error, .. } => assert!(error.contains("missing field `value`")),
        other => panic!("{other:?}"),
    }
}

/// Scenario: Early reply; and Reply before wait. A reply that comes before its wait is kept without touching `armed`;
/// the new wait then saves itself closed and arms a `Continue` with that execution's correlation id, and the value
/// reaches the code only in the next execution.
#[tokio::test(start_paused = true)]
async fn an_early_reply_is_kept_and_its_new_wait_commits_closed_before_returning_it() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![ask("ask", "r1", now(json!(7))), op(|id| Op::Step { id }, "a"), wait_op("done", "ask")];
    test.create(&run("k"), &FlowState::new(script(ops))).await;
    test.send_with(&run("k"), FlowMsg::Continue, SendOptions::new().correlation_id("c1")).await;
    test.system().run_due_turns(1).await.unwrap(); // the poke
    test.system().run_due_turns(1).await.unwrap(); // the request
    test.system().run_due_turns(1).await.unwrap(); // the target, which replies at once
    test.system().run_due_turns(1).await.unwrap(); // the step
    let armed_before = state(&test, "k").await.armed.clone();
    let reply = test.pending(&run("k")).await.into_iter().find(|m| m.message_type == "Reply").unwrap();
    assert_eq!(reply.correlation_id.as_deref(), Some("c1"), "the request's correlation, through the target");
    test.system().run_due_turns(1).await.unwrap(); // the reply, before the wait
    let held = state(&test, "k").await;
    assert_eq!(held.replies.len(), 1);
    assert_eq!(held.armed, armed_before, "an early reply leaves armed");
    assert_eq!(journal(&test, "k").await, ["ask", "a"]);

    test.system().run_due_turns(1).await.unwrap(); // the new wait, closed at once
    let asked = request_id(&test, "k", "ask").await;
    assert_eq!(wait_entry(&test, "k", "done").await, (Some(WaitOutcome::Received), vec![asked]));
    assert!(probe.got().is_empty(), "the value waits for the next execution");
    assert_eq!(armed(&test, "k").await.correlation_id.as_deref(), Some("c1"));
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), json!(7))]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// Scenario: Supplied open wait with a reply. Replay commits the closure and arms before returning the value.
#[tokio::test(start_paused = true)]
async fn a_supplied_open_wait_with_its_reply_closes_on_replay() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let ops = vec![ask("ask", "r1", Answer::Later), wait_op("done", "ask")];
    let journal = vec![saved_request("ask", "r1", "m1"), open_wait("done", &["m1"], WaitMode::All, None)];
    let progress = FlowState { replies: vec![kept("m1", json!("given"))], ..restored(script(ops), journal) };
    start_with(&test, "k", progress).await;
    test.system().run_due_turns(1).await.unwrap(); // the poke
    test.system().run_due_turns(1).await.unwrap(); // the closure
    assert_eq!(
        wait_entry(&test, "k", "done").await,
        (Some(WaitOutcome::Received), vec![MessageId::new("m1").unwrap()])
    );
    assert!(probe.got().is_empty());
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), json!("given"))]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
    assert!(!test.exists(&responder("r1")).await, "the saved request is not sent again");
}

/// Scenario: Public Continue while parked; and Lost continuation after closure. A poke of a parked run normalizes and
/// parks again without a new entry; a closing continuation that is purged leaves the run stalled until a poke, which
/// replays the saved reply without asking again; an extra poke after closure replaces the armed continuation.
#[tokio::test(start_paused = true)]
async fn pokes_park_again_and_recover_a_lost_closing_continuation() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    parked(&test, "k", script(vec![ask("ask", "r1", Answer::Later), wait_op("done", "ask")]), "done").await;
    let version = test.version(&run("k")).await;
    test.send(&run("k"), FlowMsg::Continue).await;
    test.system().run_due_turns(1).await.unwrap();
    assert!(state(&test, "k").await.armed.is_some(), "the poke arms");
    test.run_until_idle().await;
    assert_parked(&test, "k", "done").await;
    assert_eq!(test.version(&run("k")).await, version + 2, "arm, then park; no entry");
    assert_eq!(journal(&test, "k").await, ["ask", "done"]);

    test.send(&responder("r1"), ResponderMsg::Answer { value: json!("v") }).await;
    test.system().run_due_turns(1).await.unwrap(); // the target
    test.system().run_due_turns(1).await.unwrap(); // the reply, which closes and arms
    let closing = armed(&test, "k").await;
    test.system().purge_mailbox(&addr(&run("k"))).await.unwrap();
    test.run_until_idle().await;
    assert!(probe.got().is_empty(), "stalled");
    assert_eq!(wait_entry(&test, "k", "done").await.0, Some(WaitOutcome::Received), "the outcome stays");
    assert_eq!(state(&test, "k").await.armed, Some(closing.id));

    test.send(&run("k"), FlowMsg::Continue).await;
    test.send(&run("k"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), json!("v"))], "replayed once");
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
    assert_eq!(responder_state(&test, "r1").await.asked, 1, "not asked again");
}

/// Scenario: Pause and target failure. Replies to a paused run wait in its mailbox and are handled after resume; a
/// target that never replies leaves the run parked.
#[tokio::test(start_paused = true)]
async fn a_paused_run_handles_its_replies_after_resume_and_a_silent_target_leaves_it_parked() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    parked(&test, "k", script(vec![ask("ask", "r1", Answer::Later), wait_op("done", "ask")]), "done").await;
    test.system().pause_mailbox(&addr(&run("k"))).await.unwrap();
    test.send(&responder("r1"), ResponderMsg::Answer { value: json!(1) }).await;
    test.run_until_idle().await;
    assert_eq!(test.pending(&run("k")).await.len(), 1, "the reply waits");
    assert!(state(&test, "k").await.replies.is_empty());
    test.system().resume_mailbox(&addr(&run("k"))).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), json!(1))]);

    let ops = vec![ask("ask", "r1", Answer::Later), wait_op("done", "ask")];
    parked(&test, "silent", script(ops), "done").await;
    test.system().delete_actor(&addr(&responder("r1"))).await.unwrap();
    assert_eq!(test.advance_time_by(Duration::from_secs(30 * 24 * 3600)).await, 0, "nothing polls");
    assert_parked(&test, "silent", "done").await;
}

/// Scenario: Typed decode failure; and Owned encoded value. A number replied through the helper with a reply type that
/// is not `Sync` decodes; a value that does not decode fails the execution for good and keeps the outcome, and fixed
/// code recovers it with a poke without asking again.
#[tokio::test(start_paused = true)]
async fn a_typed_wait_decodes_its_reply_and_fixed_code_recovers_a_mismatch() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let typed = |answer| Op::Typed { id: "ask".into(), wait: "done".into(), to: "r1".into(), answer };
    start(&test, "number", script(vec![typed(now(json!(7)))])).await;
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), json!(7))]);

    start(&test, "text", script(vec![typed(now(json!("seven")))])).await;
    test.run_until_idle().await;
    refused(&test, "text", "the saved reply of wait `done` does not decode as the type asked for").await;
    let asked = request_id(&test, "text", "ask").await;
    assert_eq!(wait_entry(&test, "text", "done").await, (Some(WaitOutcome::Received), vec![asked.clone()]));
    assert_eq!(state(&test, "text").await.replies, [kept(asked.as_str(), json!("seven"))]);

    probe.fixed.store(true, Ordering::SeqCst);
    test.send(&run("text"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(probe.got()[1], ("done".to_owned(), json!("seven")));
    assert_eq!(state(&test, "text").await.status, FlowStatus::Done);
    assert_eq!(responder_state(&test, "r1").await.asked, 2, "one request per run");
}

/// Scenario: Rejected request commit. A child create of an actor owned elsewhere rejects the commit: no entry, no id,
/// no message; after the collision is gone, a poke sends the request afresh.
#[tokio::test(start_paused = true)]
async fn a_rejected_request_commit_saves_nothing_and_a_poke_requests_afresh() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("owned"), &ResponderState::default()).await;
    let ops = vec![
        Op::Request { id: "ask".into(), to: "owned".into(), answer: now(json!(1)), create: Some(CreateKind::Child) },
        wait_op("done", "ask"),
    ];
    start(&test, "k", script(ops)).await;
    test.run_until_idle().await;
    rejected(&test, "k").await;
    assert!(journal(&test, "k").await.is_empty(), "no entry");
    assert_eq!(responder_state(&test, "owned").await.asked, 0, "no message");

    test.system().delete_actor(&addr(&responder("owned"))).await.unwrap();
    test.send(&run("k"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
    assert_eq!(probe.got(), [("done".to_owned(), json!(1))]);
    let record = test.system().get_actor(&addr(&responder("owned"))).await.unwrap().unwrap();
    assert_eq!(record.parent, Some(addr(&run("k"))));
}

/// Scenario: Deleted run. A target that replies to a deleted run has its whole commit rejected: its state and effects.
#[tokio::test(start_paused = true)]
async fn a_reply_to_a_deleted_run_rejects_the_targets_commit() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    parked(&test, "k", script(vec![ask("ask", "r1", Answer::Later), wait_op("done", "ask")]), "done").await;
    test.system().delete_actor(&addr(&run("k"))).await.unwrap();
    let before = test.version(&responder("r1")).await;
    let answer = test.send(&responder("r1"), ResponderMsg::Answer { value: json!(1) }).await;
    test.run_until_idle().await;
    let record = outcome(&test, &answer.id).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed, "the commit is rejected");
    assert_eq!(test.version(&responder("r1")).await, before, "no state of the turn");
    assert!(!test.exists(&run("k")).await, "not recreated");
}

// Upgrade.

/// Scenario: Old supported progress. A state as 0.17 wrote it, without `replies` and `cleanup`, created as raw JSON:
/// its steps and its elapsed sleep replay without calling their closures, and the run goes on with a send.
#[tokio::test(start_paused = true)]
async fn progress_saved_by_an_earlier_release_replays_and_goes_on() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&worker("w1"), &WorkerState::default()).await;
    let ops = vec![
        op(|id| Op::Step { id }, "a"),
        op(|id| Op::Step { id }, "n"),
        sleep("w", MINUTE),
        send_op("go", "w1", WorkerMsg::Work),
        op(|id| Op::Step { id }, "b"),
    ];
    let old = json!({
        "input": script(ops),
        "journal": [
            {"kind": "step", "id": "a", "result": "from-0.17"},
            {"kind": "step", "id": "n", "result": "r-n"},
            {"kind": "sleep", "id": "w", "until": "2025-12-31T00:00:00Z"},
        ],
        "status": "running",
        "armed": "0b9f6a52-5c1e-4a55-9d0c-3f1f8f0f2a13",
    });
    test.system().create_actor(&addr(&run("k")), old).await.unwrap();
    test.send(&run("k"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    let done = state(&test, "k").await;
    assert_eq!(done.status, FlowStatus::Done);
    assert_eq!(done.result::<String>("a").unwrap().unwrap(), "from-0.17");
    assert_eq!(journal(&test, "k").await, ["a", "n", "w", "go", "b"]);
    assert_eq!(probe.ids(), ["b"], "the saved steps are replayed, not run");
    assert_eq!(worker_state(&test, "w1").await.n, 1);
    let written = test.state_json(&run("k")).await.unwrap();
    assert_eq!((written["replies"].clone(), written["cleanup"].clone()), (json!([]), json!([])));
}

/// Scenario: Reserved active wait. An eligible reply from the saved target that would close a saved open join or first
/// wait fails for good with a later-feature diagnostic, and saves no reply, no outcome and no continuation.
#[tokio::test(start_paused = true)]
async fn a_reply_that_would_close_a_reserved_wait_is_refused_for_good_and_saves_nothing() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![ask("go", "r1", Answer::Later), ask("q", "r1", Answer::Later), wait_op("x", "go")];
    let join = open_wait("x", &["m1", "m2"], WaitMode::All, None);
    let first = open_wait("x", &["m1", "m2"], WaitMode::First, None);
    for (key, wait) in [("join", join), ("first", first)] {
        let saved = vec![saved_request("go", "r1", "m1"), saved_request("q", "r1", "m2"), wait];
        let progress = restored(script(ops.clone()), saved);
        test.create(&run(key), &progress).await;
        let before = unchanged(&test, key).await;
        let reply = ResponderMsg::Raw { run: addr(&run(key)), request: MessageId::new("m1").unwrap(), depth: 1 };
        test.send(&responder("r1"), reply).await;
        test.run_until_idle().await;
        let failed: Vec<_> = test.failures().await.into_iter().filter(|r| r.envelope.to == addr(&run(key))).collect();
        assert_eq!(failed.len(), 1, "{key}");
        let record = &failed[0];
        assert_eq!((record.envelope.message_type.as_str(), record.envelope.failures), ("Reply", 0), "{key}: for good");
        assert_eq!(record.envelope.from, Some(addr(&responder("r1"))), "{key}: from the saved target");
        let error = record.error.as_deref().unwrap();
        assert!(error.contains("the reply to request m1 would close wait `x`"), "{key}: {error}");
        assert!(error.contains("a join, which this release") && error.contains("a later release"), "{key}");
        let told = told(&test).await;
        assert!(told.iter().any(|f| f.message_id == record.message_id && f.cause == FailureCause::Refused), "{key}");
        assert_eq!(unchanged(&test, key).await, before, "{key}: state and version unchanged");
        let held = state(&test, key).await;
        assert!(held.replies.is_empty() && held.armed.is_none(), "{key}: no reply, no armed continuation");
        assert_eq!(wait_entry(&test, key, "x").await, (None, vec![]), "{key}: no outcome");
        assert!(test.pending(&run(key)).await.is_empty(), "{key}: no continuation");
    }
    assert!(probe.keys().is_empty() && probe.got().is_empty(), "no flow code ran");
}

/// Scenario: Sender boundary, of a trusted remote worker. The target runs in a worker that leases its turns through the
/// core `TurnSource`; the completion of its leased turn sends the reply as the target, which passes the run's sender
/// check: the value is kept, the wait closed and the flow goes on.
#[tokio::test(start_paused = true)]
async fn a_reply_from_a_target_leased_by_a_remote_worker_passes_the_sender_check() {
    let probe = Probe::default();
    let flow = ScriptFlow { probe: probe.clone() };
    let test = TestActors::builder().with(move |node| node.host(FlowActor::new(flow))).build().await.unwrap();
    let worker = cicada::runtime::Worker::builder(test.system().clone()).host(ResponderActor).build();
    worker.initialize().await.unwrap();
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![ask("ask", "r1", Answer::Later), wait_op("done", "ask"), op(|id| Op::Step { id }, "after")];
    start(&test, "k", script(ops)).await;
    test.run_until_idle().await;
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 1, "the worker leases and runs the request");
    assert_parked(&test, "k", "done").await;
    let asked = request_id(&test, "k", "ask").await;

    test.send(&responder("r1"), ResponderMsg::Answer { value: json!({"from": "worker"}) }).await;
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 1, "the answer, completed through the source");
    let reply = test.pending(&run("k")).await;
    assert_eq!(reply.len(), 1);
    assert_eq!(reply[0].from, Some(addr(&responder("r1"))), "the leased actor is the sender");
    test.run_until_idle().await;
    let held = state(&test, "k").await;
    assert_eq!(held.replies, [kept(asked.as_str(), json!({"from": "worker"}))], "accepted");
    assert_eq!(wait_entry(&test, "k", "done").await, (Some(WaitOutcome::Received), vec![asked]), "closed");
    assert_eq!(held.status, FlowStatus::Done);
    assert_eq!(probe.got(), [("done".to_owned(), json!({"from": "worker"}))]);
    assert_eq!(probe.ids(), ["after"]);
}

/// Scenario: Sender boundary, of a dead-letter redrive. A reply that failed is redriven by the dead letters, whose
/// address is the redrive's sender: the run ignores it, before checking its value, and writes nothing.
#[tokio::test(start_paused = true)]
async fn a_reply_redriven_by_the_dead_letters_is_ignored() {
    let probe = Probe::default();
    let flow = ScriptFlow { probe: probe.clone() };
    let test = TestActors::builder()
        .with(move |node| {
            node.host(FlowActor::new(flow)).host(ResponderActor).host(DeadLettersActor::default()).host(WitnessActor)
        })
        .build()
        .await
        .unwrap();
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    let letters = addr(&DeadLetters::default());
    test.system().watch(&cicada::WatchTarget::parse("test.flow").unwrap(), &letters).await.unwrap();
    test.create(&responder("r1"), &ResponderState::default()).await;
    let answer = Answer::Raw { value: Value::Null, depth: Some(62), request: None };
    parked(&test, "k", script(vec![ask("ask", "r1", answer), wait_op("done", "ask")]), "done").await;
    let failed: Vec<_> = test.failures().await.into_iter().filter(|r| r.envelope.to == addr(&run("k"))).collect();
    assert_eq!(failed.len(), 1, "the target's deep reply fails for good");
    let failed = failed[0].envelope.clone();
    assert_eq!((failed.message_type.as_str(), failed.from.clone()), ("Reply", Some(addr(&responder("r1")))));
    let kept_letters = test.state::<_, DeadLettersState>(&DeadLetters::default()).await.letters;
    assert!(kept_letters.contains_key(&failed.id), "kept as a letter");
    let before = unchanged(&test, "k").await;

    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    test.system().run_due_turns(1).await.unwrap(); // the redrive
    let redriven = test.pending(&run("k")).await;
    assert_eq!(redriven.len(), 1);
    assert_eq!((redriven[0].message_type.as_str(), redriven[0].from.clone()), ("Reply", Some(letters)));
    let redriven = redriven[0].clone();
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &redriven.id).await.unwrap().status, OutcomeStatus::Done, "ignored, not failed");
    assert_eq!(unchanged(&test, "k").await, before, "nothing written");
    assert_parked(&test, "k", "done").await;
    assert!(probe.got().is_empty());
}
