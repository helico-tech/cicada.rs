//! Actor types, a flaky store and HTTP helpers shared by the API tests.
#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use cicada::address::{ActorAddress, TypeName};
use cicada::prelude::*;
use cicada::runtime::{ActorSystemBuilder, NodeConfig};
use cicada::store::*;
use cicada::time::Timestamp;
use cicada_test::{InMemoryStore, TokioClock, default_start};
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;
use tower::ServiceExt;

// ---- actor types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counter {
    pub id: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CounterMsg {
    Add {
        n: i32,
    },
    /// Waits until the gate named `gate` is opened by the test.
    Block {
        gate: String,
    },
    Fail {
        reason: String,
    },
    /// Sends `Add(n)` to each counter in `to`.
    Forward {
        to: Vec<String>,
        n: i32,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CounterState {
    #[serde(default)]
    pub total: i32,
}

pub struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;
    type Message = CounterMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        match message {
            CounterMsg::Add { n } => turn.state_mut().total += n,
            CounterMsg::Block { gate } => gates::wait(&gate).await,
            CounterMsg::Fail { reason } => return Err(HandlerError::msg(reason)),
            CounterMsg::Forward { to, n } => {
                for id in to {
                    turn.send(&Counter { id }, CounterMsg::Add { n })?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ping {
    pub name: String,
}

impl ActorRef for Ping {
    const TYPE: &'static str = "ping";
    type Message = PingMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum PingMsg {
    Hello,
}

/// A stateless type.
pub struct PingActor;

impl Actor for PingActor {
    type Ref = Ping;
    type State = NoState;
    type Message = PingMsg;
    const UNHANDLED: Unhandled = Unhandled::Drop;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: PingMsg) -> HandlerResult {
        Ok(())
    }
}

/// Named latches that `Block` handlers wait on.
pub mod gates {
    use super::*;

    fn gate(name: &str) -> Arc<watch::Sender<bool>> {
        static GATES: OnceLock<Mutex<HashMap<String, Arc<watch::Sender<bool>>>>> = OnceLock::new();
        let mut gates = GATES.get_or_init(Default::default).lock().unwrap();
        gates.entry(name.to_owned()).or_insert_with(|| Arc::new(watch::channel(false).0)).clone()
    }

    pub async fn wait(name: &str) {
        let mut open = gate(name).subscribe();
        let _ = open.wait_for(|o| *o).await;
    }

    pub fn open(name: &str) {
        gate(name).send_replace(true);
    }
}

pub fn unique() -> String {
    MessageId::random().to_string()
}

// ---- nodes ----

/// An in-memory store on tokio's (paused) clock.
pub fn memory_store() -> Arc<InMemoryStore> {
    Arc::new(InMemoryStore::with_clock(Arc::new(TokioClock::new(default_start()))))
}

/// A started node hosting the counter and ping types, polling every 50 ms.
pub async fn node(store: Arc<dyn Store>) -> ActorSystem {
    node_with(store, |b| b.host(CounterActor).host(PingActor)).await
}

/// The node settings of these tests: polling every `poll_interval`, the rest as by default.
pub fn polling(poll_interval: Duration) -> NodeConfig {
    NodeConfig { poll_interval, ..NodeConfig::default() }
}

/// A started node configured by `configure`, polling every 50 ms unless `configure` sets another config.
pub async fn node_with(
    store: Arc<dyn Store>,
    configure: impl FnOnce(ActorSystemBuilder) -> ActorSystemBuilder,
) -> ActorSystem {
    let system = configure(ActorSystem::builder(store).config(polling(Duration::from_millis(50)))).build();
    system.start().await.expect("the node starts");
    system
}

/// Polls `condition` every 20 ms (virtual or real time) until it holds, for at most 10 s.
pub async fn eventually(mut condition: impl AsyncFnMut() -> bool) {
    for _ in 0..500 {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Condition not met within 10 s");
}

// ---- a store that fails on demand ----

/// How [`FlakyStore::get`] fails.
#[derive(Debug, Clone, Copy)]
pub enum Failure {
    Unavailable,
    Corrupt,
    Panic,
}

/// A store whose clock, resolution and enqueue fail while [`Self::set_down`] is set, and whose `get` fails as
/// [`Self::fail_get`] says.
pub struct FlakyStore {
    inner: InMemoryStore,
    down: AtomicBool,
    get_failure: Mutex<Option<Failure>>,
}

impl FlakyStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryStore::with_clock(Arc::new(TokioClock::new(default_start()))),
            down: AtomicBool::new(false),
            get_failure: Mutex::new(None),
        })
    }

    pub fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    pub fn fail_get(&self, failure: Option<Failure>) {
        *self.get_failure.lock().unwrap() = failure;
    }

    fn check(&self) -> StoreResult<()> {
        if self.down.load(Ordering::SeqCst) {
            return Err(StoreError::unavailable("store down"));
        }
        Ok(())
    }
}

#[async_trait]
impl Store for FlakyStore {
    async fn now(&self) -> StoreResult<Timestamp> {
        self.check()?;
        self.inner.now().await
    }
    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult> {
        self.inner.register(def).await
    }
    async fn remove_type(&self, name: &TypeName) -> StoreResult<RemoveTypeResult> {
        self.inner.remove_type(name).await
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        self.check()?;
        self.inner.list_types().await
    }
    async fn create(&self, address: &ActorAddress, state: Option<Value>) -> StoreResult<CreateResult> {
        self.inner.create(address, state).await
    }
    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        let failure = *self.get_failure.lock().unwrap();
        match failure {
            None => self.inner.get(address).await,
            Some(Failure::Unavailable) => Err(StoreError::unavailable("connection refused")),
            Some(Failure::Corrupt) => Err(StoreError::Corrupt("stored state is corrupt".into())),
            Some(Failure::Panic) => panic!("a bug"),
        }
    }
    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        self.inner.list(prefix, cursor, limit).await
    }
    async fn delete(&self, address: &ActorAddress) -> StoreResult<DeleteResult> {
        self.inner.delete(address).await
    }
    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult> {
        self.check()?;
        self.inner.enqueue(message).await
    }
    async fn claim_due(
        &self,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        self.inner.claim_due(types, node, ttl, limit).await
    }
    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult> {
        self.inner.renew(address, epoch, ttl).await
    }
    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        self.inner.release(address, epoch).await
    }
    async fn next_due_at(&self, types: &BTreeSet<TypeName>) -> StoreResult<Option<Timestamp>> {
        self.inner.next_due_at(types).await
    }
    async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        self.inner.pending(address, cursor, limit).await
    }
    async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        self.inner.list_pending(prefix, cursor, limit).await
    }
    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        self.inner.cancel(id).await
    }
    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        self.inner.status(id).await
    }
    async fn outcomes(
        &self,
        status: Option<OutcomeStatus>,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>> {
        self.inner.outcomes(status, prefix, since, cursor, limit).await
    }
    async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<MessageStatus>> {
        self.inner.caused_by(id, cursor, limit).await
    }

    async fn stats(&self, prefix: &str) -> StoreResult<StoreStats> {
        self.inner.stats(prefix).await
    }
    async fn sweep_expired(&self) -> StoreResult<u64> {
        self.inner.sweep_expired().await
    }
    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult> {
        self.inner.commit(commit).await
    }

    async fn deliver_now(&self, id: &MessageId) -> StoreResult<DeliverNowResult> {
        self.inner.deliver_now(id).await
    }

    async fn set_paused(&self, address: &ActorAddress, paused: bool) -> StoreResult<bool> {
        self.inner.set_paused(address, paused).await
    }

    async fn purge(&self, address: &ActorAddress) -> StoreResult<Option<u64>> {
        self.inner.purge(address).await
    }

    async fn heartbeat_node(&self, id: &NodeId, hosted: &BTreeSet<TypeName>) -> StoreResult<()> {
        self.inner.heartbeat_node(id, hosted).await
    }

    async fn list_nodes(&self) -> StoreResult<Vec<NodeRecord>> {
        self.inner.list_nodes().await
    }

    async fn remove_node(&self, id: &NodeId) -> StoreResult<()> {
        self.inner.remove_node(id).await
    }

    async fn running_turns(
        &self,
        node: Option<&NodeId>,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<RunningTurn>> {
        self.inner.running_turns(node, prefix, cursor, limit).await
    }

    async fn running_turn(&self, address: &ActorAddress) -> StoreResult<Option<RunningTurn>> {
        self.inner.running_turn(address).await
    }

    async fn request_cancel(&self, address: &ActorAddress) -> StoreResult<bool> {
        self.inner.request_cancel(address).await
    }
    fn wake_ups(&self, types: &BTreeSet<TypeName>) -> Option<BoxStream<'static, ()>> {
        self.inner.wake_ups(types)
    }
}

