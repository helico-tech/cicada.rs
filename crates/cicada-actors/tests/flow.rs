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
//! continuations, typed decoding and its recovery, rejected request commits and replies to a deleted run; and
//! cancellation: new, parked, stalled and broken runs, armed and own ids, interrupted sleeps, waits and joins, races
//! in sequence order, queued continues, held handlers, retries, pauses, purges, deletes, rejected, lost and unanswered
//! commits, observers, hints and watches, other senders, operator cancels and a refusal by an older host. A probe
//! counts every closure invocation and notes what each wait returned.

mod common;

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::FailureCause;
use cicada::address::TypeName;
use cicada::contract::Contract;
use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
use cicada::prelude::*;
use cicada::runtime::{
    CompleteTurnResult, LeaseTurnsResult, RegisterContractResult, RenewTurnResult, RetryResult, SourceError,
    StateReport, TurnFailure, TurnReport, TurnSource,
};
use cicada::store::{DeliverNowResult, Message, NodeId, OutcomeStatus, TurnOutcome, TypeDef};
use cicada::time::{self, MAX_DELAY, Timestamp};
use cicada::turn::TurnError;
use cicada_actors::{
    Cleanup, CreateKind, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, Flow, FlowActor, FlowMsg,
    FlowReply, FlowRun, FlowState, FlowStatus, JournalEntry, MainReply, ReplyTo, Request, Step, Target, WaitMode,
    WaitOutcome, Waited,
};
use cicada_test::TestActors;
use common::*;
use futures::{FutureExt, StreamExt};
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
    /// The barrier a planned [`Op::Gate`] waits at.
    gate: Arc<tokio::sync::Notify>,
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
    /// A send of `Continue` to the run `test.flow/<key>`.
    SendRun { id: String, key: String },
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
    /// The wait `id` with a timeout of `secs` seconds and `nanos` nanoseconds for the request `request`; the probe notes
    /// `{"received": value}` or `"timedOut"`.
    TimedWait {
        id: String,
        request: String,
        secs: u64,
        #[serde(default)]
        nanos: u32,
    },
    /// A timed wait of a minute whose suspension the flow catches; the probe notes what it returns.
    SwallowTimed { id: String, request: String },
    /// A panic of the flow code itself, outside any step, when the probe plans one for `id`.
    Crash { id: String },
    /// Flow code that waits at the probe's barrier when the probe plans a wait for `id`.
    Gate { id: String },
    /// The join `id` for every one of `requests`, in that order, with a timeout of `secs` seconds when given; the probe
    /// notes the values, or `{"received": value}` and `"timedOut"` slots.
    WaitAll {
        id: String,
        requests: Vec<String>,
        #[serde(default)]
        secs: Option<u64>,
    },
    /// The join `id` for the first of `requests`, with a timeout of `secs` seconds when given; the probe notes
    /// `{"index": i, "value": value}`, wrapped as `{"received": ..}` or `"timedOut"` when timed.
    WaitFirst {
        id: String,
        requests: Vec<String>,
        #[serde(default)]
        secs: Option<u64>,
    },
    /// The requests `asks` and their join `wait`, decoded as `decode` until the probe's code is fixed, as any JSON after.
    TypedJoin { wait: String, asks: Vec<Asked>, decode: Decode },
    /// Cleanup only: notes `main_key:<id>` and the main key of `id`.
    MainKey { id: String },
    /// Cleanup only: notes `main_result:<id>` and the saved main result of `id` as any JSON, or null when absent;
    /// as a `u32` when `number`; an error is passed on, or ignored when `catch`.
    MainResult {
        id: String,
        #[serde(default)]
        number: bool,
        #[serde(default)]
        catch: bool,
    },
    /// Cleanup only: decodes the saved main result of `id` with a decoder that panics, and catches the panic.
    MainBoom { id: String },
    /// Cleanup only: the main request `id`, kept under its id for later ops, noting `main_request:<id>` and whether it
    /// is saved; an error is passed on, or ignored when `catch`.
    MainRequest {
        id: String,
        #[serde(default)]
        catch: bool,
    },
    /// Cleanup only: notes `main_reply:<request>` and `"open"`, `{"replied": value}` or `"closed"` for the request kept
    /// under `request`; an error is passed on, or ignored when `catch`.
    MainReply {
        request: String,
        #[serde(default)]
        catch: bool,
    },
    /// Cleanup only: notes `main_journal` and the ids of the main journal.
    MainJournal,
}

/// A request of [`Op::TypedJoin`]: `id` to `responder/<to>`, which answers as `answer`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Asked {
    id: String,
    to: String,
    answer: Answer,
}

/// What [`Op::TypedJoin`] decodes its replies as before the code is fixed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Decode {
    /// `u32`.
    Number,
    /// A type whose decoder panics; the flow catches the panic and goes on.
    Exploding,
}

/// A reply whose decoder panics.
struct Boom;

impl<'de> Deserialize<'de> for Boom {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        panic!("the decoder panicked")
    }
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
    /// Replies `value` to every kept route, past the helper, due at `at`.
    AnswerAt { value: Value, at: Timestamp },
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
            ResponderMsg::AnswerAt { value, at } => {
                for route in turn.state().routes.clone() {
                    let reply = FlowMsg::Reply { request: route.request.clone(), value: value.clone() };
                    let encoded = cicada::message::encode(&reply).unwrap();
                    let options = SendOptions::new().at(at);
                    turn.send_raw(&route.run, &encoded.message_type, encoded.payload, options).await?;
                }
            }
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
    /// What the cleanup does, in `Flow::on_cancel`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cleanup: Vec<Op>,
}

fn script(ops: Vec<Op>) -> Script {
    Script { ops, end: End::Ok, cleanup: Vec::new() }
}

fn ending(ops: Vec<Op>, end: End) -> Script {
    Script { ops, end, cleanup: Vec::new() }
}

/// A script that runs `ops`, and `cleanup` once cancelled.
fn finalized(ops: Vec<Op>, cleanup: Vec<Op>) -> Script {
    Script { ops, end: End::Ok, cleanup }
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

    async fn on_cancel(&self, cleanup: &mut Cleanup, script: Script) -> HandlerResult {
        let mut requests = HashMap::new();
        for op in script.cleanup {
            self.cleanup_op(cleanup, op, &mut requests).await?;
        }
        Ok(())
    }
}

impl ScriptFlow {
    /// One op of the cleanup: a lookup, or an op of the main code through the cleanup's `Step`.
    async fn cleanup_op(
        &self,
        cleanup: &mut Cleanup,
        op: Op,
        requests: &mut HashMap<String, Request<Value>>,
    ) -> HandlerResult {
        let p = &self.probe;
        let caught = |catch: bool, e: HandlerError| if catch { Ok(()) } else { Err(e) };
        match op {
            Op::MainKey { id } => {
                let key = cleanup.main_key(&id)?;
                p.note(&format!("main_key:{id}"), json!(key));
            }
            Op::MainResult { id, number, catch } => {
                let found = if number {
                    cleanup.main_result::<u32>(&id).map(|n| json!(n))
                } else {
                    cleanup.main_result::<Value>(&id).map(|v| json!(v))
                };
                match found {
                    Ok(value) => p.note(&format!("main_result:{id}"), value),
                    Err(e) => caught(catch, e)?,
                }
            }
            Op::MainBoom { id } => {
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| cleanup.main_result::<Boom>(&id)));
            }
            Op::MainRequest { id, catch } => match cleanup.main_request::<Value>(&id) {
                Ok(found) => {
                    p.note(&format!("main_request:{id}"), json!(found.is_some()));
                    if let Some(request) = found {
                        requests.insert(id, request);
                    }
                }
                Err(e) => caught(catch, e)?,
            },
            Op::MainReply { request, catch } => match cleanup.main_reply(&requests[&request]) {
                Ok(answer) => {
                    let noted = match answer {
                        MainReply::Open => json!("open"),
                        MainReply::Replied(value) => json!({"replied": value}),
                        MainReply::Closed => json!("closed"),
                    };
                    p.note(&format!("main_reply:{request}"), noted);
                }
                Err(e) => caught(catch, e)?,
            },
            Op::MainJournal => {
                let ids: Vec<&str> = cleanup.main_journal().iter().map(JournalEntry::id).collect();
                p.note("main_journal", json!(ids));
            }
            other => self.op(cleanup, other, requests).await?,
        }
        Ok(())
    }

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
            Op::SendRun { id, key } => step.send(&id, &run(&key), FlowMsg::Continue).await?,
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
            Op::TimedWait { id, request, secs, nanos } => {
                match step.wait_timeout(&id, &requests[&request], Duration::new(secs, nanos)).await? {
                    Waited::Received(value) => p.note(&id, json!({"received": value})),
                    Waited::TimedOut => p.note(&id, json!("timedOut")),
                }
            }
            Op::SwallowTimed { id, request } => match step.wait_timeout(&id, &requests[&request], MINUTE).await {
                Ok(Waited::Received(value)) => p.note(&id, json!({"received": value})),
                Ok(Waited::TimedOut) => p.note(&id, json!("timedOut")),
                Err(_) => {}
            },
            Op::Crash { id } => {
                if p.glitch(&id) {
                    panic!("the flow crashed");
                }
            }
            Op::Gate { id } => {
                if p.glitch(&id) {
                    p.gate.notified().await;
                }
            }
            Op::WaitAll { id, requests: names, secs } => {
                let handles: Vec<Request<Value>> = names.iter().map(|name| requests[name].clone()).collect();
                match secs {
                    None => {
                        let values = step.wait_all(&id, &handles).await?;
                        p.note(&id, Value::Array(values));
                    }
                    Some(secs) => {
                        let slots = step.wait_all_timeout(&id, &handles, Duration::from_secs(secs)).await?;
                        p.note(&id, Value::Array(slots.into_iter().map(noted).collect()));
                    }
                }
            }
            Op::WaitFirst { id, requests: names, secs } => {
                let handles: Vec<Request<Value>> = names.iter().map(|name| requests[name].clone()).collect();
                let won = |(index, value): (usize, Value)| json!({"index": index, "value": value});
                match secs {
                    None => {
                        let first = step.wait_first(&id, &handles).await?;
                        p.note(&id, won(first));
                    }
                    Some(secs) => match step.wait_first_timeout(&id, &handles, Duration::from_secs(secs)).await? {
                        Waited::Received(first) => p.note(&id, json!({"received": won(first)})),
                        Waited::TimedOut => p.note(&id, json!("timedOut")),
                    },
                }
            }
            Op::TypedJoin { wait, asks, decode } => {
                if p.fixed.load(Ordering::SeqCst) {
                    let values: Vec<Value> = typed_join(step, &wait, &asks).await?;
                    p.note(&wait, Value::Array(values));
                } else if let Decode::Number = decode {
                    let values: Vec<u32> = typed_join(step, &wait, &asks).await?;
                    p.note(&wait, json!(values));
                } else {
                    let _ = AssertUnwindSafe(typed_join::<Boom>(step, &wait, &asks)).catch_unwind().await;
                    step.run("after-boom", |key| p.call(key)).await?;
                }
            }
            Op::MainKey { .. }
            | Op::MainResult { .. }
            | Op::MainBoom { .. }
            | Op::MainRequest { .. }
            | Op::MainReply { .. }
            | Op::MainJournal => {
                return Err(HandlerError::permanent("a lookup of the main journal is for the cleanup"));
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

/// What the probe notes for a slot of a timed join.
fn noted(slot: Waited<Value>) -> Value {
    match slot {
        Waited::Received(value) => json!({"received": value}),
        Waited::TimedOut => json!("timedOut"),
    }
}

/// The requests `asks`, each typed by the reply `T` whatever the target's message says, and their join `wait`.
async fn typed_join<T: serde::de::DeserializeOwned + Send>(
    step: &mut Step,
    wait: &str,
    asks: &[Asked],
) -> Result<Vec<T>, HandlerError> {
    let mut handles = Vec::new();
    for Asked { id, to, answer } in asks {
        let build = |reply: ReplyTo<T>| ResponderMsg::Ask { reply: retype(&reply), answer: answer.clone() };
        handles.push(step.request(id, &responder(to), build).await?);
    }
    step.wait_all(wait, &handles).await
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
    Go {
        key: String,
        script: Script,
    },
    /// Sends the run `key` a `Cancel` with this message's correlation id, after creating it with `create` when given.
    Cancel {
        key: String,
        create: Option<Script>,
    },
}

struct ProducerActor;

impl Actor for ProducerActor {
    type Ref = Producer;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProducerMsg) -> HandlerResult {
        match message {
            ProducerMsg::Go { key, script } => {
                turn.create(&run(&key), &FlowState::new(script))?;
                turn.send(&run(&key), FlowMsg::Continue)?;
            }
            ProducerMsg::Cancel { key, create } => {
                if let Some(script) = create {
                    turn.create(&run(&key), &FlowState::new(script))?;
                }
                let mut options = SendOptions::new();
                options.correlation_id = turn.message().correlation_id.clone();
                turn.send_with(&run(&key), FlowMsg::Cancel, options)?;
            }
        }
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

/// Scenarios: Reserved message and phase; Reserved cleanup phase; and Terminal commands. A structurally valid
/// cancelling run takes a public `Continue` as a recovery of its cleanup, which normalizes and then runs the cleanup,
/// never the main code; a `Reply` that names no request is ignored and a `Cancel` is a no-op, both without a write. A
/// done or cancelled run takes `Continue`, `Reply` (a too-deep one too) and `Cancel` without writing, sending or running
/// flow code.
#[tokio::test(start_paused = true)]
async fn a_cancelling_run_runs_only_its_cleanup_and_terminal_runs_ignore_continue_reply_and_cancel() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let reply = |value: Value| FlowMsg::Reply { request: MessageId::new("m1").unwrap(), value };
    let messages = || [("continue", FlowMsg::Continue), ("reply", reply(Value::Null)), ("cancel", FlowMsg::Cancel)];
    for (name, message) in messages() {
        let key = format!("cancelling-{name}");
        let cancelling = FlowState { status: FlowStatus::Cancelling, ..FlowState::new(script(steps(&["a"]))) };
        test.create(&run(&key), &cancelling).await;
        let saved = progress(&test, &key).await;
        let sent = test.send(&run(&key), message).await;
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "{key}");
        assert_eq!(outcome(&test, &sent.id).await.unwrap().status, OutcomeStatus::Done, "{key}");
        if name == "continue" {
            assert_cancelling(&test, &key, &saved).await;
            assert_eq!(test.version(&run(&key)).await, 1, "{key}: normalized");
            test.run_until_idle().await;
            assert_cancelled(&test, &key, &saved).await;
        } else {
            assert_eq!(test.version(&run(&key)).await, 0, "{key}: no write");
            assert_eq!(state(&test, &key).await.status, FlowStatus::Cancelling, "{key}");
            assert!(test.pending(&run(&key)).await.is_empty(), "{key}: nothing armed");
        }
    }

    for status in [FlowStatus::Done, FlowStatus::Cancelled] {
        let deep = ("deep-reply", reply(nested(63)));
        for (name, message) in messages().into_iter().chain([deep]) {
            let key = format!("{status:?}-{name}");
            let terminal = FlowState { status, ..restored(script(steps(&["a"])), vec![saved("a", json!("r-a"))]) };
            test.create(&run(&key), &terminal).await;
            let sent = test.send(&run(&key), message).await;
            test.run_until_idle().await;
            assert_eq!(outcome(&test, &sent.id).await.unwrap().status, OutcomeStatus::Done, "{key}");
            let kept = test.state_json(&run(&key)).await.unwrap();
            assert_eq!(kept, serde_json::to_value(&terminal).unwrap(), "{key}: the state stays");
            assert_eq!(test.version(&run(&key)).await, 0, "{key}");
            assert!(test.pending(&run(&key)).await.is_empty(), "{key}: no continuation");
        }
    }
    assert!(probe.keys().is_empty(), "no main code ran");
    assert!(test.failures().await.is_empty());
}

