//! A value of any depth the parser allows is checked like any other (decision 11): three types with several schema steps
//! between their levels, through the HTTP door served by `axum::serve` on a multi-thread runtime with its default worker
//! stacks, as a host runs it, on a node that hosts the types and on one that does not.

mod common;

use std::marker::PhantomData;
use std::net::SocketAddr;

use cicada::api::problem_types as pt;
use cicada::prelude::*;
use cicada::{ActorSystem, Store};
use common::shapes::{Chain, PutMsg, Shape, Stack, Wide, build, payload};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The ref of the test type of shape `T`, written as its key.
struct ShapeRef<T>(String, PhantomData<fn() -> T>);

impl<T> Clone for ShapeRef<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<T> Serialize for ShapeRef<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, T> Deserialize<'de> for ShapeRef<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(deserializer)?, PhantomData))
    }
}

impl<T: Shape> ActorRef for ShapeRef<T> {
    const TYPE: &'static str = T::NAME;
    type Message = PutMsg<T>;
}

/// The test type of shape `T`: its state is a `T`, its message carries one.
struct ShapeActor<T>(PhantomData<fn() -> T>);

impl<T: Shape> Actor for ShapeActor<T> {
    type Ref = ShapeRef<T>;
    type State = T;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: PutMsg<T>) -> HandlerResult {
        Ok(())
    }
}

/// One HTTP/1.1 request over a fresh connection; the status and the body.
async fn http(to: SocketAddr, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(to).await.unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    let text = String::from_utf8_lossy(&answer).into_owned();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, text.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default())
}

/// `{key: value}` without copying `value` by recursion, as `json!` would.
fn with(key: &str, value: Value, more: Vec<(&str, Value)>) -> String {
    let mut fields = Map::new();
    fields.insert(key.to_owned(), value);
    fields.extend(more.into_iter().map(|(k, v)| (k.to_owned(), v)));
    Value::Object(fields).to_string()
}

/// A state and a payload of shape `T` at 21, 24 and 126 levels, matching and not, on both nodes.
async fn shape<T: Shape>(host: SocketAddr, other: SocketAddr) {
    let name = T::NAME;
    let (status, body) = http(host, "PUT", &format!("/actors/{name}/a"), &with("state", T::leaf(true), vec![])).await;
    assert_eq!(status, 201, "{body}");
    for levels in [21, 24, 126] {
        for ok in [true, false] {
            for (node, to) in [("host", host), ("other", other)] {
                let case = format!("{name} at {levels} levels, matching {ok}, on {node}");
                let refused = |body: &str, problem_type: &str| {
                    let problem: Value =
                        serde_json::from_str(body).unwrap_or_else(|e| panic!("{case}: {e}: {body:.200}"));
                    assert_eq!(problem["type"], problem_type, "{case}: {body:.300}");
                };
                let state = with("state", build::<T>(levels, ok), vec![]);
                let (status, body) = http(to, "PUT", &format!("/actors/{name}/{levels}-{ok}-{node}"), &state).await;
                assert_eq!(status, if ok { 201 } else { 422 }, "{case}, a state: {body:.300}");
                if !ok {
                    refused(&body, pt::INVALID_STATE);
                }
                let send = with("payload", payload::<T>(levels, ok), vec![("type", json!("Put"))]);
                let (status, body) = http(to, "POST", &format!("/actors/{name}/a/messages"), &send).await;
                assert_eq!(status, if ok { 202 } else { 422 }, "{case}, a payload: {body:.300}");
                if !ok {
                    refused(&body, pt::INVALID_PAYLOAD);
                }
            }
        }
    }
}

/// Scenarios: A value at the deepest level the parser allows; A type with several schema steps between its levels.
/// The hosting node is initialised and not started: it checks and decodes at the door, and runs no turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_value_is_checked_on_a_stack_of_its_own_on_both_nodes_whatever_its_type_and_depth() {
    let store = Store::memory();
    let host = ActorSystem::builder(store.clone())
        .host(ShapeActor::<Chain>(PhantomData))
        .host(ShapeActor::<Stack>(PhantomData))
        .host(ShapeActor::<Wide>(PhantomData))
        .build();
    host.initialize().await.unwrap();
    let other = ActorSystem::builder(store).build();
    let mut addresses = Vec::new();
    for system in [&host, &other] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        addresses.push(listener.local_addr().unwrap());
        let app = cicada_axum::actor_api(system.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    }
    // Each request is served by a task on a worker thread of the runtime, with its default stack.
    shape::<Chain>(addresses[0], addresses[1]).await;
    shape::<Stack>(addresses[0], addresses[1]).await;
    shape::<Wide>(addresses[0], addresses[1]).await;
}