// ---- HTTP ----

/// A response with its body read.
#[derive(Debug)]
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub text: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text))
    }

    pub fn content_type(&self) -> &str {
        self.headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("")
    }
}

pub async fn request(app: &Router, req: Request<Body>) -> Reply {
    let response = app.clone().oneshot(req).await.expect("the router is infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body is readable");
    Reply { status, headers, text: String::from_utf8(bytes.to_vec()).expect("UTF-8") }
}

/// A request with a JSON body, or without body (and content type) when `body` is empty.
pub async fn call(app: &Router, method: Method, uri: &str, body: &str) -> Reply {
    let mut builder = Request::builder().method(method).uri(uri);
    if !body.is_empty() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    request(app, builder.body(Body::from(body.to_owned())).unwrap()).await
}

pub async fn get(app: &Router, uri: &str) -> Reply {
    call(app, Method::GET, uri, "").await
}

pub async fn get_json(app: &Router, uri: &str) -> Value {
    let reply = get(app, uri).await;
    assert_eq!(reply.status, StatusCode::OK, "{uri}: {}", reply.text);
    reply.json()
}

pub async fn put(app: &Router, uri: &str, body: &str) -> Reply {
    call(app, Method::PUT, uri, body).await
}

pub async fn post(app: &Router, uri: &str, body: &str) -> Reply {
    call(app, Method::POST, uri, body).await
}

pub async fn delete(app: &Router, uri: &str) -> Reply {
    call(app, Method::DELETE, uri, "").await
}

/// Sends `body` and returns the accepted message's id.
pub async fn send_id(app: &Router, uri: &str, body: &str) -> String {
    let reply = post(app, uri, body).await;
    assert_eq!(reply.status, StatusCode::ACCEPTED, "{}", reply.text);
    reply.json()["id"].as_str().unwrap().to_owned()
}

/// Asserts a problem response of `status` and `problem_type`, with every member of RFC 9457 present.
pub fn assert_problem(reply: &Reply, status: u16, problem_type: &str) {
    assert_eq!(reply.status.as_u16(), status, "{}", reply.text);
    assert_eq!(reply.content_type(), cicada_axum::PROBLEM_JSON);
    let body = reply.json();
    assert_eq!(body["type"], problem_type, "{}", reply.text);
    assert_eq!(body["status"], status);
    assert!(!body["title"].as_str().unwrap().trim().is_empty());
    assert!(body.get("detail").is_some());
    assert!(body["instance"].as_str().unwrap().starts_with('/'));
}

/// Percent-encodes a query parameter value.
pub fn enc(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn items(page: &Value) -> &Vec<Value> {
    page["items"].as_array().expect("a page")
}