/// Scenario: Replay misuse, through the runner: a saved request does not replay as a send, and a saved join, first wait
/// or wait with a deadline does not replay through `Step::wait` of one request.
#[tokio::test(start_paused = true)]
async fn a_saved_request_or_another_wait_stalls_replay_for_good() {
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
        let why = match key {
            "join" => "wait `x` is saved for all of the requests m1, m2, not for the request m1",
            "first" => "wait `x` is saved for the first of the requests m1, not for the request m1",
            _ => "wait `x` is saved with a deadline, which Step::wait",
        };
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

// Waits with a deadline (flow-wait-timeout).

/// The timed wait `id` for the request `request`, with `timeout`.
fn timed_op(id: &str, request: &str, timeout: Duration) -> Op {
    Op::TimedWait { id: id.into(), request: request.into(), secs: timeout.as_secs(), nanos: timeout.subsec_nanos() }
}

/// What the probe notes for a received reply.
fn received(value: Value) -> Value {
    json!({ "received": value })
}

/// What the probe notes for a timeout.
fn timed_out() -> Value {
    json!("timedOut")
}

/// The wait entry `id` of the journal of `key`, as its deadline, its outcome and its selection.
async fn timed_entry(
    test: &TestActors,
    key: &str,
    id: &str,
) -> (Option<Timestamp>, Option<WaitOutcome>, Vec<MessageId>) {
    match state(test, key).await.journal.iter().find(|e| e.id() == id) {
        Some(JournalEntry::Wait { until, outcome, received, .. }) => (*until, *outcome, received.clone()),
        other => panic!("{key}: no wait `{id}`: {other:?}"),
    }
}

/// Asserts that `key` waits at the open wait `wait` with the deadline `until`: its one pending message is the armed
/// `Continue`, due then; returns it.
async fn waiting(test: &TestActors, key: &str, wait: &str, until: Timestamp) -> Message {
    assert_eq!(timed_entry(test, key, wait).await, (Some(until), None, vec![]), "{key}: open until its deadline");
    let timer = armed(test, key).await;
    assert_eq!(timer.deliver_at, until, "{key}: the armed continuation is due at the deadline");
    assert_eq!(test.pending(&run(key)).await.len(), 1, "{key}: one continuation, no polling");
    timer
}

/// Scenarios: New timed wait; Timer execution and later replay; and Correlation. The deadline counts from the claim
/// of the wait's own turn, after a sleep, not from the request's; the open wait and its timer commit together; at the
/// deadline a turn of its own commits the timeout and an immediate `Continue` with the timer chain's correlation id,
/// running no code after the wait; the next execution returns `TimedOut`.
#[tokio::test(start_paused = true)]
async fn a_timed_wait_arms_its_deadline_from_its_own_claim_and_times_out_in_a_turn_of_its_own() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![
        ask("ask", "r1", Answer::Later),
        sleep("nap", MINUTE),
        timed_op("done", "ask", MINUTE),
        op(|id| Op::Step { id }, "after"),
    ];
    test.create(&run("k"), &FlowState::new(script(ops))).await;
    test.send_with(&run("k"), FlowMsg::Continue, SendOptions::new().correlation_id("c-start")).await;
    let t0 = test.now();
    test.run_until_idle().await;
    let version = test.version(&run("k")).await;
    assert_eq!(test.advance_time_by(MINUTE).await, 1, "the sleep ends: the wait's turn");
    assert_eq!(test.version(&run("k")).await, version + 1, "the open wait and its timer commit together");
    let until = after(t0, MINUTE * 2);
    let timer = waiting(&test, "k", "done", until).await;
    assert_eq!((timer.retry.clone(), timer.correlation_id.as_deref()), (Some(three_attempts()), Some("c-start")));
    assert!(probe.ids().is_empty() && probe.got().is_empty());

    let ms = Duration::from_millis(1);
    assert_eq!(test.advance_time_by(MINUTE - ms).await, 0, "no handler runs before the deadline");
    tokio::time::sleep(ms).await;
    let version = test.version(&run("k")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the timer");
    assert_eq!(test.version(&run("k")).await, version + 1, "one commit: the decision");
    assert_eq!(timed_entry(&test, "k", "done").await, (Some(until), Some(WaitOutcome::TimedOut), vec![]));
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Done);
    assert!(probe.ids().is_empty() && probe.got().is_empty(), "the deciding turn runs no code after the wait");
    let next = armed(&test, "k").await;
    assert!(next.deliver_at <= test.now(), "due now");
    assert_eq!((next.retry, next.correlation_id.as_deref()), (Some(three_attempts()), Some("c-start")));

    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), timed_out())]);
    assert_eq!(probe.ids(), ["after"]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
    assert_eq!(timed_entry(&test, "k", "done").await.0, Some(until), "the saved deadline stays");
}

/// Scenarios: Reply closes an open wait, with a deadline; and Correlation. The target's reply commits its value, the
/// received outcome, the cancel of the deadline's `Continue` and an immediate one with the reply's correlation id,
/// without flow code; the next execution returns the value.
#[tokio::test(start_paused = true)]
async fn a_reply_closes_a_timed_wait_cancels_its_deadline_and_arms_with_its_correlation() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", SLOW), op(|id| Op::Step { id }, "after")];
    start(&test, "k", script(ops)).await;
    let until = after(test.now(), SLOW);
    test.run_until_idle().await;
    let timer = waiting(&test, "k", "done", until).await;
    let version = test.version(&run("k")).await;

    let answer = ResponderMsg::Answer { value: json!({"ok": 1}) };
    test.send_with(&responder("r1"), answer, SendOptions::new().correlation_id("c-answer")).await;
    test.system().run_due_turns(1).await.unwrap(); // the target
    test.system().run_due_turns(1).await.unwrap(); // the reply
    assert_eq!(test.version(&run("k")).await, version + 1, "one commit");
    let asked = request_id(&test, "k", "ask").await;
    assert_eq!(timed_entry(&test, "k", "done").await, (Some(until), Some(WaitOutcome::Received), vec![asked.clone()]));
    assert_eq!(state(&test, "k").await.replies, [kept(asked.as_str(), json!({"ok": 1}))]);
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled, "the deadline's timer");
    let next = armed(&test, "k").await;
    assert!(next.deliver_at <= test.now(), "due now");
    assert_eq!((next.retry, next.correlation_id.as_deref()), (Some(three_attempts()), Some("c-answer")));
    assert!(probe.keys().is_empty() && probe.got().is_empty(), "the Reply turn runs no flow code");

    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), received(json!({"ok": 1})))]);
    assert_eq!(probe.ids(), ["after"]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// Scenarios: Early reply and zero duration, of a reply kept before the wait; and Correlation. The new wait commits
/// received with its deadline and an immediate `Continue` with the executing `Continue`'s correlation id; no timer.
#[tokio::test(start_paused = true)]
async fn an_early_reply_closes_a_new_timed_wait_at_once_with_its_deadline() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![ask("ask", "r1", now(json!(7))), op(|id| Op::Step { id }, "a"), timed_op("done", "ask", MINUTE)];
    test.create(&run("k"), &FlowState::new(script(ops))).await;
    test.send_with(&run("k"), FlowMsg::Continue, SendOptions::new().correlation_id("c1")).await;
    for _ in 0..5 {
        test.system().run_due_turns(1).await.unwrap(); // the poke, the request, the target, the step, the reply
    }
    assert_eq!(state(&test, "k").await.replies.len(), 1, "kept before the wait");
    let version = test.version(&run("k")).await;
    test.system().run_due_turns(1).await.unwrap(); // the new wait
    assert_eq!(test.version(&run("k")).await, version + 1);
    let asked = request_id(&test, "k", "ask").await;
    let until = after(test.now(), MINUTE);
    assert_eq!(timed_entry(&test, "k", "done").await, (Some(until), Some(WaitOutcome::Received), vec![asked]));
    let next = armed(&test, "k").await;
    assert!(next.deliver_at <= test.now(), "due now, not at the deadline");
    assert_eq!(next.correlation_id.as_deref(), Some("c1"));
    assert_eq!(test.pending(&run("k")).await.len(), 1, "no timer");
    assert!(probe.got().is_empty());
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), received(json!(7)))]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// Scenario: Correlation and timer classification. The deadline's `Continue` is a timer for observers; the immediate
/// `Continue` after the timeout is one of the other sends.
#[tokio::test(start_paused = true)]
async fn the_deadline_continue_is_a_timer_and_the_one_after_the_decision_is_not() {
    struct Picks;
    impl cicada::observer::TurnObserver for Picks {
        fn turn_started(&self, turn: &mut cicada::observer::TurnStart<'_>) -> Box<dyn cicada::observer::ObservedTurn> {
            turn.add_header_for(cicada::observer::Sends::Timers, "timer", "yes");
            turn.add_header_for(cicada::observer::Sends::Others, "timer", "no");
            Box::new(())
        }
    }
    let probe = Probe::default();
    let flow = ScriptFlow { probe: probe.clone() };
    let test = TestActors::builder()
        .with(move |node| node.host(FlowActor::new(flow)).host(ResponderActor).observe(Picks))
        .build()
        .await
        .unwrap();
    test.create(&responder("r1"), &ResponderState::default()).await;
    start(&test, "k", script(vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", MINUTE)])).await;
    test.run_until_idle().await;
    let timer = armed(&test, "k").await;
    assert_eq!(timer.headers, [("timer".to_owned(), "yes".to_owned())].into());
    tokio::time::sleep(MINUTE).await;
    test.system().run_due_turns(1).await.unwrap();
    assert_eq!(timed_entry(&test, "k", "done").await.1, Some(WaitOutcome::TimedOut));
    assert_eq!(armed(&test, "k").await.headers, [("timer".to_owned(), "no".to_owned())].into());
}

/// Scenarios: Late reply after timeout; and Later failure cannot change the branch. After the timeout commits, the
/// step after it fails transiently; a late reply from the target, also one 62 levels deep, is ignored before its value
/// is checked, writing nothing; the retry still returns `TimedOut` and asks the target nothing again.
#[tokio::test(start_paused = true)]
async fn a_committed_timeout_survives_a_later_failure_and_a_late_reply() {
    let probe = Probe::default();
    probe.plan("after", &[Act::Fail]);
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", MINUTE), op(|id| Op::Step { id }, "after")];
    start(&test, "k", script(ops)).await;
    test.run_until_idle().await;
    test.advance_time_by(MINUTE).await;
    assert_eq!(timed_entry(&test, "k", "done").await.1, Some(WaitOutcome::TimedOut));
    let failing = armed(&test, "k").await;
    assert_eq!((failing.failures, failing.last_error.as_deref()), (1, Some("the service failed")));
    assert_eq!(probe.got(), [("done".to_owned(), timed_out())], "the failed execution got the timeout");

    let before = unchanged(&test, "k").await;
    let asked = request_id(&test, "k", "ask").await;
    test.send(&responder("r1"), ResponderMsg::Raw { run: addr(&run("k")), request: asked.clone(), depth: 62 }).await;
    test.send(&responder("r1"), ResponderMsg::Answer { value: json!("late") }).await;
    test.run_until_idle().await;
    assert_eq!(unchanged(&test, "k").await, before, "late replies write nothing");
    assert!(test.failures().await.is_empty(), "the deep reply is ignored before its value is checked");
    assert!(state(&test, "k").await.replies.is_empty());

    test.advance_time_by(BACKOFF).await;
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
    assert_eq!(probe.got(), [("done".to_owned(), timed_out())], "still timed out");
    assert_eq!(probe.ids(), ["after", "after"]);
    assert_eq!(responder_state(&test, "r1").await.asked, 1, "the request is not sent again");
}

/// Scenario: Equal delivery times, in both orders. A reply due at the deadline races the deadline's `Continue`: the
/// lower sequence number is handled first and its decision wins. A poke before the deadline re-arms the timer behind
/// the reply.
#[tokio::test(start_paused = true)]
async fn a_reply_and_the_deadline_due_at_the_same_time_are_handled_in_sequence_order() {
    for reply_first in [false, true] {
        let probe = Probe::default();
        let test = node(&probe).await;
        test.create(&responder("r1"), &ResponderState::default()).await;
        start(&test, "k", script(vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", MINUTE)])).await;
        let until = after(test.now(), MINUTE);
        test.run_until_idle().await;
        test.send(&responder("r1"), ResponderMsg::AnswerAt { value: json!("v"), at: until }).await;
        test.run_until_idle().await;
        if reply_first {
            test.send(&run("k"), FlowMsg::Continue).await;
            test.run_until_idle().await;
        }
        let pending = test.pending(&run("k")).await;
        let order: Vec<_> = pending.iter().map(|m| (m.message_type.as_str(), m.deliver_at)).collect();
        let expected = if reply_first { ["Reply", "Continue"] } else { ["Continue", "Reply"] };
        assert_eq!(order, [(expected[0], until), (expected[1], until)], "reply first: {reply_first}");

        test.advance_time_by(MINUTE).await;
        let (wins, outcome) = if reply_first {
            (received(json!("v")), WaitOutcome::Received)
        } else {
            (timed_out(), WaitOutcome::TimedOut)
        };
        assert_eq!(probe.got(), [("done".to_owned(), wins)], "reply first: {reply_first}");
        assert_eq!(timed_entry(&test, "k", "done").await.1, Some(outcome), "reply first: {reply_first}");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
        assert!(test.failures().await.is_empty(), "the losing message is handled without failing");
    }
}

/// Scenario: Early reply and zero duration. Without a reply a zero timeout commits the wait open first and times out
/// in the next turn; a reply kept before the wait closes it at once; a reply queued before the zero timer wins it.
#[tokio::test(start_paused = true)]
async fn a_zero_timeout_commits_open_first_and_a_queued_or_early_reply_still_wins() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    start(&test, "k", script(vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", Duration::ZERO)])).await;
    for _ in 0..3 {
        test.system().run_due_turns(1).await.unwrap(); // the poke, the request, the target
    }
    let version = test.version(&run("k")).await;
    test.system().run_due_turns(1).await.unwrap(); // the new wait
    assert_eq!(test.version(&run("k")).await, version + 1);
    waiting(&test, "k", "done", test.now()).await;
    assert!(probe.got().is_empty());
    test.system().run_due_turns(1).await.unwrap(); // the timer, due at once
    assert_eq!(timed_entry(&test, "k", "done").await, (Some(test.now()), Some(WaitOutcome::TimedOut), vec![]));
    assert!(probe.got().is_empty(), "decided, not yet returned");
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), timed_out())]);

    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let early =
        vec![ask("ask", "r1", now(json!(1))), op(|id| Op::Step { id }, "a"), timed_op("done", "ask", Duration::ZERO)];
    start(&test, "early", script(early)).await;
    start(&test, "queued", script(vec![ask("ask", "r1", now(json!(2))), timed_op("done", "ask", Duration::ZERO)]))
        .await;
    test.run_until_idle().await;
    let got = probe.got();
    assert!(got.contains(&("done".to_owned(), received(json!(1)))), "{got:?}");
    assert!(got.contains(&("done".to_owned(), received(json!(2)))), "{got:?}");
    for key in ["early", "queued"] {
        assert_eq!(timed_entry(&test, key, "done").await.1, Some(WaitOutcome::Received), "{key}");
        assert_eq!(state(&test, key).await.status, FlowStatus::Done, "{key}");
    }
}

/// Scenario: Supplied reply past the deadline. Supplied progress with an open timed wait past its deadline commits
/// received when it keeps the reply, and timed out when it does not, each before a later turn returns it.
#[tokio::test(start_paused = true)]
async fn supplied_open_timed_progress_past_its_deadline_decides_by_its_kept_reply() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let ops = vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", MINUTE)];
    let journal =
        vec![saved_request("ask", "r1", "m1"), open_wait("done", &["m1"], WaitMode::All, Some("2020-01-01T00:00:00Z"))];
    let replied =
        FlowState { replies: vec![kept("m1", json!("given"))], ..restored(script(ops.clone()), journal.clone()) };
    let m1 = MessageId::new("m1").unwrap();
    for (key, progress, decided, selected, got) in [
        ("replied", replied, WaitOutcome::Received, vec![m1], received(json!("given"))),
        ("silent", restored(script(ops), journal), WaitOutcome::TimedOut, vec![], timed_out()),
    ] {
        start_with(&test, key, progress).await;
        test.system().run_due_turns(1).await.unwrap(); // the poke
        test.system().run_due_turns(1).await.unwrap(); // the decision
        assert_eq!(timed_entry(&test, key, "done").await.1, Some(decided), "{key}");
        assert_eq!(timed_entry(&test, key, "done").await.2, selected, "{key}");
        assert!(!probe.got().iter().any(|(_, v)| *v == got), "{key}: decided, not yet returned");
        test.run_until_idle().await;
        assert!(probe.got().contains(&("done".to_owned(), got)), "{key}");
        assert_eq!(state(&test, key).await.status, FlowStatus::Done, "{key}");
    }
    assert!(!test.exists(&responder("r1")).await, "the saved request is not sent again");
}

/// Scenarios: Recovery after a missing timer; and Reply after deadline with no timer. A deadline `Continue` that is
/// cancelled, purged or failed for good leaves the wait open past its deadline with nothing pending; a reply handled
/// then wins with its correlation id, and otherwise a public `Continue` recovers the timeout.
#[tokio::test(start_paused = true)]
async fn a_lost_deadline_continue_leaves_the_wait_open_for_a_reply_or_a_public_continue() {
    for how in ["cancel", "purge", "fail"] {
        for recovery in ["reply", "poke"] {
            let case = format!("{how} then {recovery}");
            let probe = Probe::default();
            let test = node(&probe).await;
            test.create(&responder("r1"), &ResponderState::default()).await;
            let ops = vec![
                ask("ask", "r1", Answer::Later),
                op(|id| Op::Glitch { id }, "g"),
                timed_op("done", "ask", MINUTE),
                op(|id| Op::Step { id }, "after"),
            ];
            start(&test, "k", script(ops)).await;
            let until = after(test.now(), MINUTE);
            test.run_until_idle().await;
            let timer = waiting(&test, "k", "done", until).await;
            match how {
                "cancel" => drop(test.system().cancel(&timer.id).await.unwrap()),
                "purge" => drop(test.system().purge_mailbox(&addr(&run("k"))).await.unwrap()),
                _ => probe.plan("g", &[Act::Fail, Act::Fail, Act::Fail]),
            }
            test.advance_time_by(MINUTE * 2).await;
            if how == "fail" {
                let record = outcome(&test, &timer.id).await.unwrap();
                assert_eq!((record.status, record.envelope.failures), (OutcomeStatus::Failed, 2), "{case}");
            }
            assert_eq!(test.advance_time_by(SLOW).await, 0, "{case}: nothing polls");
            assert_eq!(timed_entry(&test, "k", "done").await, (Some(until), None, vec![]), "{case}: still open");
            assert!(test.pending(&run("k")).await.is_empty(), "{case}");
            assert!(probe.got().is_empty(), "{case}");

            if recovery == "reply" {
                let answer = ResponderMsg::Answer { value: json!("late") };
                test.send_with(&responder("r1"), answer, SendOptions::new().correlation_id("c-late")).await;
                test.system().run_due_turns(1).await.unwrap(); // the target
                test.system().run_due_turns(1).await.unwrap(); // the reply, past the deadline
                assert_eq!(timed_entry(&test, "k", "done").await.1, Some(WaitOutcome::Received), "{case}");
                assert_eq!(armed(&test, "k").await.correlation_id.as_deref(), Some("c-late"), "{case}");
            } else {
                test.send(&run("k"), FlowMsg::Continue).await;
            }
            test.run_until_idle().await;
            let wins = if recovery == "reply" { received(json!("late")) } else { timed_out() };
            assert_eq!(probe.got(), [("done".to_owned(), wins)], "{case}");
            assert_eq!(state(&test, "k").await.status, FlowStatus::Done, "{case}");
        }
    }
}

