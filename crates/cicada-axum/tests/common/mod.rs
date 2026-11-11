//! Actor types, nodes and HTTP helpers shared by the API tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use cicada::prelude::*;
use cicada::runtime::{ActorSystemBuilder, NodeConfig};
use cicada::store::*;
use cicada_test::{TokioClock, default_start};
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
    /// Creates the counters `ids` as children of this one.
    Spawn {
        ids: Vec<String>,
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
            CounterMsg::Spawn { ids } => {
                for id in ids {
                    turn.create_child(&Counter { id }, &CounterState::default())?;
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

/// A type that keeps no data.
pub struct PingActor;

impl Actor for PingActor {
    type Ref = Ping;
    type State = NoState;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: PingMsg) -> HandlerResult {
        Ok(())
    }
}

/// type `maintenance.cleanup`, key `_`: a singleton that keeps no data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cleanup;

impl ActorRef for Cleanup {
    const TYPE: &'static str = "maintenance.cleanup";
    type Message = CleanupMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CleanupMsg {
    Run,
}

pub struct CleanupActor;

impl Actor for CleanupActor {
    type Ref = Cleanup;
    type State = NoState;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: CleanupMsg) -> HandlerResult {
        Ok(())
    }
}

/// type `tickets`: a ref with a reader of its own, whose name is a number.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Ticket(pub u32);

impl From<Ticket> for String {
    fn from(t: Ticket) -> Self {
        t.0.to_string()
    }
}

impl TryFrom<String> for Ticket {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        name.parse().map(Ticket).map_err(|_| format!("'{name}' is not a number"))
    }
}

impl ActorRef for Ticket {
    const TYPE: &'static str = "tickets";
    type Message = PingMsg;
}

pub struct TicketActor;

impl Actor for TicketActor {
    type Ref = Ticket;
    type State = NoState;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: PingMsg) -> HandlerResult {
        Ok(())
    }
}

/// type `watcher`: takes the notifications of watches and notes them; refuses them when its state says so.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watcher {
    pub id: String,
}

impl ActorRef for Watcher {
    const TYPE: &'static str = "watcher";
    type Message = WatcherMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum WatcherMsg {
    Failed(cicada::Failed),
    Stopped(cicada::Stopped),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Seen {
    #[serde(default)]
    pub failed: Vec<cicada::Failed>,
    #[serde(default)]
    pub stopped: Vec<cicada::Stopped>,
    /// Fails on every notification.
    #[serde(default)]
    pub refuse: bool,
}

pub struct WatcherActor;

impl Actor for WatcherActor {
    type Ref = Watcher;
    type State = Seen;

    async fn handle(&self, turn: &mut Turn<Self>, message: WatcherMsg) -> HandlerResult {
        if turn.state().refuse {
            return Err(turn.fail("refused"));
        }
        match message {
            WatcherMsg::Failed(failed) => turn.state_mut().failed.push(failed),
            WatcherMsg::Stopped(stopped) => turn.state_mut().stopped.push(stopped),
        }
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
pub fn memory_store() -> Store {
    Store::memory_on(Arc::new(TokioClock::new(default_start())))
}

/// A started node hosting the counter and ping types, polling every 50 ms.
pub async fn node(store: Store) -> ActorSystem {
    node_with(store, |b| b.host(CounterActor).host(PingActor)).await
}

/// The node settings of these tests: polling every `poll_interval`, the rest as by default.
pub fn polling(poll_interval: Duration) -> NodeConfig {
    NodeConfig { poll_interval, ..NodeConfig::default() }
}

/// A started node configured by `configure`, polling every 50 ms unless `configure` sets another config.
pub async fn node_with(store: Store, configure: impl FnOnce(ActorSystemBuilder) -> ActorSystemBuilder) -> ActorSystem {
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