/// Scenarios: Early execution; and Recovery. A public `Continue` before the deadline normalizes and re-arms at the
/// saved deadline without a new entry; a deliver-now of the timer does the same; the timeout still comes at the
/// deadline.
#[tokio::test(start_paused = true)]
async fn a_poke_or_a_deliver_now_before_the_deadline_rearms_at_the_saved_deadline() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    start(&test, "k", script(vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", MINUTE)])).await;
    let until = after(test.now(), MINUTE);
    test.run_until_idle().await;
    let timer = waiting(&test, "k", "done", until).await;
    let journal = state(&test, "k").await.journal;
    test.advance_time_by(SECOND * 10).await;

    let version = test.version(&run("k")).await;
    test.send(&run("k"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled, "replaced");
    let rearmed = waiting(&test, "k", "done", until).await;
    assert!(rearmed.id != timer.id);
    assert_eq!(test.version(&run("k")).await, version + 2, "arm now, then re-arm; no entry");
    assert_eq!(state(&test, "k").await.journal, journal);

    let DeliverNowResult::Delivered(_) = test.system().deliver_now(&rearmed.id).await.unwrap() else {
        panic!("delivered now")
    };
    assert_eq!(test.run_until_idle().await, 1);
    assert_eq!(outcome(&test, &rearmed.id).await.unwrap().status, OutcomeStatus::Done, "not cancelled by itself");
    let again = waiting(&test, "k", "done", until).await;
    assert!(again.id != rearmed.id);
    assert!(probe.got().is_empty(), "nothing shortened the deadline");

    test.advance_time_by(MINUTE).await;
    assert_eq!(probe.got(), [("done".to_owned(), timed_out())]);
}

/// Scenario: Delayed commit and pause, of a pause. Messages of a paused run keep their mailbox order: a reply due
/// before the deadline wins after resume, and one due after it loses to the timer and is ignored.
#[tokio::test(start_paused = true)]
async fn a_paused_run_decides_by_mailbox_order_after_resume() {
    for reply_first in [true, false] {
        let probe = Probe::default();
        let test = node(&probe).await;
        test.create(&responder("r1"), &ResponderState::default()).await;
        start(&test, "k", script(vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", MINUTE)])).await;
        test.run_until_idle().await;
        assert!(test.system().pause_mailbox(&addr(&run("k"))).await.unwrap());
        let answer_after = if reply_first { SECOND * 30 } else { MINUTE * 2 };
        test.advance_time_by(answer_after).await;
        test.send(&responder("r1"), ResponderMsg::Answer { value: json!("v") }).await;
        test.advance_time_by(MINUTE * 3).await;
        assert!(probe.got().is_empty() && state(&test, "k").await.replies.is_empty(), "paused");
        assert!(test.system().resume_mailbox(&addr(&run("k"))).await.unwrap());
        test.run_until_idle().await;
        let wins = if reply_first { received(json!("v")) } else { timed_out() };
        assert_eq!(probe.got(), [("done".to_owned(), wins)], "reply first: {reply_first}");
        assert!(test.failures().await.is_empty(), "reply first: {reply_first}");
    }
}

/// An aborted execution, beside the rejected decision below: the timer's execution latches the timeout, then the flow
/// code panics before the runner handles the latch: nothing commits, no branch was returned and the wait stays open; a
/// reply handled before the retry closes it as received.
#[tokio::test(start_paused = true)]
async fn a_decision_that_does_not_commit_lets_a_reply_win() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![
        ask("ask", "r1", Answer::Later),
        Op::SwallowTimed { id: "done".into(), request: "ask".into() },
        op(|id| Op::Crash { id }, "x"),
        op(|id| Op::Step { id }, "after"),
    ];
    start(&test, "k", script(ops)).await;
    test.run_until_idle().await;
    let timer = armed(&test, "k").await;
    probe.plan("x", &[Act::Panic]);
    test.advance_time_by(MINUTE).await;
    let retried = armed(&test, "k").await;
    assert_eq!(retried.id, timer.id);
    assert_eq!((retried.failures, retried.last_error.as_deref()), (1, Some("panic: the flow crashed")));
    assert_eq!(timed_entry(&test, "k", "done").await.1, None, "the latched timeout did not commit");
    assert!(probe.got().is_empty() && probe.ids().is_empty());

    test.send(&responder("r1"), ResponderMsg::Answer { value: json!("v") }).await;
    test.run_until_idle().await;
    assert_eq!(timed_entry(&test, "k", "done").await.1, Some(WaitOutcome::Received));
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled, "the retry is replaced");
    assert_eq!(probe.got(), [("done".to_owned(), received(json!("v")))]);
    assert_eq!(probe.ids(), ["after"]);
}

/// Scenario: Delayed commit and pause, of a slow commit. The wait's turn is held at a barrier past its deadline: the
/// saved deadline still counts from the claim, the core makes the past timer due at commit, and the timeout decided
/// next leaves the saved deadline as it was.
#[tokio::test(start_paused = true)]
async fn a_slow_commit_makes_the_timer_due_late_without_moving_the_saved_deadline() {
    let probe = Probe::default();
    let test = node(&probe).await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![ask("ask", "r1", Answer::Later), op(|id| Op::Gate { id }, "g"), timed_op("done", "ask", MINUTE)];
    start(&test, "k", script(ops)).await;
    for _ in 0..3 {
        test.system().run_due_turns(1).await.unwrap(); // the poke, the request, the target
    }
    probe.plan("g", &[Act::Slow]);
    let claim = test.now();
    let system = test.system().clone();
    let turn = tokio::spawn(async move { system.run_due_turns(1).await.unwrap() });
    tokio::time::sleep(MINUTE * 2).await;
    assert!(!turn.is_finished(), "held at the barrier");
    probe.gate.notify_one();
    assert_eq!(turn.await.unwrap(), 1);
    let until = after(claim, MINUTE);
    assert_eq!(timed_entry(&test, "k", "done").await, (Some(until), None, vec![]), "from the claim");
    let timer = armed(&test, "k").await;
    assert!(timer.deliver_at > until, "due at commit, after the deadline: {} > {until}", timer.deliver_at);
    test.run_until_idle().await;
    assert_eq!(timed_entry(&test, "k", "done").await, (Some(until), Some(WaitOutcome::TimedOut), vec![]));
    assert_eq!(probe.got(), [("done".to_owned(), timed_out())]);
}

/// The core `TurnSource` of a system, for a worker that hosts the flow. While `reject` is set, it turns down the
/// completion of the first turn whose new state closes a wait with `outcome` (`timedOut` or `received`), as a store that
/// rejects the commit does (cause `commit`, not retryable), applying nothing, and keeps that report; every other call
/// goes to the system.
struct RejectingDecisions {
    system: ActorSystem,
    outcome: &'static str,
    reject: Arc<AtomicBool>,
    rejected: Arc<Mutex<Vec<TurnReport>>>,
}

impl RejectingDecisions {
    /// Whether `report` sets a state whose journal holds a wait closed with the outcome this source rejects.
    fn decides(&self, report: &TurnReport) -> bool {
        let TurnReport::Returned { state: StateReport::Set(state), .. } = report else {
            return false;
        };
        state["journal"].as_array().is_some_and(|entries| entries.iter().any(|e| e["outcome"] == self.outcome))
    }
}

#[async_trait::async_trait]
impl TurnSource for RejectingDecisions {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        TurnSource::register_contract(&self.system, contract).await
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        TurnSource::list_types(&self.system).await
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        TurnSource::lease_turns(&self.system, owner, types, limit).await
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        TurnSource::renew_turn(&self.system, actor, epoch).await
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        TurnSource::release_turn(&self.system, actor, epoch).await
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        if self.decides(&report) && self.reject.swap(false, Ordering::SeqCst) {
            self.rejected.lock().unwrap().push(report);
            let reason = "the store rejected the commit".to_owned();
            return Ok(CompleteTurnResult::Rejected(TurnFailure {
                cause: FailureCause::Commit,
                retryable: false,
                reason,
            }));
        }
        TurnSource::complete_turn(&self.system, actor, epoch, message_id, report).await
    }
}

/// Runs the node's turns (the responders) and the worker's (the flow) until neither has one due.
async fn drive(test: &TestActors, worker: &cicada::runtime::Worker) {
    loop {
        let ran = test.run_until_idle().await + worker.run_due_turns(10).await.unwrap();
        if ran == 0 {
            return;
        }
    }
}

/// Scenario: Failed decision. The flow runs in a worker whose completion of the deciding turn is rejected after the
/// runner prepared the timed-out wait and its immediate `Continue`: nothing applies, so the saved wait and the version
/// stay, the prepared `Continue` is never sent, and no timeout reached flow code. The run then waits with nothing
/// pending; a reply from the target commits received, and without one a public `Continue` makes another timeout
/// decision, which commits.
#[tokio::test(start_paused = true)]
async fn a_rejected_timeout_decision_commits_nothing_and_a_reply_or_a_later_decision_wins() {
    for recovery in ["reply", "poke"] {
        let probe = Probe::default();
        let test =
            TestActors::builder().with(|node| node.host(ResponderActor).host(WitnessActor)).build().await.unwrap();
        let (reject, rejected) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(Vec::new())));
        let system = test.system().clone();
        let source =
            RejectingDecisions { system, outcome: "timedOut", reject: reject.clone(), rejected: rejected.clone() };
        let worker =
            cicada::runtime::Worker::builder(source).host(FlowActor::new(ScriptFlow { probe: probe.clone() })).build();
        worker.initialize().await.unwrap();
        test.create(&responder("r1"), &ResponderState::default()).await;
        let ops =
            vec![ask("ask", "r1", Answer::Later), timed_op("done", "ask", MINUTE), op(|id| Op::Step { id }, "after")];
        start(&test, "k", script(ops)).await;
        let until = after(test.now(), MINUTE);
        drive(&test, &worker).await;
        let timer = waiting(&test, "k", "done", until).await;
        let before = unchanged(&test, "k").await;

        reject.store(true, Ordering::SeqCst);
        tokio::time::sleep(MINUTE).await;
        assert_eq!(worker.run_due_turns(10).await.unwrap(), 1, "{recovery}: the timer's turn");
        let reports = rejected.lock().unwrap().clone();
        assert_eq!(reports.len(), 1, "{recovery}: the deciding completion was turned down");
        let TurnReport::Returned { effects, .. } = &reports[0] else { panic!("{recovery}: {:?}", reports[0]) };
        let prepared: Vec<_> = effects.sends.iter().filter(|s| s.message_type == "Continue").collect();
        assert_eq!(prepared.len(), 1, "{recovery}: the decision prepared its continuation");
        assert_eq!(prepared[0].delivery, cicada::store::Delivery::Now, "{recovery}: due now");
        let prepared = prepared[0].id.clone();

        assert_eq!(unchanged(&test, "k").await, before, "{recovery}: the saved wait and the version stay");
        assert_eq!(timed_entry(&test, "k", "done").await, (Some(until), None, vec![]), "{recovery}: still open");
        assert!(test.pending(&run("k")).await.is_empty(), "{recovery}: nothing pending");
        assert!(outcome(&test, &prepared).await.is_none(), "{recovery}: the prepared continuation was never sent");
        let record = outcome(&test, &timer.id).await.unwrap();
        assert_eq!(
            (record.status, record.error.as_deref()),
            (OutcomeStatus::Failed, Some("the store rejected the commit"))
        );
        assert!(probe.got().is_empty() && probe.ids().is_empty(), "{recovery}: no timeout reached flow code");

        if recovery == "reply" {
            test.send(&responder("r1"), ResponderMsg::Answer { value: json!("v") }).await;
            drive(&test, &worker).await;
            assert_eq!(timed_entry(&test, "k", "done").await.1, Some(WaitOutcome::Received));
            assert_eq!(probe.got(), [("done".to_owned(), received(json!("v")))]);
        } else {
            test.send(&run("k"), FlowMsg::Continue).await;
            drive(&test, &worker).await;
            assert_eq!(timed_entry(&test, "k", "done").await.1, Some(WaitOutcome::TimedOut));
            assert_eq!(probe.got(), [("done".to_owned(), timed_out())]);
        }
        assert_eq!(timed_entry(&test, "k", "done").await.0, Some(until), "{recovery}: the saved deadline stays");
        assert_eq!(probe.ids(), ["after"], "{recovery}");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Done, "{recovery}");
    }
}

// Joins (flow-wait-join).

/// A request `id` to `responder/<to>`, which keeps the route for a later answer.
fn later(id: &str, to: &str) -> Op {
    ask(id, to, Answer::Later)
}

/// The join `id` of every one of `requests`, with a timeout of `secs` seconds when given.
fn all_op(id: &str, requests: &[&str], secs: Option<u64>) -> Op {
    Op::WaitAll { id: id.into(), requests: requests.iter().map(|r| (*r).to_owned()).collect(), secs }
}

/// The join `id` of the first of `requests`, with a timeout of `secs` seconds when given.
fn first_op(id: &str, requests: &[&str], secs: Option<u64>) -> Op {
    Op::WaitFirst { id: id.into(), requests: requests.iter().map(|r| (*r).to_owned()).collect(), secs }
}

/// Creates the responders `names`.
async fn responders(test: &TestActors, names: &[&str]) {
    for name in names {
        test.create(&responder(name), &ResponderState::default()).await;
    }
}

/// Has `responder/<to>` reply `value` to its kept route, with the correlation id `correlation` when given.
async fn answer(test: &TestActors, to: &str, value: Value, correlation: Option<&str>) {
    let mut options = SendOptions::new();
    options.correlation_id = correlation.map(str::to_owned);
    test.send_with(&responder(to), ResponderMsg::Answer { value }, options).await;
}

/// The message ids of the requests `names` of the journal of `key`, in that order.
async fn request_ids(test: &TestActors, key: &str, names: &[&str]) -> Vec<MessageId> {
    let mut ids = Vec::new();
    for name in names {
        ids.push(request_id(test, key, name).await);
    }
    ids
}

/// What the probe notes for a first join won by the request at `index` with `value`.
fn won(index: usize, value: Value) -> Value {
    json!({"index": index, "value": value})
}

/// Scenarios: Partial and final all reply; Reply closes an open wait; and Partial typed results, of a received join. Each
/// partial reply to a timed join keeps its value and leaves the deadline's `Continue`, its id, due time and correlation,
/// as they were, without flow code; the last missing reply closes the join with every request in request order,
/// cancels the deadline's `Continue` and arms one due now with its own correlation id; the next execution returns the
/// values in request order.
#[tokio::test(start_paused = true)]
async fn partial_replies_keep_the_deadline_and_the_last_one_closes_the_join_in_request_order() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2", "r3"]).await;
    let ops = vec![
        later("a", "r1"),
        later("b", "r2"),
        later("c", "r3"),
        all_op("done", &["a", "b", "c"], Some(SLOW.as_secs())),
        op(|id| Op::Step { id }, "after"),
    ];
    start(&test, "k", script(ops)).await;
    let until = after(test.now(), SLOW);
    test.run_until_idle().await;
    let timer = waiting(&test, "k", "done", until).await;
    let asked = request_ids(&test, "k", &["a", "b", "c"]).await;

    for (to, value, kept) in [("r3", "c", vec![2]), ("r1", "a", vec![2, 0])] {
        let version = test.version(&run("k")).await;
        answer(&test, to, json!(value), Some("c-partial")).await;
        test.run_until_idle().await;
        assert_eq!(test.version(&run("k")).await, version + 1, "{to}: the value alone commits");
        let expected: Vec<FlowReply> = kept.iter().map(|&i| kept_reply(&asked[i], json!(["a", "b", "c"][i]))).collect();
        assert_eq!(state(&test, "k").await.replies, expected, "{to}: kept in the order accepted");
        let still = waiting(&test, "k", "done", until).await;
        assert_eq!((still.id, still.correlation_id), (timer.id.clone(), timer.correlation_id.clone()), "{to}");
        assert!(probe.keys().is_empty() && probe.got().is_empty(), "{to}: the Reply turn runs no flow code");
    }

    answer(&test, "r2", json!("b"), Some("c-last")).await;
    test.system().run_due_turns(1).await.unwrap(); // the target
    let version = test.version(&run("k")).await;
    test.system().run_due_turns(1).await.unwrap(); // the last reply
    assert_eq!(test.version(&run("k")).await, version + 1, "one commit");
    assert_eq!(timed_entry(&test, "k", "done").await, (Some(until), Some(WaitOutcome::Received), asked.clone()));
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled, "the deadline's timer");
    let next = armed(&test, "k").await;
    assert!(next.deliver_at <= test.now(), "due now");
    assert_eq!((next.retry, next.correlation_id.as_deref()), (Some(three_attempts()), Some("c-last")));
    assert!(probe.keys().is_empty() && probe.got().is_empty(), "the Reply turn runs no flow code");

    test.run_until_idle().await;
    let slots = json!([received(json!("a")), received(json!("b")), received(json!("c"))]);
    assert_eq!(probe.got(), [("done".to_owned(), slots)], "request order, not arrival order");
    assert_eq!(probe.ids(), ["after"]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// A kept reply to `request`.
fn kept_reply(request: &MessageId, value: Value) -> FlowReply {
    FlowReply { request: request.clone(), value }
}

/// Scenario: Parked or unrelated reply, of a parked join. A partial reply to an untimed join keeps `armed` null and
/// arms nothing; the last one closes it.
#[tokio::test(start_paused = true)]
async fn a_partial_reply_to_a_parked_join_keeps_it_parked() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2"]).await;
    let ops = vec![later("a", "r1"), later("b", "r2"), all_op("done", &["a", "b"], None)];
    parked(&test, "k", script(ops), "done").await;
    answer(&test, "r2", json!(2), None).await;
    test.run_until_idle().await;
    assert_parked(&test, "k", "done").await;
    assert_eq!(state(&test, "k").await.replies.len(), 1, "the value is kept");
    answer(&test, "r1", json!(1), None).await;
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), json!([1, 2]))]);
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
}

/// Scenarios: First result; Late loser and cleanup; and Reply closes an open wait, of a first join. The first reply to
/// any listed request closes the join with that request alone and arms with its correlation id; the next execution
/// returns its position. A losing target's later reply, a deep one too, is ignored before its value is checked, the
/// losing target is not stopped, and a later wait cannot name its request again.
#[tokio::test(start_paused = true)]
async fn the_first_reply_wins_a_first_join_and_a_late_loser_is_ignored() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2", "r3"]).await;
    let ops = vec![
        later("a", "r1"),
        later("b", "r2"),
        later("c", "r3"),
        first_op("quote", &["a", "b", "c"], None),
        op(|id| Op::Step { id }, "after"),
        wait_op("again", "a"),
    ];
    parked(&test, "k", script(ops), "quote").await;
    let asked = request_ids(&test, "k", &["a", "b", "c"]).await;
    answer(&test, "r2", json!(20), Some("c-win")).await;
    test.system().run_due_turns(1).await.unwrap(); // the target
    test.system().run_due_turns(1).await.unwrap(); // the reply
    assert_eq!(wait_entry(&test, "k", "quote").await, (Some(WaitOutcome::Received), vec![asked[1].clone()]));
    assert_eq!(armed(&test, "k").await.correlation_id.as_deref(), Some("c-win"));
    assert!(probe.got().is_empty(), "decided, not yet returned");
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("quote".to_owned(), won(1, json!(20)))]);
    assert_eq!(probe.ids(), ["after"]);
    refused(&test, "k", "wait `again` names the request").await;
    refused(&test, "k", "of the wait `quote`").await;

    let before = unchanged(&test, "k").await;
    test.send(&responder("r1"), ResponderMsg::Raw { run: addr(&run("k")), request: asked[0].clone(), depth: 62 }).await;
    answer(&test, "r1", json!(10), None).await;
    answer(&test, "r3", json!(30), None).await;
    test.run_until_idle().await;
    assert_eq!(unchanged(&test, "k").await, before, "late losers write nothing");
    assert!(test.failures().await.iter().all(|f| f.envelope.message_type != "Reply"), "no reply failed");
    assert!(test.exists(&responder("r1")).await, "the losing target is not stopped");
    assert_eq!(responder_state(&test, "r3").await.routes.len(), 1, "and keeps its work");
}

/// Scenario: Parked or unrelated reply, of an unrelated reply. A reply to a request the active join does not name
/// keeps its value and leaves the join and its armed continuation as they were, also when supplied progress already
/// holds enough replies for that join; a later execution then decides it.
#[tokio::test(start_paused = true)]
async fn an_unrelated_reply_leaves_the_active_join_alone() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2", "r3"]).await;
    let ops =
        vec![later("a", "r1"), later("b", "r2"), later("c", "r3"), all_op("done", &["a", "b"], Some(SLOW.as_secs()))];
    start(&test, "k", script(ops)).await;
    let until = after(test.now(), SLOW);
    test.run_until_idle().await;
    let timer = waiting(&test, "k", "done", until).await;
    answer(&test, "r3", json!(3), None).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "k").await.replies.len(), 1, "kept");
    assert_eq!(waiting(&test, "k", "done", until).await.id, timer.id, "the join and its timer stay");

    // Supplied progress: an open first join with a listed reply kept, and no armed continuation.
    let ops = vec![later("a", "r1"), later("b", "r2"), later("c", "r3"), first_op("done", &["a", "b"], None)];
    let journal = vec![
        saved_request("a", "r1", "m1"),
        saved_request("b", "r2", "m2"),
        saved_request("c", "r3", "m3"),
        open_wait("done", &["m1", "m2"], WaitMode::First, None),
    ];
    let progress = FlowState { replies: vec![kept("m1", json!(1))], ..restored(script(ops), journal) };
    test.create(&run("supplied"), &progress).await;
    let message = ResponderMsg::Raw { run: addr(&run("supplied")), request: MessageId::new("m3").unwrap(), depth: 1 };
    test.send(&responder("r3"), message).await;
    test.run_until_idle().await;
    let held = state(&test, "supplied").await;
    assert_eq!(held.replies, [kept("m1", json!(1)), kept("m3", json!([null]))]);
    assert_eq!((held.armed, wait_entry(&test, "supplied", "done").await), (None, (None, vec![])), "left alone");
    assert!(test.pending(&run("supplied")).await.is_empty());
    test.send(&run("supplied"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(wait_entry(&test, "supplied", "done").await.1, vec![MessageId::new("m1").unwrap()]);
    assert!(probe.got().contains(&("done".to_owned(), won(0, json!(1)))), "{:?}", probe.got());
}

/// Scenarios: Multiple early replies; and First waits select the earliest accepted listed reply, in both open and early
/// arrival orders. The winner is the listed request accepted first, whatever its place in the request list and
/// whether the join existed when the replies came.
#[tokio::test(start_paused = true)]
async fn the_earliest_accepted_reply_wins_a_first_join_whatever_the_order_of_its_requests() {
    for (case, early, order, index) in [
        ("open, b first", false, ["r2", "r1"], 1),
        ("open, a first", false, ["r1", "r2"], 0),
        ("early, b first", true, ["r2", "r1"], 1),
        ("early, a first", true, ["r1", "r2"], 0),
    ] {
        let probe = Probe::default();
        let test = node(&probe).await;
        responders(&test, &["r1", "r2"]).await;
        let mut ops = vec![later("a", "r1"), later("b", "r2")];
        if early {
            ops.push(sleep("nap", MINUTE));
        }
        ops.push(first_op("quote", &["a", "b"], None));
        start(&test, "k", script(ops)).await;
        test.run_until_idle().await;
        for to in order {
            answer(&test, to, json!(to), None).await;
            test.run_until_idle().await;
        }
        test.advance_time_by(MINUTE).await;
        let winner = request_id(&test, "k", ["a", "b"][index]).await;
        assert_eq!(wait_entry(&test, "k", "quote").await, (Some(WaitOutcome::Received), vec![winner]), "{case}");
        assert_eq!(probe.got(), [("quote".to_owned(), won(index, json!(order[0])))], "{case}");
        let kept = if early { 2 } else { 1 };
        assert_eq!(state(&test, "k").await.replies.len(), kept, "{case}: an early loser's value stays kept");
    }
}

/// Scenario: Supplied open first wait. Supplied progress is trusted with the order of its kept replies: the earliest
/// listed one in that array wins, and is committed before any data reaches the code.
#[tokio::test(start_paused = true)]
async fn supplied_first_progress_is_decided_by_the_order_of_its_kept_replies() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let ops = vec![later("a", "r1"), later("b", "r2"), first_op("quote", &["a", "b"], None)];
    let journal = vec![
        saved_request("a", "r1", "m1"),
        saved_request("b", "r2", "m2"),
        open_wait("quote", &["m1", "m2"], WaitMode::First, None),
    ];
    for (key, replies, index) in [("b-first", ["m2", "m1"], 1), ("a-first", ["m1", "m2"], 0)] {
        let kept_replies = replies.iter().map(|r| kept(r, json!(r))).collect();
        let progress = FlowState { replies: kept_replies, ..restored(script(ops.clone()), journal.clone()) };
        start_with(&test, key, progress).await;
        test.system().run_due_turns(1).await.unwrap(); // the poke
        test.system().run_due_turns(1).await.unwrap(); // the decision
        let winner = MessageId::new(replies[0]).unwrap();
        assert_eq!(wait_entry(&test, key, "quote").await, (Some(WaitOutcome::Received), vec![winner]), "{key}");
        assert!(!probe.got().contains(&("quote".to_owned(), won(index, json!(replies[0])))), "{key}: not yet");
        test.run_until_idle().await;
        assert!(probe.got().contains(&("quote".to_owned(), won(index, json!(replies[0])))), "{key}");
    }
}

/// Scenarios: Partial typed results, in both arrival orders; and Zero timeout and supplied progress, of a strict
/// subset. At the deadline a join with replies missing commits timed out with the kept ones in request order, and the
/// next execution returns a slot per request; a first join that times out returns no winner.
#[tokio::test(start_paused = true)]
async fn a_partial_join_times_out_with_its_kept_replies_in_request_order() {
    for order in [["r1", "r3"], ["r3", "r1"]] {
        let probe = Probe::default();
        let test = node(&probe).await;
        responders(&test, &["r1", "r2", "r3"]).await;
        let ops =
            vec![later("a", "r1"), later("b", "r2"), later("c", "r3"), all_op("done", &["a", "b", "c"], Some(60))];
        start(&test, "k", script(ops)).await;
        let until = after(test.now(), MINUTE);
        test.run_until_idle().await;
        for to in order {
            answer(&test, to, json!(to), None).await;
            test.run_until_idle().await;
        }
        let timer = waiting(&test, "k", "done", until).await;
        tokio::time::sleep(MINUTE).await;
        test.system().run_due_turns(1).await.unwrap(); // the deadline
        let asked = request_ids(&test, "k", &["a", "c"]).await;
        assert_eq!(
            timed_entry(&test, "k", "done").await,
            (Some(until), Some(WaitOutcome::TimedOut), asked),
            "{order:?}"
        );
        assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Done);
        assert!(probe.got().is_empty(), "{order:?}: decided, not yet returned");
        test.run_until_idle().await;
        let slots = json!([received(json!("r1")), timed_out(), received(json!("r3"))]);
        assert_eq!(probe.got(), [("done".to_owned(), slots)], "{order:?}");
    }

    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2"]).await;
    start(&test, "k", script(vec![later("a", "r1"), later("b", "r2"), first_op("quote", &["a", "b"], Some(60))])).await;
    test.run_until_idle().await;
    test.advance_time_by(MINUTE).await;
    assert_eq!(wait_entry(&test, "k", "quote").await, (Some(WaitOutcome::TimedOut), vec![]));
    assert_eq!(probe.got(), [("quote".to_owned(), timed_out())]);
}

/// Scenario: Zero timeout and supplied progress, of a new zero-timeout join: with a partial set of early replies it
/// commits open first and times out in the next turn with that subset; with all of them it commits received at once.
#[tokio::test(start_paused = true)]
async fn a_zero_timeout_join_commits_open_first_unless_every_reply_came_early() {
    for (key, early) in [("partial", vec!["r1"]), ("all", vec!["r1", "r2"])] {
        let probe = Probe::default();
        let test = node(&probe).await;
        responders(&test, &["r1", "r2"]).await;
        let ops = vec![later("a", "r1"), later("b", "r2"), sleep("nap", MINUTE), all_op("done", &["a", "b"], Some(0))];
        start(&test, key, script(ops)).await;
        test.run_until_idle().await;
        for to in &early {
            answer(&test, to, json!(to), None).await;
        }
        test.run_until_idle().await;
        tokio::time::sleep(MINUTE).await;
        test.system().run_due_turns(1).await.unwrap(); // the new join
        let asked = request_ids(&test, key, &["a", "b"]).await;
        if key == "partial" {
            waiting(&test, key, "done", test.now()).await;
            test.system().run_due_turns(1).await.unwrap(); // the timer, due at once
            assert_eq!(wait_entry(&test, key, "done").await, (Some(WaitOutcome::TimedOut), asked[..1].to_vec()));
        } else {
            assert_eq!(wait_entry(&test, key, "done").await, (Some(WaitOutcome::Received), asked));
        }
        assert!(probe.got().is_empty(), "{key}: decided, not yet returned");
        test.run_until_idle().await;
        let slots = if key == "partial" {
            json!([received(json!("r1")), timed_out()])
        } else {
            json!([received(json!("r1")), received(json!("r2"))])
        };
        assert_eq!(probe.got(), [("done".to_owned(), slots)], "{key}");
    }
}

/// Scenario: Zero timeout and supplied progress, of supplied progress past its deadline: with every reply kept it
/// closes received, not timed out; with a strict subset it closes timed out with that subset in request order.
#[tokio::test(start_paused = true)]
async fn supplied_timed_join_progress_past_its_deadline_decides_by_its_kept_replies() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let ops = vec![later("a", "r1"), later("b", "r2"), later("c", "r3"), all_op("done", &["a", "b", "c"], Some(60))];
    let journal = vec![
        saved_request("a", "r1", "m1"),
        saved_request("b", "r2", "m2"),
        saved_request("c", "r3", "m3"),
        open_wait("done", &["m1", "m2", "m3"], WaitMode::All, Some("2020-01-01T00:00:00Z")),
    ];
    let m = |ids: &[&str]| -> Vec<MessageId> { ids.iter().map(|i| MessageId::new(*i).unwrap()).collect() };
    for (key, replies, decided, selected) in [
        ("every", vec!["m3", "m1", "m2"], WaitOutcome::Received, m(&["m1", "m2", "m3"])),
        ("subset", vec!["m3", "m1"], WaitOutcome::TimedOut, m(&["m1", "m3"])),
    ] {
        let kept_replies = replies.iter().map(|r| kept(r, json!(r))).collect();
        let progress = FlowState { replies: kept_replies, ..restored(script(ops.clone()), journal.clone()) };
        start_with(&test, key, progress).await;
        test.system().run_due_turns(1).await.unwrap(); // the poke
        test.system().run_due_turns(1).await.unwrap(); // the decision
        assert_eq!(wait_entry(&test, key, "done").await, (Some(decided), selected), "{key}");
        test.run_until_idle().await;
        assert_eq!(state(&test, key).await.status, FlowStatus::Done, "{key}");
    }
    let slots = |middle: Value| json!([received(json!("m1")), middle, received(json!("m3"))]);
    assert_eq!(
        probe.got(),
        [("done".to_owned(), slots(received(json!("m2")))), ("done".to_owned(), slots(timed_out()))]
    );
    assert!(!test.exists(&responder("r1")).await, "the saved requests are not sent again");
}

/// Scenario: Races and immutable replay, of equal delivery times in both orders. The last missing reply, due at the
/// deadline, races the deadline's `Continue`: the lower sequence number is handled first and its decision wins.
#[tokio::test(start_paused = true)]
async fn the_last_needed_reply_and_the_deadline_due_at_the_same_time_are_handled_in_sequence_order() {
    for reply_first in [false, true] {
        let probe = Probe::default();
        let test = node(&probe).await;
        responders(&test, &["r1", "r2"]).await;
        start(&test, "k", script(vec![later("a", "r1"), later("b", "r2"), all_op("done", &["a", "b"], Some(60))]))
            .await;
        let until = after(test.now(), MINUTE);
        test.run_until_idle().await;
        answer(&test, "r2", json!("b"), None).await;
        test.send(&responder("r1"), ResponderMsg::AnswerAt { value: json!("a"), at: until }).await;
        test.run_until_idle().await;
        if reply_first {
            test.send(&run("k"), FlowMsg::Continue).await;
            test.run_until_idle().await;
        }
        let pending = test.pending(&run("k")).await;
        let order: Vec<_> = pending.iter().map(|m| (m.message_type.as_str(), m.deliver_at)).collect();
        let expected = if reply_first { ["Reply", "Continue"] } else { ["Continue", "Reply"] };
        assert_eq!(order, [(expected[0], until), (expected[1], until)], "reply first: {reply_first}");

        test.advance_time_by(MINUTE).await;
        let asked = request_ids(&test, "k", &["a", "b"]).await;
        let (decided, selected, slots) = if reply_first {
            (WaitOutcome::Received, asked, json!([received(json!("a")), received(json!("b"))]))
        } else {
            (WaitOutcome::TimedOut, asked[1..].to_vec(), json!([timed_out(), received(json!("b"))]))
        };
        assert_eq!(wait_entry(&test, "k", "done").await, (Some(decided), selected), "reply first: {reply_first}");
        assert_eq!(probe.got(), [("done".to_owned(), slots)], "reply first: {reply_first}");
        assert!(test.failures().await.is_empty(), "the losing message is handled without failing");
    }
}

/// Scenarios: Races and immutable replay, of a later failure; and Late loser and cleanup, of missing positions. After
/// a partial timeout commits, the step after it fails; late replies to the missing requests, a deep one too, are
/// ignored before their values are checked; the retry returns the same slots and sends no request again.
#[tokio::test(start_paused = true)]
async fn a_committed_join_survives_a_later_failure_and_late_replies() {
    let probe = Probe::default();
    probe.plan("after", &[Act::Fail]);
    let test = node(&probe).await;
    responders(&test, &["r1", "r2", "r3"]).await;
    let ops = vec![
        later("a", "r1"),
        later("b", "r2"),
        later("c", "r3"),
        all_op("done", &["a", "b", "c"], Some(60)),
        op(|id| Op::Step { id }, "after"),
    ];
    start(&test, "k", script(ops)).await;
    test.run_until_idle().await;
    answer(&test, "r2", json!("b"), None).await;
    test.run_until_idle().await;
    test.advance_time_by(MINUTE).await;
    let asked = request_ids(&test, "k", &["a", "b", "c"]).await;
    assert_eq!(wait_entry(&test, "k", "done").await, (Some(WaitOutcome::TimedOut), vec![asked[1].clone()]));
    let failing = armed(&test, "k").await;
    assert_eq!((failing.failures, failing.last_error.as_deref()), (1, Some("the service failed")));
    let slots = json!([timed_out(), received(json!("b")), timed_out()]);
    assert_eq!(probe.got(), [("done".to_owned(), slots.clone())]);

    let before = unchanged(&test, "k").await;
    test.send(&responder("r1"), ResponderMsg::Raw { run: addr(&run("k")), request: asked[0].clone(), depth: 62 }).await;
    answer(&test, "r1", json!("late a"), None).await;
    answer(&test, "r3", json!("late c"), None).await;
    test.run_until_idle().await;
    assert_eq!(unchanged(&test, "k").await, before, "late replies write nothing");
    assert!(test.failures().await.is_empty(), "the deep reply is ignored before its value is checked");

    test.advance_time_by(BACKOFF).await;
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
    assert_eq!(probe.got(), [("done".to_owned(), slots)], "the same positions");
    assert_eq!(probe.ids(), ["after", "after"]);
    for to in ["r1", "r2", "r3"] {
        assert_eq!(responder_state(&test, to).await.asked, 1, "{to}: asked once");
    }
}

/// Scenario: Lost timer and pause. Replies to a paused run keep their mailbox order after resume, so one due before the
/// deadline completes the join; a purged deadline `Continue` leaves the partial join open, with no polling and its
/// deadline unchanged, until the last reply closes it or a public `Continue` times it out with the kept subset.
#[tokio::test(start_paused = true)]
async fn a_paused_join_keeps_mailbox_order_and_a_purged_timer_waits_for_a_reply_or_a_poke() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2"]).await;
    let ops = || script(vec![later("a", "r1"), later("b", "r2"), all_op("done", &["a", "b"], Some(60))]);
    start(&test, "paused", ops()).await;
    test.run_until_idle().await;
    answer(&test, "r1", json!("a"), None).await;
    test.run_until_idle().await;
    assert!(test.system().pause_mailbox(&addr(&run("paused"))).await.unwrap());
    test.advance_time_by(SECOND * 30).await;
    answer(&test, "r2", json!("b"), None).await;
    test.advance_time_by(MINUTE * 3).await;
    assert_eq!(state(&test, "paused").await.replies.len(), 1, "paused");
    assert!(test.system().resume_mailbox(&addr(&run("paused"))).await.unwrap());
    test.run_until_idle().await;
    assert_eq!(wait_entry(&test, "paused", "done").await.0, Some(WaitOutcome::Received), "the reply came first");
    assert!(test.failures().await.is_empty());

    for recovery in ["reply", "poke"] {
        let key = format!("purged-{recovery}");
        let r = |n: u8| format!("{key}-r{n}");
        responders(&test, &[r(1).as_str(), r(2).as_str()]).await;
        start(&test, &key, script(vec![later("a", &r(1)), later("b", &r(2)), all_op("done", &["a", "b"], Some(60))]))
            .await;
        let until = after(test.now(), MINUTE);
        test.run_until_idle().await;
        answer(&test, &r(1), json!("a"), None).await;
        test.run_until_idle().await;
        test.system().purge_mailbox(&addr(&run(&key))).await.unwrap();
        assert_eq!(test.advance_time_by(SLOW).await, 0, "{key}: nothing polls");
        assert_eq!(timed_entry(&test, &key, "done").await, (Some(until), None, vec![]), "{key}: open, same deadline");
        let asked = request_ids(&test, &key, &["a", "b"]).await;
        if recovery == "reply" {
            answer(&test, &r(2), json!("b"), Some("c-late")).await;
            test.system().run_due_turns(1).await.unwrap(); // the target
            test.system().run_due_turns(1).await.unwrap(); // the reply, past the deadline
            assert_eq!(wait_entry(&test, &key, "done").await, (Some(WaitOutcome::Received), asked));
            assert_eq!(armed(&test, &key).await.correlation_id.as_deref(), Some("c-late"));
        } else {
            test.send(&run(&key), FlowMsg::Continue).await;
            test.run_until_idle().await;
            assert_eq!(wait_entry(&test, &key, "done").await, (Some(WaitOutcome::TimedOut), asked[..1].to_vec()));
        }
        test.run_until_idle().await;
        assert_eq!(state(&test, &key).await.status, FlowStatus::Done, "{key}");
    }
}

/// Scenario: Decode selection and failure, through the runner. A join whose second selected reply does not decode
/// fails for good naming that position, keeps its outcome, and fixed code recovers it with a poke without asking
/// again; a decoder panic that the flow catches fails the execution for good, blocking the code after it.
#[tokio::test(start_paused = true)]
async fn a_join_decodes_its_selection_and_fixed_code_recovers_a_failure() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2", "r3"]).await;
    let asks = || {
        [("a", "r1", json!(1)), ("b", "r2", json!("two")), ("c", "r3", json!(3))]
            .map(|(id, to, value)| Asked { id: id.into(), to: to.into(), answer: now(value) })
            .to_vec()
    };
    let join = |decode| Op::TypedJoin { wait: "done".into(), asks: asks(), decode };
    start(&test, "number", script(vec![join(Decode::Number)])).await;
    test.run_until_idle().await;
    let asked = request_ids(&test, "number", &["a", "b", "c"]).await;
    let why = format!("does not decode as the type asked for, at position 1 (request {})", asked[1]);
    refused(&test, "number", &why).await;
    assert_eq!(wait_entry(&test, "number", "done").await, (Some(WaitOutcome::Received), asked));

    start(&test, "exploding", script(vec![join(Decode::Exploding)])).await;
    test.run_until_idle().await;
    refused(&test, "exploding", "wait `done` panicked while its saved reply was decoded").await;
    assert!(probe.ids().is_empty(), "the marker blocks the code after the caught panic");

    probe.fixed.store(true, Ordering::SeqCst);
    for key in ["number", "exploding"] {
        test.send(&run(key), FlowMsg::Continue).await;
    }
    test.run_until_idle().await;
    assert_eq!(probe.got(), [("done".to_owned(), json!([1, "two", 3]))]);
    for key in ["number", "exploding"] {
        assert_eq!(state(&test, key).await.status, FlowStatus::Done, "{key}");
    }
    for to in ["r1", "r2", "r3"] {
        assert_eq!(responder_state(&test, to).await.asked, 2, "{to}: once per run");
    }
}

/// Scenarios: Partial and final all reply, of a failed commit; and Failed decision, of a join. The flow runs in a
/// worker whose completion of a deciding turn is rejected: the last reply's turn, which would keep the value, close the
/// join, cancel the deadline's `Continue` and arm one, or the deadline's turn, which would time the partial join out.
/// Nothing applies: the state and version stay, no prepared `Continue` is sent, and no data reaches flow code. Then the
/// deadline times the join out with the reply kept before, or a reply completes it.
#[tokio::test(start_paused = true)]
async fn a_rejected_join_decision_commits_nothing() {
    for rejected_outcome in ["received", "timedOut"] {
        let probe = Probe::default();
        let test =
            TestActors::builder().with(|node| node.host(ResponderActor).host(WitnessActor)).build().await.unwrap();
        let (reject, rejected) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(Vec::new())));
        let system = test.system().clone();
        let source = RejectingDecisions {
            system,
            outcome: rejected_outcome,
            reject: reject.clone(),
            rejected: rejected.clone(),
        };
        let worker =
            cicada::runtime::Worker::builder(source).host(FlowActor::new(ScriptFlow { probe: probe.clone() })).build();
        worker.initialize().await.unwrap();
        responders(&test, &["r1", "r2"]).await;
        let ops = vec![later("a", "r1"), later("b", "r2"), all_op("done", &["a", "b"], Some(60))];
        start(&test, "k", script(ops)).await;
        let until = after(test.now(), MINUTE);
        drive(&test, &worker).await;
        answer(&test, "r2", json!("b"), None).await;
        drive(&test, &worker).await;
        let timer = waiting(&test, "k", "done", until).await;
        let before = unchanged(&test, "k").await;

        reject.store(true, Ordering::SeqCst);
        let deciding = if rejected_outcome == "received" {
            answer(&test, "r1", json!("a"), None).await;
            test.run_until_idle().await;
            test.pending(&run("k")).await.into_iter().find(|m| m.message_type == "Reply").unwrap().id
        } else {
            tokio::time::sleep(MINUTE).await;
            timer.id.clone()
        };
        assert_eq!(worker.run_due_turns(10).await.unwrap(), 1, "{rejected_outcome}: the deciding turn");
        let reports = rejected.lock().unwrap().clone();
        assert_eq!(reports.len(), 1, "{rejected_outcome}: turned down");
        let TurnReport::Returned { effects, .. } = &reports[0] else { panic!("{:?}", reports[0]) };
        let prepared: Vec<_> = effects.sends.iter().filter(|s| s.message_type == "Continue").collect();
        assert_eq!(prepared.len(), 1, "{rejected_outcome}: the decision prepared its continuation");
        let cancels = if rejected_outcome == "received" { vec![timer.id.clone()] } else { vec![] };
        assert_eq!(effects.cancels, cancels, "{rejected_outcome}: and the cancel of the deadline's timer");
        assert_eq!(unchanged(&test, "k").await, before, "{rejected_outcome}: the state and the version stay");
        assert!(outcome(&test, &prepared[0].id).await.is_none(), "{rejected_outcome}: never sent");
        let record = outcome(&test, &deciding).await.unwrap();
        assert_eq!(
            (record.status, record.error.as_deref()),
            (OutcomeStatus::Failed, Some("the store rejected the commit"))
        );
        assert!(probe.got().is_empty(), "{rejected_outcome}: no data reached flow code");

        let asked = request_ids(&test, "k", &["a", "b"]).await;
        let slots = if rejected_outcome == "received" {
            assert_eq!(armed(&test, "k").await.id, timer.id, "the deadline's timer stays");
            tokio::time::sleep(MINUTE).await;
            drive(&test, &worker).await;
            assert_eq!(wait_entry(&test, "k", "done").await, (Some(WaitOutcome::TimedOut), asked[1..].to_vec()));
            json!([timed_out(), received(json!("b"))])
        } else {
            assert!(test.pending(&run("k")).await.is_empty(), "nothing pending");
            answer(&test, "r1", json!("a"), None).await;
            drive(&test, &worker).await;
            assert_eq!(wait_entry(&test, "k", "done").await, (Some(WaitOutcome::Received), asked));
            json!([received(json!("a")), received(json!("b"))])
        };
        assert_eq!(probe.got(), [("done".to_owned(), slots)], "{rejected_outcome}");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Done, "{rejected_outcome}");
    }
}

/// Scenarios: One-request all structural equivalence; and Rollout and contract equality, of progress that 0.18.2 wrote.
/// States of single waits as a node of 0.18.2 saved them, without and with a deadline, created as raw JSON, replay
/// through joins of one request, and a join of one request saves the entry that `Step::wait` saves.
#[tokio::test(start_paused = true)]
async fn single_waits_saved_by_0_18_2_replay_through_joins_of_one_request() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let ops = vec![
        later("a", "r1"),
        later("b", "r2"),
        all_op("one", &["a"], None),
        all_op("timed", &["b"], Some(60)),
        op(|id| Op::Step { id }, "after"),
    ];
    let old = json!({
        "input": script(ops),
        "journal": [
            {"kind": "send", "id": "a", "to": "responder/r1", "message": "m1", "create": null, "reply": true},
            {"kind": "send", "id": "b", "to": "responder/r2", "message": "m2", "create": null, "reply": true},
            {"kind": "wait", "id": "one", "requests": ["m1"], "mode": "all", "until": null, "outcome": "received",
                "received": ["m1"]},
            {"kind": "wait", "id": "timed", "requests": ["m2"], "mode": "all", "until": "2026-01-01T00:01:00Z",
                "outcome": "timedOut", "received": []},
        ],
        "status": "running",
        "armed": null,
        "replies": [{"request": "m1", "value": "from-0.18.2"}],
        "cleanup": [],
    });
    test.system().create_actor(&addr(&run("k")), old).await.unwrap();
    test.send(&run("k"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "k").await.status, FlowStatus::Done);
    assert_eq!(probe.got(), [("one".to_owned(), json!(["from-0.18.2"])), ("timed".to_owned(), json!([timed_out()]))]);
    assert_eq!(probe.ids(), ["after"]);

    responders(&test, &["r1"]).await;
    let ops = |join: bool| {
        script(vec![later("a", "r1"), if join { all_op("one", &["a"], None) } else { wait_op("one", "a") }])
    };
    for (key, join) in [("by-wait", false), ("by-join", true)] {
        parked(&test, key, ops(join), "one").await;
    }
    let mut shapes = Vec::new();
    for key in ["by-wait", "by-join"] {
        let mut entry = serde_json::to_value(state(&test, key).await.journal[1].clone()).unwrap();
        entry["requests"] = json!(["<a>"]);
        shapes.push(entry);
    }
    assert_eq!(shapes[0], shapes[1], "the same entry");
}

// Cancellation (flow-cancellation).

/// The saved progress of `key` as JSON: its state without the status and the armed continuation.
async fn progress(test: &TestActors, key: &str) -> Value {
    let mut state = test.state_json(&run(key)).await.unwrap();
    let fields = state.as_object_mut().unwrap();
    fields.remove("status");
    fields.remove("armed");
    state
}

/// Asserts that `key` is cancelled with no armed continuation and nothing pending, and still holds `saved`.
async fn assert_cancelled(test: &TestActors, key: &str, saved: &Value) {
    let held = state(test, key).await;
    assert_eq!((held.status, held.armed), (FlowStatus::Cancelled, None), "{key}");
    assert_eq!(&progress(test, key).await, saved, "{key}: the input, the journals and the replies stay");
    assert!(test.pending(&run(key)).await.is_empty(), "{key}: nothing pending");
}

/// Asserts that `key` is cancelling with its cleanup `Continue` armed and pending, due now with the flow's policy, and
/// still holds `saved`.
async fn assert_cancelling(test: &TestActors, key: &str, saved: &Value) -> Message {
    let held = state(test, key).await;
    assert_eq!(held.status, FlowStatus::Cancelling, "{key}");
    assert_eq!(&progress(test, key).await, saved, "{key}: the input, the journals and the replies stay");
    let cleanup = armed(test, key).await;
    assert_eq!((cleanup.message_type.as_str(), cleanup.deliver_at), ("Continue", test.now()), "{key}");
    assert_eq!(cleanup.retry, Some(three_attempts()), "{key}: the flow's policy");
    assert_eq!(test.pending(&run(key)).await.len(), 1, "{key}: only the cleanup's continuation");
    cleanup
}

/// Scenario: Never started or stalled. A run that was never started, a parked join with a partial reply, a run whose
/// sleep timer was cancelled and a run whose replay failed for good each become cancelling in one commit of the
/// `Cancel` turn, without flow code, keeping their saved progress, with the cleanup's `Continue` armed; that
/// `Continue`, with the default cleanup of the test flow's empty script, marks them cancelled in a commit of its own.
/// Later replies and time change nothing.
#[tokio::test(start_paused = true)]
async fn cancel_ends_a_new_parked_timerless_or_broken_run_without_flow_code() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2"]).await;
    test.create(&run("new"), &FlowState::new(script(steps(&["a"])))).await;
    start(&test, "parked", script(vec![later("a", "r1"), later("b", "r2"), all_op("both", &["a", "b"], None)])).await;
    start(&test, "timerless", script(vec![sleep("nap", MINUTE), op(|id| Op::Step { id }, "b")])).await;
    start_with(&test, "broken", restored(script(steps(&["a", "b"])), vec![saved("b", json!("r-b"))])).await;
    test.run_until_idle().await;
    answer(&test, "r1", json!("one"), None).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "parked").await.replies.len(), 1, "a partial reply is kept");
    drop(test.system().cancel(&armed(&test, "timerless").await.id).await.unwrap());
    refused(&test, "broken", "calls step `a` where its journal holds step `b`").await;
    for key in ["parked", "timerless", "broken"] {
        assert_eq!(state(&test, key).await.status, FlowStatus::Running, "{key}: a core control does not cancel a flow");
    }

    for key in ["new", "parked", "timerless", "broken"] {
        let (before, version) = (progress(&test, key).await, test.version(&run(key)).await);
        let sent = test.send(&run(key), FlowMsg::Cancel).await;
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "{key}: the cancel's turn");
        assert_eq!(outcome(&test, &sent.id).await.unwrap().status, OutcomeStatus::Done, "{key}");
        assert_eq!(test.version(&run(key)).await, version + 1, "{key}: one commit");
        assert_cancelling(&test, key, &before).await;
        assert_eq!(test.run_until_idle().await, 1, "{key}: the cleanup's turn");
        assert_eq!(test.version(&run(key)).await, version + 2, "{key}: a commit of its own");
        assert_cancelled(&test, key, &before).await;
    }
    let before = progress(&test, "parked").await;
    answer(&test, "r2", json!("two"), None).await;
    test.advance_time_by(MINUTE * 2).await;
    assert_cancelled(&test, "parked", &before).await;
    assert!(probe.keys().is_empty() && probe.got().is_empty(), "no flow code ran");
}

/// How often a [`Counted`] input was cloned.
static CLONES: AtomicUsize = AtomicUsize::new(0);
/// How often [`CountedFlow::run`] was called.
static RUNS: AtomicUsize = AtomicUsize::new(0);

/// An input that counts its clones.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Counted(u32);

impl Clone for Counted {
    fn clone(&self) -> Self {
        CLONES.fetch_add(1, Ordering::SeqCst);
        Counted(self.0)
    }
}

/// `test.counted`: one sleep of a minute, counting its executions.
struct CountedFlow;

impl Flow for CountedFlow {
    const TYPE: &'static str = "test.counted";
    type Input = Counted;

    async fn run(&self, step: &mut Step, _input: Counted) -> HandlerResult {
        RUNS.fetch_add(1, Ordering::SeqCst);
        step.sleep("nap", MINUTE).await
    }
}

/// Scenario: Never started or stalled, of the input: a `Cancel` neither calls `Flow::run` nor clones the input.
#[tokio::test(start_paused = true)]
async fn cancel_neither_runs_the_flow_nor_clones_its_input() {
    let test = TestActors::builder().with(|node| node.host(FlowActor::new(CountedFlow))).build().await.unwrap();
    let counted = FlowRun::<CountedFlow>::named("c");
    test.create(&counted, &FlowState::new(Counted(1))).await;
    test.send(&counted, FlowMsg::Continue).await;
    test.run_until_idle().await;
    let (clones, runs) = (CLONES.load(Ordering::SeqCst), RUNS.load(Ordering::SeqCst));
    assert_eq!(runs, 1, "the sleep's execution");
    test.send(&counted, FlowMsg::Cancel).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the cancel");
    let held: FlowState<Counted> = test.state(&counted).await;
    assert_eq!(held.status, FlowStatus::Cancelling);
    assert_eq!((CLONES.load(Ordering::SeqCst), RUNS.load(Ordering::SeqCst)), (clones, runs), "no clone, no run");
    assert_eq!(test.run_until_idle().await, 1, "the default cleanup");
    let held: FlowState<Counted> = test.state(&counted).await;
    assert_eq!((held.status, held.armed), (FlowStatus::Cancelled, None));
    let after = (CLONES.load(Ordering::SeqCst), RUNS.load(Ordering::SeqCst));
    assert_eq!(after, (clones + 1, runs), "the cleanup's turn clones the input for the hook, and never runs the flow");
}

/// Scenario: Armed message and current id. Another armed message still pending ends cancelled, and no handler runs
/// for it; an armed id that is no longer pending, or that names the `Cancel` itself in supplied progress, is not
/// cancelled, and the `Cancel` completes normally.
#[tokio::test(start_paused = true)]
async fn cancel_cancels_another_armed_continuation_and_never_its_own_message() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "sleeping", script(vec![sleep("nap", MINUTE), op(|id| Op::Step { id }, "b")])).await;
    test.run_until_idle().await;
    let timer = armed(&test, "sleeping").await;
    let before = progress(&test, "sleeping").await;
    test.send(&run("sleeping"), FlowMsg::Cancel).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled, "the armed timer");
    assert_cancelled(&test, "sleeping", &before).await;
    assert_eq!(test.advance_time_by(MINUTE * 2).await, 0, "no handler runs for the cancelled timer");

    let (done, own) = (MessageId::random(), MessageId::random());
    for (key, id) in [("finished", &done), ("own", &own)] {
        let held =
            FlowState { armed: Some(id.clone()), ..restored(script(steps(&["a"])), vec![saved("a", json!("r-a"))]) };
        test.create(&run(key), &held).await;
    }
    let paused = addr(&run("finished"));
    test.system().pause_mailbox(&paused).await.unwrap();
    test.send_with(&run("finished"), FlowMsg::Continue, SendOptions { id: Some(done.clone()), ..SendOptions::new() })
        .await;
    drop(test.system().cancel(&done).await.unwrap());
    test.system().resume_mailbox(&paused).await.unwrap();
    assert_eq!(outcome(&test, &done).await.unwrap().status, OutcomeStatus::Cancelled, "no longer pending");
    for (key, id) in [("finished", None), ("own", Some(own))] {
        let before = progress(&test, key).await;
        let sent = test.send_with(&run(key), FlowMsg::Cancel, SendOptions { id, ..SendOptions::new() }).await;
        test.run_until_idle().await;
        assert_eq!(outcome(&test, &sent.id).await.unwrap().status, OutcomeStatus::Done, "{key}: completes normally");
        assert_cancelled(&test, key, &before).await;
    }
    assert!(probe.keys().is_empty());
}

/// Scenario: Interrupted waits and sleeps. A `Cancel` cancels a sleep's timer, finds a parked wait with nothing armed,
/// and cancels the deadline of a timed join that has kept a partial reply; the open waits, the journal and the replies
/// stay as they were. Later replies, one too deep to keep among them, and the time of the deadlines change nothing.
#[tokio::test(start_paused = true)]
async fn cancel_interrupts_a_sleep_a_parked_wait_and_a_timed_join_and_later_replies_are_ignored() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1", "r2", "r3", "p1"]).await;
    start(&test, "sleep", script(vec![sleep("nap", MINUTE), op(|id| Op::Step { id }, "after")])).await;
    start(&test, "parked", script(vec![later("a", "p1"), wait_op("done", "a"), op(|id| Op::Step { id }, "after")]))
        .await;
    let join = vec![
        later("a", "r1"),
        later("b", "r2"),
        later("c", "r3"),
        all_op("done", &["a", "b", "c"], Some(MINUTE.as_secs())),
        op(|id| Op::Step { id }, "after"),
    ];
    start(&test, "join", script(join)).await;
    test.run_until_idle().await;
    answer(&test, "r1", json!("a"), None).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "join").await.replies.len(), 1, "a partial reply");
    let timers = [
        ("sleep", Some(armed(&test, "sleep").await.id)),
        ("parked", None),
        ("join", Some(armed(&test, "join").await.id)),
    ];

    for (key, timer) in timers {
        let before = progress(&test, key).await;
        test.send(&run(key), FlowMsg::Cancel).await;
        test.run_until_idle().await;
        assert_cancelled(&test, key, &before).await;
        if let Some(timer) = timer {
            assert_eq!(outcome(&test, &timer).await.unwrap().status, OutcomeStatus::Cancelled, "{key}: its timer");
        }
    }
    let kept = [("parked", unchanged(&test, "parked").await), ("join", unchanged(&test, "join").await)];
    let deep = Answer::Raw { value: Value::Null, depth: Some(63), request: None };
    for (to, run_key) in [("p1", "parked"), ("r2", "join")] {
        answer(&test, to, json!("late"), None).await;
        let asked = request_id(&test, run_key, if to == "p1" { "a" } else { "b" }).await;
        let reply = serde_json::from_value(json!({"run": format!("test.flow/{run_key}"), "request": asked.as_str()}));
        test.send(&responder(to), ResponderMsg::Ask { reply: reply.unwrap(), answer: deep.clone() }).await;
    }
    test.send(&responder("r3"), ResponderMsg::Answer { value: json!("c") }).await;
    test.advance_time_by(MINUTE * 2).await;
    for (key, before) in kept {
        assert_eq!(unchanged(&test, key).await, before, "{key}: no state or version change");
    }
    assert!(
        test.failures().await.iter().all(|f| f.envelope.to.type_name().as_str() != "test.flow"),
        "ignored, not failed"
    );
    assert!(probe.keys().is_empty() && probe.got().is_empty(), "no flow code after the cancel");
}

/// Scenario: Reply or timer decision race, of a deadline. A `Cancel` and the deadline's `Continue` due at the same time
/// are handled in sequence order. Cancel first: the timer is cancelled, the wait stays open and the cleanup's
/// `Continue` is armed. Deadline first: the timeout commits, and the `Cancel` cancels the `Continue` it armed, so no
/// main code consumes the decision.
#[tokio::test(start_paused = true)]
async fn a_cancel_and_the_deadline_due_at_the_same_time_are_handled_in_sequence_order() {
    for cancel_first in [true, false] {
        let probe = Probe::default();
        let test = node(&probe).await;
        test.create(&responder("r1"), &ResponderState::default()).await;
        let ops = vec![later("ask", "r1"), timed_op("done", "ask", MINUTE), op(|id| Op::Step { id }, "after")];
        start(&test, "k", script(ops)).await;
        let until = after(test.now(), MINUTE);
        let at = SendOptions::new().at(until);
        if cancel_first {
            test.send_with(&run("k"), FlowMsg::Cancel, at.clone()).await;
        }
        test.run_until_idle().await;
        let timer = state(&test, "k").await.armed.unwrap();
        if !cancel_first {
            test.send_with(&run("k"), FlowMsg::Cancel, at).await;
        }
        let pending = test.pending(&run("k")).await;
        let order: Vec<_> = pending.iter().map(|m| (m.message_type.as_str(), m.deliver_at)).collect();
        let expected = if cancel_first { ["Cancel", "Continue"] } else { ["Continue", "Cancel"] };
        assert_eq!(order, [(expected[0], until), (expected[1], until)], "cancel first: {cancel_first}");

        tokio::time::sleep(MINUTE).await;
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "cancel first: {cancel_first}: the first due");
        let first = state(&test, "k").await;
        let phase = if cancel_first { FlowStatus::Cancelling } else { FlowStatus::Running };
        assert_eq!(first.status, phase, "the timeout arms a continuation, the cancel the cleanup's");
        let decision = first.armed;
        assert_ne!(decision.as_ref(), Some(&timer), "cancel first: {cancel_first}");
        test.advance_time_by(MINUTE).await;
        let decided = if cancel_first { None } else { Some(WaitOutcome::TimedOut) };
        assert_eq!(
            timed_entry(&test, "k", "done").await,
            (Some(until), decided, vec![]),
            "cancel first: {cancel_first}"
        );
        let held = state(&test, "k").await;
        assert_eq!((held.status, held.armed), (FlowStatus::Cancelled, None), "cancel first: {cancel_first}");
        let timer_outcome = outcome(&test, &timer).await.unwrap().status;
        if cancel_first {
            assert_eq!(timer_outcome, OutcomeStatus::Cancelled, "the deadline's timer");
        } else {
            assert_eq!(timer_outcome, OutcomeStatus::Done, "the timeout committed");
            let decision = outcome(&test, &decision.unwrap()).await.unwrap().status;
            assert_eq!(decision, OutcomeStatus::Cancelled, "the decision's continuation, by the cancel");
        }
        assert!(probe.got().is_empty() && probe.ids().is_empty(), "cancel first: {cancel_first}: no flow code");
        assert!(test.pending(&run("k")).await.is_empty());
        assert!(test.failures().await.is_empty());
    }
}

/// Scenario: Reply or timer decision race, of a reply. A `Cancel` and the reply that closes a parked wait due at the
/// same time are handled in sequence order. Cancel first: the reply comes while the run is cancelling, so it is kept
/// for the cleanup and the main wait stays open. Reply first: the wait commits received with its value, and the
/// `Cancel` cancels the `Continue` it armed before code consumes it.
#[tokio::test(start_paused = true)]
async fn a_cancel_and_the_closing_reply_due_at_the_same_time_are_handled_in_sequence_order() {
    for cancel_first in [true, false] {
        let probe = Probe::default();
        let test = node(&probe).await;
        test.create(&responder("r1"), &ResponderState::default()).await;
        let ops = vec![later("ask", "r1"), wait_op("done", "ask"), op(|id| Op::Step { id }, "after")];
        parked(&test, "k", script(ops), "done").await;
        let until = after(test.now(), MINUTE);
        let reply = ResponderMsg::AnswerAt { value: json!("v"), at: until };
        if cancel_first {
            test.send_with(&run("k"), FlowMsg::Cancel, SendOptions::new().at(until)).await;
            test.send(&responder("r1"), reply).await;
            test.run_until_idle().await;
        } else {
            test.send(&responder("r1"), reply).await;
            test.run_until_idle().await;
            test.send_with(&run("k"), FlowMsg::Cancel, SendOptions::new().at(until)).await;
        }
        let pending = test.pending(&run("k")).await;
        let order: Vec<_> = pending.iter().map(|m| (m.message_type.as_str(), m.deliver_at)).collect();
        let expected = if cancel_first { ["Cancel", "Reply"] } else { ["Reply", "Cancel"] };
        assert_eq!(order, [(expected[0], until), (expected[1], until)], "cancel first: {cancel_first}");
        let asked = request_id(&test, "k", "ask").await;

        test.advance_time_by(MINUTE * 2).await;
        let held = state(&test, "k").await;
        assert_eq!((held.status, held.armed.clone()), (FlowStatus::Cancelled, None), "cancel first: {cancel_first}");
        if cancel_first {
            assert_eq!(wait_entry(&test, "k", "done").await, (None, vec![]), "the main wait stays open");
            assert_eq!(held.replies, [kept_reply(&asked, json!("v"))], "kept for the cleanup, which did not use it");
        } else {
            assert_eq!(wait_entry(&test, "k", "done").await, (Some(WaitOutcome::Received), vec![asked.clone()]));
            assert_eq!(held.replies, [kept_reply(&asked, json!("v"))], "the decision stays saved");
        }
        assert!(probe.got().is_empty() && probe.ids().is_empty(), "cancel first: {cancel_first}: no flow code");
        assert!(test.pending(&run("k")).await.is_empty());
        assert!(test.failures().await.is_empty(), "the losing message is handled without failing");
    }
}

/// Scenario: Finishing and queued continuations. A public `Continue` queued before the `Cancel` normalizes as ever,
/// replacing the armed timer; the `Cancel` then cancels the fresh continuation and arms the cleanup's, a `Continue`
/// queued after the `Cancel` normalizes the cleanup, and one after cancelled does nothing. A `Cancel` that runs after
/// the last step committed but before the run recorded done still cancels it, although every effect ran, and the
/// cleanup runs before cancelled; once done is recorded a `Cancel` is a no-op.
#[tokio::test(start_paused = true)]
async fn queued_continues_keep_mailbox_order_around_a_cancel_and_a_cancel_after_the_last_step_still_wins() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "queued", script(vec![sleep("nap", MINUTE), op(|id| Op::Step { id }, "after")])).await;
    test.run_until_idle().await;
    let timer = armed(&test, "queued").await;
    let version = test.version(&run("queued")).await;
    let before = test.send(&run("queued"), FlowMsg::Continue).await;
    let cancel = test.send(&run("queued"), FlowMsg::Cancel).await;
    let behind = test.send(&run("queued"), FlowMsg::Continue).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the earlier Continue");
    assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled, "replaced as ever");
    let fresh = armed(&test, "queued").await;
    assert_eq!(test.version(&run("queued")).await, version + 1);
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the cancel");
    assert_eq!(outcome(&test, &fresh.id).await.unwrap().status, OutcomeStatus::Cancelled, "by the cancel");
    let cleanup = armed(&test, "queued").await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the later Continue, queued before the cleanup's");
    assert_eq!(outcome(&test, &cleanup.id).await.unwrap().status, OutcomeStatus::Cancelled, "normalized");
    assert_eq!(
        state(&test, "queued").await.status,
        FlowStatus::Cancelling,
        "the later Continue normalizes the cleanup"
    );
    assert_eq!(test.run_until_idle().await, 1, "its fresh continuation runs the cleanup");
    for message in [&before, &cancel, &behind] {
        assert_eq!(outcome(&test, &message.id).await.unwrap().status, OutcomeStatus::Done);
    }
    assert_eq!(test.version(&run("queued")).await, version + 4);
    assert_eq!(state(&test, "queued").await.status, FlowStatus::Cancelled);
    let after = test.send(&run("queued"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &after.id).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.version(&run("queued")).await, version + 4, "a Continue after cancelled writes nothing");
    assert!(test.pending(&run("queued")).await.is_empty(), "and arms nothing");
    assert!(probe.ids().is_empty(), "the step after the sleep never ran");

    start(&test, "last", script(steps(&["a"]))).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the poke arms the execution");
    test.send(&run("last"), FlowMsg::Cancel).await;
    assert_eq!(test.run_until_idle().await, 3, "the last step, the cancel before the completion, the cleanup");
    assert_eq!(probe.ids(), ["a"], "its effect ran");
    let held = state(&test, "last").await;
    assert_eq!((held.status, held.journal), (FlowStatus::Cancelled, vec![saved("a", json!("r-a"))]));

    start(&test, "done", script(steps(&["a"]))).await;
    test.run_until_idle().await;
    let version = test.version(&run("done")).await;
    test.send(&run("done"), FlowMsg::Cancel).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "done").await.status, FlowStatus::Done, "a no-op once done");
    assert_eq!(test.version(&run("done")).await, version);
}

/// Scenario: Running handler and retry backoff. A `Cancel` queued while the armed execution is held in a step does not
/// preempt it: no turn of the run can be claimed meanwhile, the step commits, and the `Cancel` then cancels its
/// successor. A `Cancel` due during an armed retry's backoff cancels the retry without running the step again; the
/// failed attempt's effect is not compensated.
#[tokio::test(start_paused = true)]
async fn cancel_waits_for_a_running_handler_and_cancels_a_retry_in_backoff() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(
        &test,
        "held",
        script(vec![op(|id| Op::Gate { id }, "g"), op(|id| Op::Step { id }, "a"), op(|id| Op::Step { id }, "b")]),
    )
    .await;
    test.system().run_due_turns(1).await.unwrap(); // the poke
    probe.plan("g", &[Act::Slow]);
    let system = test.system().clone();
    let turn = tokio::spawn(async move { system.run_due_turns(1).await.unwrap() });
    tokio::time::sleep(SECOND).await;
    let cancel = test.send(&run("held"), FlowMsg::Cancel).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 0, "the run's turn is held; the cancel waits");
    assert!(!turn.is_finished(), "held at the barrier");
    probe.gate.notify_one();
    assert_eq!(turn.await.unwrap(), 1);
    assert_eq!(journal(&test, "held").await, ["a"], "the held step committed first");
    let successor = armed(&test, "held").await;
    assert_eq!(test.run_until_idle().await, 2, "the cancel, then the cleanup");
    assert_eq!(outcome(&test, &cancel.id).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(outcome(&test, &successor.id).await.unwrap().status, OutcomeStatus::Cancelled);
    assert_eq!(state(&test, "held").await.status, FlowStatus::Cancelled);
    assert_eq!(probe.ids(), ["a"], "b never ran");

    probe.plan("x", &[Act::Fail]);
    start(&test, "backoff", script(steps(&["x"]))).await;
    test.run_until_idle().await;
    let retry = armed(&test, "backoff").await;
    assert_eq!(retry.failures, 1, "in backoff");
    test.send(&run("backoff"), FlowMsg::Cancel).await;
    assert_eq!(test.run_until_idle().await, 2, "the cancel, due before the retry, then the cleanup");
    assert_eq!(outcome(&test, &retry.id).await.unwrap().status, OutcomeStatus::Cancelled);
    assert_eq!(test.advance_time_by(BACKOFF * 2).await, 0);
    assert_eq!(probe.ids(), ["a", "x"], "the step ran once, and is neither run again nor compensated");
    assert_eq!(state(&test, "backoff").await.status, FlowStatus::Cancelled);
}

/// Scenario: Pause purge and delete, of a pause and a purge. A paused run takes its `Cancel` when it is resumed, before
/// its later timer; a purge of a paused mailbox discards the queued `Cancel` with the timer, and the run stays running.
#[tokio::test(start_paused = true)]
async fn a_paused_run_takes_its_cancel_after_resume_and_a_purged_cancel_leaves_it_running() {
    for purge in [false, true] {
        let probe = Probe::default();
        let test = node(&probe).await;
        start(&test, "k", script(vec![sleep("nap", MINUTE), op(|id| Op::Step { id }, "after")])).await;
        test.run_until_idle().await;
        let before = progress(&test, "k").await;
        assert!(test.system().pause_mailbox(&addr(&run("k"))).await.unwrap());
        let cancel = test.send(&run("k"), FlowMsg::Cancel).await;
        assert_eq!(test.advance_time_by(MINUTE * 2).await, 0, "purge: {purge}: paused");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Running, "purge: {purge}: not before resume");
        if purge {
            assert_eq!(test.system().purge_mailbox(&addr(&run("k"))).await.unwrap(), Some(2));
        }
        assert!(test.system().resume_mailbox(&addr(&run("k"))).await.unwrap());
        test.advance_time_by(MINUTE).await;
        if purge {
            assert_ne!(outcome(&test, &cancel.id).await.map(|o| o.status), Some(OutcomeStatus::Done), "never handled");
            assert_eq!(state(&test, "k").await.status, FlowStatus::Running, "the run stays running");
        } else {
            assert_eq!(outcome(&test, &cancel.id).await.unwrap().status, OutcomeStatus::Done);
            assert_cancelled(&test, "k", &before).await;
        }
        assert!(probe.ids().is_empty(), "purge: {purge}: the step after the sleep never ran");
    }
}

/// What [`CancelFaults`] does to the completion of a turn whose new state has a planned status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// Turns it down as a store that rejects the commit does (cause `commit`, not retryable), applying nothing.
    Reject,
    /// Loses it on the way to the store, applying nothing: the worker abandons the turn and its lease runs out.
    Drop,
    /// Applies it and loses the answer: the worker cannot know that it applied.
    LoseAnswer,
}

/// The core `TurnSource` of a system, for a worker that hosts the flow: the completion of a turn whose new state has
/// the status of the next planned fault (`cancelling` at the phase's entry, `cancelled` at its end) meets that fault;
/// every other call goes to the system.
struct CancelFaults {
    system: ActorSystem,
    faults: Arc<Mutex<VecDeque<(&'static str, Fault)>>>,
}

#[async_trait::async_trait]
impl TurnSource for CancelFaults {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        TurnSource::register_contract(&self.system, contract).await
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        TurnSource::list_types(&self.system).await
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        TurnSource::lease_turns(&self.system, owner, types, limit).await
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        TurnSource::renew_turn(&self.system, actor, epoch).await
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        TurnSource::release_turn(&self.system, actor, epoch).await
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        let status = match &report {
            TurnReport::Returned { state: StateReport::Set(s), .. } => s["status"].as_str().map(str::to_owned),
            _ => None,
        };
        let fault = {
            let mut faults = self.faults.lock().unwrap();
            let planned = faults.front().is_some_and(|(at, _)| status.as_deref() == Some(*at));
            if planned { faults.pop_front().map(|(_, fault)| fault) } else { None }
        };
        match fault {
            None => TurnSource::complete_turn(&self.system, actor, epoch, message_id, report).await,
            Some(Fault::Reject) => Ok(CompleteTurnResult::Rejected(TurnFailure {
                cause: FailureCause::Commit,
                retryable: false,
                reason: "the store rejected the commit".to_owned(),
            })),
            Some(Fault::Drop) => Err(SourceError::Invalid("the commit was lost on its way".to_owned())),
            Some(Fault::LoseAnswer) => {
                TurnSource::complete_turn(&self.system, actor, epoch, message_id, report).await?;
                Err(SourceError::Unavailable("the answer was lost".to_owned()))
            }
        }
    }
}

/// A node that hosts the responders and the witness, and a worker on it that hosts the test flow on `probe` through a
/// [`CancelFaults`], with the faults it will meet.
async fn faulty(probe: &Probe) -> (TestActors, cicada::runtime::Worker, Arc<Mutex<VecDeque<(&'static str, Fault)>>>) {
    let test = TestActors::builder().with(|node| node.host(ResponderActor).host(WitnessActor)).build().await.unwrap();
    witness(&test, "test.flow").await;
    let faults = Arc::new(Mutex::new(VecDeque::new()));
    let source = CancelFaults { system: test.system().clone(), faults: faults.clone() };
    let worker =
        cicada::runtime::Worker::builder(source).host(FlowActor::new(ScriptFlow { probe: probe.clone() })).build();
    worker.initialize().await.unwrap();
    (test, worker, faults)
}

/// The node's lease of a turn.
const LEASE: Duration = Duration::from_secs(30);

/// Scenarios: Atomic failure; and Unknown commit acknowledgement. A rejected commit of the `Cancel` saves neither the
/// status, the cleanup's `Continue` nor the cancel of the armed timer, and fails the `Cancel` without its sender's
/// retry policy retrying it; a fresh `Cancel` then commits. A commit whose answer was lost did apply: a reread shows
/// the run cancelling, and a fresh `Cancel` is a no-op; the cleanup then marks it cancelled.
#[tokio::test(start_paused = true)]
async fn a_cancel_commits_whole_or_not_at_all_and_an_unknown_answer_is_settled_by_a_reread() {
    for fault in [Fault::Reject, Fault::LoseAnswer] {
        let probe = Probe::default();
        let (test, worker, faults) = faulty(&probe).await;
        start(&test, "k", script(vec![sleep("nap", MINUTE), op(|id| Op::Step { id }, "after")])).await;
        drive(&test, &worker).await;
        let timer = armed(&test, "k").await;
        let (before, saved) = (unchanged(&test, "k").await, progress(&test, "k").await);
        faults.lock().unwrap().push_back(("cancelling", fault));
        let retried = SendOptions::new().retry(three_attempts());
        let cancel = test.send_with(&run("k"), FlowMsg::Cancel, retried).await;
        assert_eq!(worker.run_due_turns(10).await.unwrap(), 1, "{fault:?}: the cancel's turn");
        assert!(faults.lock().unwrap().is_empty(), "{fault:?}: the fault was met");
        let record = outcome(&test, &cancel.id).await.unwrap();
        if fault == Fault::Reject {
            assert_eq!(unchanged(&test, "k").await, before, "neither the status nor anything else is saved");
            assert_eq!(armed(&test, "k").await.id, timer.id, "the timer is still pending");
            let failed = (record.status, record.envelope.failures, record.error.as_deref());
            assert_eq!(failed, (OutcomeStatus::Failed, 0, Some("the store rejected the commit")), "not retried");
            test.run_until_idle().await;
            assert_eq!(told(&test).await.len(), 1, "a failed message, as for any actor");
            test.send(&run("k"), FlowMsg::Cancel).await;
            drive(&test, &worker).await;
        } else {
            assert_eq!(record.status, OutcomeStatus::Done, "it applied although the worker got no answer");
            let version = test.version(&run("k")).await;
            assert_eq!(version, before.1 + 1);
            assert_eq!(state(&test, "k").await.status, FlowStatus::Cancelling, "a reread shows the phase entered");
            let again = test.send(&run("k"), FlowMsg::Cancel).await;
            drive(&test, &worker).await;
            assert_eq!(outcome(&test, &again.id).await.unwrap().status, OutcomeStatus::Done, "a fresh cancel");
            assert_eq!(test.version(&run("k")).await, version + 1, "is a no-op: the one commit is the cleanup's");
        }
        assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled, "{fault:?}");
        assert_cancelled(&test, "k", &saved).await;
        tokio::time::sleep(MINUTE * 2).await;
        drive(&test, &worker).await;
        assert!(probe.ids().is_empty(), "{fault:?}: no progress undone, no flow restarted");
    }
}

/// Scenario: Atomic failure, of a commit that does not get through after the handler returned. The `Cancel` stays
/// leased until its lease runs out, then is claimed again and commits, with no failure counted against a retry
/// policy; a `Cancel` whose commit never gets through fails at the claim limit, leaving the run running with its timer.
#[tokio::test(start_paused = true)]
async fn an_unapplied_cancel_commit_is_reclaimed_after_its_lease_and_fails_at_the_claim_limit() {
    for drops in [1, 5] {
        let probe = Probe::default();
        let (test, worker, faults) = faulty(&probe).await;
        start(&test, "k", script(vec![sleep("nap", Duration::from_secs(3600)), op(|id| Op::Step { id }, "after")]))
            .await;
        drive(&test, &worker).await;
        let timer = armed(&test, "k").await;
        let (before, saved) = (unchanged(&test, "k").await, progress(&test, "k").await);
        faults.lock().unwrap().extend(std::iter::repeat_n(("cancelling", Fault::Drop), drops));
        let cancel = test.send_with(&run("k"), FlowMsg::Cancel, SendOptions::new().retry(three_attempts())).await;
        for claim in 0..drops {
            assert_eq!(worker.run_due_turns(10).await.unwrap(), 1, "{drops}: claim {claim}");
            assert_eq!(unchanged(&test, "k").await, before, "{drops}: claim {claim}: nothing applied");
            assert!(outcome(&test, &cancel.id).await.is_none(), "{drops}: claim {claim}: still leased");
            assert_eq!(worker.run_due_turns(10).await.unwrap(), 0, "{drops}: claim {claim}: until the lease runs out");
            tokio::time::sleep(LEASE + SECOND).await;
        }
        drive(&test, &worker).await;
        let record = outcome(&test, &cancel.id).await.unwrap();
        if drops == 1 {
            let reclaimed = (record.status, record.envelope.attempt, record.envelope.failures);
            assert_eq!(reclaimed, (OutcomeStatus::Done, 2, 0), "claimed again, not retried");
            assert_eq!(outcome(&test, &timer.id).await.unwrap().status, OutcomeStatus::Cancelled);
            assert_cancelled(&test, "k", &saved).await;
        } else {
            assert_eq!(record.status, OutcomeStatus::Failed);
            assert_eq!(record.error.as_deref(), Some("claim limit: 5 lost claims"));
            test.run_until_idle().await;
            assert_eq!(told(&test).await[0].cause, FailureCause::ClaimLimit);
            assert_eq!(unchanged(&test, "k").await, before, "the run stays running");
            assert_eq!(armed(&test, "k").await.id, timer.id, "with its timer");
        }
        assert!(probe.ids().is_empty());
    }
}

/// Scenario: Pause purge and delete, of a delete and of an absent run. A delete purges a queued `Cancel` with the run,
/// and no handler runs; a `Cancel` to an absent run is refused like any send, and in a turn its commit is rejected; a
/// turn that creates a run and sends it `Cancel` commits both, and the new run is cancelled before it ever starts.
#[tokio::test(start_paused = true)]
async fn delete_purges_a_queued_cancel_an_absent_run_refuses_one_and_a_create_with_cancel_commits_together() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "deleted", script(vec![sleep("nap", MINUTE), op(|id| Op::Step { id }, "after")])).await;
    test.run_until_idle().await;
    let later = SendOptions::new().delay(MINUTE / 2);
    let cancel = test.send_with(&run("deleted"), FlowMsg::Cancel, later).await;
    test.system().delete_actor(&addr(&run("deleted"))).await.unwrap();
    assert!(test.pending(&run("deleted")).await.is_empty(), "the queued cancel is purged with the run");
    assert_eq!(test.advance_time_by(MINUTE * 2).await, 0, "no cancellation handler runs");
    assert_ne!(outcome(&test, &cancel.id).await.map(|o| o.status), Some(OutcomeStatus::Done));
    assert!(!test.exists(&run("deleted")).await);

    let refused = test.system().send(&run("deleted"), FlowMsg::Cancel).await.unwrap();
    assert!(matches!(refused, cicada::SendResult::NoSuchInstance), "{refused:?}");
    let turn = test.send(&Producer, ProducerMsg::Cancel { key: "absent".into(), create: None }).await;
    test.run_until_idle().await;
    let record = outcome(&test, &turn.id).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed, "the producer's commit is rejected");
    assert!(!test.exists(&run("absent")).await);

    let create = Some(script(steps(&["a"])));
    test.send(&Producer, ProducerMsg::Cancel { key: "fresh".into(), create }).await;
    test.run_until_idle().await;
    assert_cancelled(
        &test,
        "fresh",
        &json!({"input": {"ops": [{"op": "step", "id": "a"}], "end": "ok"},
        "journal": [], "replies": [], "cleanup": []}),
    )
    .await;
    assert!(probe.keys().is_empty());
}

/// The turns a node committed, as an observer is told them: the actor, the message's id, type and correlation id,
/// the outcome and how many messages the commit sent.
#[derive(Clone, Default)]
struct Ended(Arc<Mutex<Vec<Observed>>>);

/// A committed turn: the actor, the message's id, type and correlation id, the outcome and the count of sends.
type Observed = (ActorAddress, MessageId, String, Option<String>, TurnOutcome, usize);

/// One turn of [`Ended`].
struct EndedTurn {
    log: Ended,
    actor: ActorAddress,
    message: Message,
}

impl TurnObserver for Ended {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        Box::new(EndedTurn { log: self.clone(), actor: turn.actor().clone(), message: turn.message().clone() })
    }
}

impl ObservedTurn for EndedTurn {
    fn turn_ended(self: Box<Self>, end: &TurnEnd<'_>) {
        let EndedTurn { log, actor, message } = *self;
        let turn =
            (actor, message.id, message.message_type, message.correlation_id, end.outcome().clone(), end.sent().len());
        log.0.lock().unwrap().push(turn);
    }
}

/// Scenarios: Observers and watches; and Targets and senders. Another actor cancels a run that has requested a job of
/// a responder and created a child worker: observers see an ordinary done `Cancel` turn with the incoming correlation
/// id that sends only the cleanup's `Continue`, then that `Continue`'s done turn with the same correlation id; the
/// cancelled timer has no turn, the witness of the type is told nothing, the run stays readable, a
/// change hint names it, and the target and the child keep existing; the target's later reply is ignored.
#[tokio::test(start_paused = true)]
async fn a_cancel_from_another_actor_is_an_ordinary_turn_with_a_hint_and_no_notification_and_leaves_targets_alone() {
    let probe = Probe::default();
    let ended = Ended::default();
    let (flow, observer) = (ScriptFlow { probe: probe.clone() }, ended.clone());
    let test = TestActors::builder()
        .with(move |node| {
            node.host(FlowActor::new(flow))
                .host(ProducerActor)
                .host(WitnessActor)
                .host(WorkerActor)
                .host(ResponderActor)
                .observe(observer)
        })
        .build()
        .await
        .unwrap();
    test.create(&Producer, &NoState {}).await;
    witness(&test, "test.flow").await;
    test.create(&responder("r1"), &ResponderState::default()).await;
    let ops = vec![
        later("job", "r1"),
        create_op("kid", "w1", CreateKind::Child, initial(0), WorkerMsg::Work),
        sleep("nap", MINUTE),
        wait_op("done", "job"),
    ];
    start(&test, "k", script(ops)).await;
    test.run_until_idle().await;
    let timer = armed(&test, "k").await;
    let before = progress(&test, "k").await;
    let mut changes = test.system().changes("test.flow/");
    assert_eq!(changes.next().await, Some(cicada::ActorChange::Resync));

    let options = SendOptions::new().correlation_id("c-cancel");
    let order = test.send_with(&Producer, ProducerMsg::Cancel { key: "k".into(), create: None }, options).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &order.id).await.unwrap().status, OutcomeStatus::Done);
    assert_cancelled(&test, "k", &before).await;
    let turns = ended.0.lock().unwrap().clone();
    let of_run: Vec<_> = turns.iter().filter(|t| t.0 == addr(&run("k")) && t.1 != timer.id).collect();
    let types: Vec<&str> = of_run.iter().skip_while(|t| t.2 != "Cancel").map(|t| t.2.as_str()).collect();
    assert_eq!(types, ["Cancel", "Continue"], "the cancel, then the cleanup's continuation");
    let cancel_turn = of_run.iter().find(|t| t.2 == "Cancel").unwrap();
    let (actor, _, _, correlation, outcome_of_cancel, sent) = (*cancel_turn).clone();
    assert_eq!((actor, correlation.as_deref()), (addr(&run("k")), Some("c-cancel")), "the incoming correlation id");
    assert_eq!((outcome_of_cancel, sent), (TurnOutcome::Done, 1), "an ordinary done turn that sends the cleanup's");
    let cleanup_turn = of_run.last().unwrap();
    assert_eq!((cleanup_turn.3.as_deref(), &cleanup_turn.4, cleanup_turn.5), (Some("c-cancel"), &TurnOutcome::Done, 0));
    assert!(turns.iter().all(|t| t.1 != timer.id), "no turn for the cancelled timer");
    let cancel = test.list("test.flow/").await;
    assert_eq!(cancel, [addr(&run("k"))], "the run stays readable");
    let hint = tokio::time::timeout(MINUTE, changes.next()).await.unwrap().unwrap();
    assert!(matches!(&hint, cicada::ActorChange::Changed { address, .. } if *address == addr(&run("k"))), "{hint:?}");
    assert_eq!(state(&test, "k").await.status, FlowStatus::Cancelled, "the reread after the hint");

    test.send(&responder("r1"), ResponderMsg::Answer { value: json!("late") }).await;
    test.advance_time_by(MINUTE * 2).await;
    assert_cancelled(&test, "k", &before).await;
    assert!(test.exists(&responder("r1")).await && test.exists(&worker("w1")).await, "the target and the child stay");
    assert_eq!(worker_state(&test, "w1").await.n, 1);
    assert!(turns.iter().chain(ended.0.lock().unwrap().iter()).all(|t| t.0 != addr(&Witness)), "no notification");
    assert!(told(&test).await.is_empty() && test.pending(&Witness).await.is_empty());
    assert!(probe.got().is_empty());
}

/// Scenario: Core controls and later cleanup, of an operator's cancel of the running turn. The armed execution, held in
/// its code, is cancelled by the node at its next heartbeat: the message ends cancelled and the run stalls, still
/// running. A `FlowMsg::Cancel` then ends it, and a later `Continue` does not reopen it.
#[tokio::test(start_paused = true)]
async fn an_operator_cancel_of_the_running_turn_stalls_the_run_and_a_flow_cancel_ends_it() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "k", script(vec![op(|id| Op::Gate { id }, "g"), op(|id| Op::Step { id }, "a")])).await;
    test.system().run_due_turns(1).await.unwrap(); // the poke
    let execution = armed(&test, "k").await;
    probe.plan("g", &[Act::Slow]);
    let system = test.system().clone();
    let turn = tokio::spawn(async move { system.run_due_turns(1).await.unwrap() });
    tokio::time::sleep(SECOND).await;
    assert!(test.system().cancel_turn(&addr(&run("k"))).await.unwrap(), "a turn is running");
    assert_eq!(turn.await.unwrap(), 1);
    assert_eq!(outcome(&test, &execution.id).await.unwrap().status, OutcomeStatus::Cancelled);
    let held = state(&test, "k").await;
    assert_eq!((held.status, held.armed), (FlowStatus::Running, Some(execution.id)), "stalled, not cancelled");
    assert!(test.pending(&run("k")).await.is_empty());

    let before = progress(&test, "k").await;
    test.send(&run("k"), FlowMsg::Cancel).await;
    test.run_until_idle().await;
    assert_cancelled(&test, "k", &before).await;
    let version = test.version(&run("k")).await;
    test.send(&run("k"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(test.version(&run("k")).await, version, "not reopened");
    assert_cancelled(&test, "k", &before).await;
    assert!(probe.ids().is_empty());
}

/// A host of `test.flow` as 0.18.3 was, on today's contract: it refuses `Cancel` for good and ignores the rest.
struct OldHost;

impl Actor for OldHost {
    type Ref = FlowRun<ScriptFlow>;
    type State = FlowState<Script>;

    async fn handle(&self, _turn: &mut Turn<Self>, message: FlowMsg) -> HandlerResult {
        match message {
            FlowMsg::Cancel => {
                Err(HandlerError::permanent("this release of cicada-actors refuses FlowMsg::Cancel for good"))
            }
            _ => Ok(()),
        }
    }
}

/// Scenario: Refused Cancel recovery. A worker of the old release refuses a `Cancel` for good, although the contract
/// is the same, and the run stays running. Once a worker of this release hosts the type, a fresh `Cancel`, an
/// operator's retry of the failed one or its redrive by the dead letters, from another sender, cancels it.
#[tokio::test(start_paused = true)]
async fn a_cancel_refused_by_an_older_host_is_recovered_by_a_fresh_cancel_a_retry_or_a_redrive_after_the_upgrade() {
    for recovery in ["fresh", "retry", "redrive"] {
        let probe = Probe::default();
        let test = TestActors::builder()
            .with(|node| node.host(DeadLettersActor::default()).host(WitnessActor))
            .build()
            .await
            .unwrap();
        test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
        let letters = addr(&DeadLetters::default());
        test.system().watch(&cicada::WatchTarget::parse("test.flow").unwrap(), &letters).await.unwrap();
        let old = cicada::runtime::Worker::builder(test.system().clone()).host(OldHost).build();
        old.initialize().await.unwrap();
        let progress_saved = restored(script(steps(&["a", "b"])), vec![saved("a", json!("r-a"))]);
        test.create(&run("k"), &progress_saved).await;
        let before = progress(&test, "k").await;
        let refused = test.send(&run("k"), FlowMsg::Cancel).await;
        assert_eq!(old.run_due_turns(10).await.unwrap(), 1, "{recovery}");
        test.run_until_idle().await;
        let record = outcome(&test, &refused.id).await.unwrap();
        assert_eq!(record.status, OutcomeStatus::Failed, "{recovery}: refused for good");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Running, "{recovery}: a refusal does not cancel");
        drop(old);

        let new = cicada::runtime::Worker::builder(test.system().clone())
            .host(FlowActor::new(ScriptFlow { probe: probe.clone() }))
            .build();
        new.initialize().await.unwrap();
        match recovery {
            "fresh" => drop(test.send(&run("k"), FlowMsg::Cancel).await),
            "retry" => {
                let RetryResult::Accepted(_) = test.system().retry_message(&refused.id).await.unwrap() else {
                    panic!("retried")
                };
            }
            _ => {
                let letters_state = test.state::<_, DeadLettersState>(&DeadLetters::default()).await.letters;
                assert!(letters_state.contains_key(&refused.id), "kept as a letter");
                test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: refused.id.clone() }).await;
                test.run_until_idle().await;
                let redriven = test.pending(&run("k")).await;
                assert_eq!(
                    (redriven[0].message_type.as_str(), redriven[0].from.clone()),
                    ("Cancel", Some(letters.clone()))
                );
            }
        }
        assert_eq!(new.run_due_turns(10).await.unwrap(), 1, "{recovery}: the cancel");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Cancelling, "{recovery}");
        assert_eq!(new.run_due_turns(10).await.unwrap(), 1, "{recovery}: the cleanup");
        assert_cancelled(&test, "k", &before).await;
        assert!(probe.keys().is_empty(), "{recovery}");
    }
}

// Finalization (flow-finalizer).

/// The ids of the cleanup journal of `key`.
async fn cleanup_journal(test: &TestActors, key: &str) -> Vec<String> {
    state(test, key).await.cleanup.iter().map(|e| e.id().to_owned()).collect()
}

/// The main journal of `key` as JSON, to compare byte for byte.
async fn main_json(test: &TestActors, key: &str) -> Value {
    test.state_json(&run(key)).await.unwrap()["journal"].clone()
}

/// A cancelling state of `script` with the saved `journal` and `cleanup`, and nothing armed.
fn cancelling(script: Script, journal: Vec<JournalEntry>, cleanup: Vec<JournalEntry>) -> FlowState<Script> {
    FlowState { status: FlowStatus::Cancelling, cleanup, ..restored(script, journal) }
}

/// Scenarios: Default and custom hooks; Separate namespaces. The `Cancel` turn commits cancelling and the cleanup's
/// `Continue` without running the cleanup; each cleanup turn then saves one new entry of the cleanup journal with the
/// next continuation, which keeps the `Cancel`'s correlation id, and a last turn of its own marks the run cancelled.
/// A cleanup step with the id of a main step gets the key `<run>/cancel/<id>`; the main journal stays as it was.
#[tokio::test(start_paused = true)]
async fn a_cleanup_saves_one_entry_per_turn_under_its_own_keys_and_a_last_turn_marks_the_run_cancelled() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let ops = vec![op(|id| Op::Step { id }, "a"), sleep("nap", MINUTE), op(|id| Op::Step { id }, "after")];
    start(&test, "k", finalized(ops, steps(&["a", "b"]))).await;
    test.run_until_idle().await;
    let main = main_json(&test, "k").await;
    let version = test.version(&run("k")).await;
    test.send_with(&run("k"), FlowMsg::Cancel, SendOptions::new().correlation_id("c-cancel")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the cancel");
    assert_eq!(probe.ids(), ["a"], "no cleanup in the cancel's turn");
    for (turn, saved) in [(1, vec!["a"]), (2, vec!["a", "b"])] {
        let next = armed(&test, "k").await;
        assert_eq!(next.correlation_id.as_deref(), Some("c-cancel"), "turn {turn}");
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "turn {turn}");
        assert_eq!(cleanup_journal(&test, "k").await, saved, "turn {turn}: one new entry");
        assert_eq!(state(&test, "k").await.status, FlowStatus::Cancelling, "turn {turn}");
        assert_eq!(test.version(&run("k")).await, version + 1 + turn, "turn {turn}");
    }
    assert_eq!(test.run_until_idle().await, 1, "the turn that replays both and marks the run cancelled");
    let held = state(&test, "k").await;
    assert_eq!((held.status, held.armed.clone()), (FlowStatus::Cancelled, None));
    assert_eq!(test.version(&run("k")).await, version + 4);
    assert_eq!(probe.keys(), ["test.flow/k/a", "test.flow/k/cancel/a", "test.flow/k/cancel/b"]);
    assert_eq!(main_json(&test, "k").await, main, "the main journal never changes");
    assert_eq!(held.cleanup, [saved("a", json!("r-a")), saved("b", json!("r-b"))]);
    assert!(test.pending(&run("k")).await.is_empty() && test.failures().await.is_empty());
}

/// Scenario: No normal-finally behavior. A run whose flow returned `Ok` is done, and a `Cancel` then changes nothing and
/// runs no cleanup; a run cancelled by 0.18.4, before this release, is never reopened or finalized by `Continue`,
/// `Reply` or `Cancel`.
#[tokio::test(start_paused = true)]
async fn a_done_run_or_one_cancelled_by_an_earlier_release_never_runs_its_cleanup() {
    let probe = Probe::default();
    let test = node(&probe).await;
    start(&test, "done", finalized(steps(&["a"]), steps(&["undo"]))).await;
    test.run_until_idle().await;
    let earlier = FlowState {
        status: FlowStatus::Cancelled,
        ..restored(finalized(steps(&["a", "b"]), steps(&["undo"])), vec![saved("a", json!("r-a"))])
    };
    test.create(&run("earlier"), &earlier).await;
    let reply = FlowMsg::Reply { request: MessageId::random(), value: json!(1) };
    for (key, message) in
        [("done", FlowMsg::Cancel), ("earlier", FlowMsg::Continue), ("earlier", reply), ("earlier", FlowMsg::Cancel)]
    {
        let before = unchanged(&test, key).await;
        test.send(&run(key), message).await;
        test.run_until_idle().await;
        assert_eq!(unchanged(&test, key).await, before, "{key}: nothing written");
        assert!(test.pending(&run(key)).await.is_empty(), "{key}");
    }
    assert_eq!(state(&test, "done").await.status, FlowStatus::Done);
    assert_eq!(state(&test, "earlier").await.status, FlowStatus::Cancelled);
    assert_eq!(probe.ids(), ["a"], "no cleanup ran");
}

/// Scenario: Stale continuation and repeated Cancel, of a stale continuation. The main step fails for good, the run
/// is cancelled, and its first cleanup attempt fails for good too, so the run stays cancelling. Then a main `Continue`
/// reaches it: one delayed since before the `Cancel`, an operator's retry of the failed main continuation, or its
/// redrive by the dead letters. Each only normalizes, and the fresh continuation runs the cleanup, never main code.
#[tokio::test(start_paused = true)]
async fn a_stale_retried_or_redriven_main_continue_reaching_a_cancelling_run_runs_only_the_cleanup() {
    for how in ["delayed", "retried", "redriven"] {
        let probe = Probe::default();
        let flow = ScriptFlow { probe: probe.clone() };
        let test = TestActors::builder()
            .with(move |node| node.host(FlowActor::new(flow)).host(DeadLettersActor::default()))
            .build()
            .await
            .unwrap();
        test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
        let letters = addr(&DeadLetters::default());
        test.system().watch(&cicada::WatchTarget::parse("test.flow").unwrap(), &letters).await.unwrap();
        probe.plan("x", &[Act::Refuse]);
        probe.plan("c", &[Act::Refuse]);
        start(&test, "k", finalized(steps(&["x", "y"]), steps(&["c"]))).await;
        if how == "delayed" {
            test.send_with(&run("k"), FlowMsg::Continue, SendOptions::new().delay(MINUTE)).await;
        }
        test.run_until_idle().await;
        let failed = state(&test, "k").await.armed.unwrap();
        assert_eq!(outcome(&test, &failed).await.unwrap().status, OutcomeStatus::Failed, "{how}");
        test.send(&run("k"), FlowMsg::Cancel).await;
        test.run_until_idle().await;
        let stalled = state(&test, "k").await;
        assert_eq!(stalled.status, FlowStatus::Cancelling, "{how}: a permanent cleanup failure stays cancelling");
        assert_eq!(outcome(&test, stalled.armed.as_ref().unwrap()).await.unwrap().status, OutcomeStatus::Failed);
        assert_eq!(probe.ids(), ["x", "c"], "{how}");

        match how {
            "delayed" => drop(test.advance_time_by(MINUTE).await),
            "retried" => {
                let RetryResult::Accepted(_) = test.system().retry_message(&failed).await.unwrap() else {
                    panic!("retried")
                };
            }
            _ => drop(test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: failed.clone() }).await),
        }
        test.run_until_idle().await;
        let held = state(&test, "k").await;
        assert_eq!((held.status, held.armed.clone()), (FlowStatus::Cancelled, None), "{how}");
        assert_eq!(probe.ids(), ["x", "c", "c"], "{how}: the cleanup ran again, and y never did");
        assert_eq!(held.journal, [], "{how}: the main journal stays empty");
    }
}

/// Scenario: Stale continuation and repeated Cancel, of a repeated `Cancel`. While the cleanup's continuation waits
/// out a retry's backoff, `Cancel`s change nothing: no state write, the same armed continuation with its failure
/// count, the saved cleanup entry kept. The retry then runs the cleanup on to cancelled.
#[tokio::test(start_paused = true)]
async fn repeated_cancels_during_the_cleanup_reset_nothing_and_do_not_starve_it() {
    let probe = Probe::default();
    let test = node(&probe).await;
    probe.plan("c", &[Act::Fail]);
    start(&test, "k", finalized(vec![sleep("nap", MINUTE)], steps(&["b", "c"]))).await;
    test.run_until_idle().await;
    test.send(&run("k"), FlowMsg::Cancel).await;
    test.run_until_idle().await;
    let retry = armed(&test, "k").await;
    assert_eq!((retry.failures, cleanup_journal(&test, "k").await), (1, vec!["b".to_owned()]), "c in backoff");
    let before = unchanged(&test, "k").await;
    for _ in 0..2 {
        let again = test.send(&run("k"), FlowMsg::Cancel).await;
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
        assert_eq!(outcome(&test, &again.id).await.unwrap().status, OutcomeStatus::Done);
    }
    assert_eq!(unchanged(&test, "k").await, before, "no state or version change");
    let still = armed(&test, "k").await;
    assert_eq!((still.id, still.failures), (retry.id, 1), "the same continuation, attempts kept");
    test.advance_time_by(BACKOFF).await;
    assert_eq!(state(&test, "k").await.status, FlowStatus::Cancelled);
    assert_eq!(cleanup_journal(&test, "k").await, ["b", "c"]);
    assert_eq!(probe.ids(), ["b", "c", "c"]);
}

/// Scenario: Separate namespaces, of addresses. A cleanup request's route names the run's own address, not its key
/// prefix, and the reply closes the cleanup wait; a cleanup send to the run itself is refused for good.
#[tokio::test(start_paused = true)]
async fn a_cleanup_request_routes_replies_to_the_run_and_a_cleanup_send_to_the_run_itself_is_refused() {
    let probe = Probe::default();
    let test = node(&probe).await;
    responders(&test, &["r1"]).await;
    let cleanup = vec![later("stop", "r1"), wait_op("stopped", "stop")];
    start(&test, "k", finalized(vec![sleep("nap", MINUTE)], cleanup)).await;
    test.run_until_idle().await;
    test.send(&run("k"), FlowMsg::Cancel).await;
    test.run_until_idle().await;
    let route = responder_state(&test, "r1").await.routes[0].clone();
    assert_eq!(route.run, addr(&run("k")), "the run's address");
    assert_eq!(state(&test, "k").await.armed, None, "parked on the cleanup wait");
    answer(&test, "r1", json!("gone"), None).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, "k").await.status, FlowStatus::Cancelled);
    assert_eq!(probe.got(), [("stopped".to_owned(), json!("gone"))]);

    let cleanup = vec![Op::SendRun { id: "poke".into(), key: "self".into() }];
    start(&test, "self", finalized(vec![sleep("nap", MINUTE)], cleanup)).await;
    test.run_until_idle().await;
    test.send(&run("self"), FlowMsg::Cancel).await;
    test.run_until_idle().await;
    refused(&test, "self", "send `poke` names the run itself, test.flow/self").await;
    assert_eq!(state(&test, "self").await.status, FlowStatus::Cancelling);
}

/// Scenario: Completion discipline. A cleanup that returns `Ok` before reaching a saved cleanup entry, or that calls
/// another entry where one is saved, fails for good and stays cancelling; a new cleanup entry after 1000 fails for good
/// without changing the main journal.
#[tokio::test(start_paused = true)]
async fn an_early_return_a_mismatch_or_entry_1001_of_the_cleanup_fails_for_good_and_stays_cancelling() {
    let probe = Probe::default();
    let test = node(&probe).await;
    let main = vec![saved("m", json!("r-m"))];
    let z = vec![saved("z", json!("r-z"))];
    let full: Vec<JournalEntry> = (0..1000).map(|i| saved(&format!("s{i}"), json!(format!("r-s{i}")))).collect();
    let thousand_and_one = vec![Op::Repeat { prefix: "s".into(), count: 1001 }];
    for (key, cleanup, saved_cleanup, why) in [
        ("early", vec![], z.clone(), "the cleanup returned before reaching its saved step `z`"),
        ("mismatch", steps(&["a"]), z, "calls step `a` where its journal holds step `z`"),
        ("full", thousand_and_one, full, "step `s1000` would be entry 1001 of a journal that holds at most 1000"),
    ] {
        let held = cancelling(finalized(steps(&["m"]), cleanup), main.clone(), saved_cleanup);
        start_with(&test, key, held).await;
        let before = main_json(&test, key).await;
        test.run_until_idle().await;
        refused(&test, key, why).await;
        assert_eq!(state(&test, key).await.status, FlowStatus::Cancelling, "{key}");
        assert_eq!(main_json(&test, key).await, before, "{key}: the main journal stays");
    }
    assert!(probe.ids().is_empty(), "no closure ran");
}
