//! Every value accepted from 0.16.0 on can be read back over HTTP (value-depth, ADR 0029): the views of a payload and a
//! state at the depth limit, also the deepest view, a page of outcomes that holds a `Failed` about such a payload, are
//! read by the client, whose JSON parser allows 128 levels; a `Worker` over the client leases and decodes the `Failed` of
//! a payload at the limit. Against a real `actor_api()` server, in real time.

mod common;

use cicada::api::MessageState;
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::limits::MAX_DEPTH;
use cicada::runtime::{Worker, WorkerConfig};
use cicada::store::OutcomeStatus;
use cicada::{Failed, Stopped, WatchTarget};
use cicada_axum::actor_api;
use cicada_client::ActorClient;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// `deep/{id}`: its state is any JSON value.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Deep {
    id: String,
}

impl ActorRef for Deep {
    const TYPE: &'static str = "deep";
    type Message = DeepMsg;
}

fn deep(id: &str) -> Deep {
    Deep { id: id.to_owned() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum DeepMsg {
    /// Keeps `v` as the state.
    Keep { v: Value },
    /// Fails.
    Fail { v: Value },
}

struct DeepActor;

impl Actor for DeepActor {
    type Ref = Deep;
    type State = Value;

    async fn handle(&self, turn: &mut Turn<Self>, message: DeepMsg) -> HandlerResult {
        match message {
            DeepMsg::Keep { v } => turn.set_state(v),
            DeepMsg::Fail { .. } => return Err(HandlerError::msg("declined")),
        }
        Ok(())
    }
}

/// `refuser/{id}`: a watcher whose handler fails every notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Refuser {
    id: String,
}

impl ActorRef for Refuser {
    const TYPE: &'static str = "refuser";
    type Message = RefuserMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
enum RefuserMsg {
    Failed(Failed),
    Stopped(Stopped),
}

struct RefuserActor;

impl Actor for RefuserActor {
    type Ref = Refuser;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: RefuserMsg) -> HandlerResult {
        Err(HandlerError::msg("refused"))
    }
}

/// `probe/_`: a typed watcher that compares the payload of each `Failed` with the one it expects and keeps only the id
/// of the failed message and whether it matched, never the payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Probe;

impl ActorRef for Probe {
    const TYPE: &'static str = "probe";
    type Message = RefuserMsg;
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Probed {
    seen: Vec<(MessageId, bool)>,
}

struct ProbeActor {
    expected: Value,
}

impl Actor for ProbeActor {
    type Ref = Probe;
    type State = Probed;

    async fn handle(&self, turn: &mut Turn<Self>, message: RefuserMsg) -> HandlerResult {
        if let RefuserMsg::Failed(failed) = message {
            turn.state_mut().seen.push((failed.message_id, failed.payload == self.expected));
        }
        Ok(())
    }
}

/// A value `levels` deep: arrays and objects in turn around a number, built without recursion.
fn nested(levels: usize) -> Value {
    (0..levels).fold(
        json!(1),
        |inner, level| if level.is_multiple_of(2) { Value::Array(vec![inner]) } else { field("n", inner) },
    )
}

/// `{key: inner}`, one level deeper, without copying `inner`.
fn field(key: &str, inner: Value) -> Value {
    Value::Object(Map::from_iter([(key.to_owned(), inner)]))
}

/// The payload `{"v": …}` of a message, `levels` deep.
fn payload(levels: usize) -> Value {
    field("v", nested(levels - 1))
}

/// A node that hosts `deep` and `refuser`, with `deep/a`, and a client for it.
async fn client() -> ActorClient {
    let system = node_with(|b| b.host(DeepActor).host(RefuserActor)).await;
    let actors = ActorClient::new(serve(actor_api(system)).await);
    actors.create(&deep("a"), &json!({})).await.unwrap();
    actors
}

/// Scenario: A message at the limit. A pending payload of 64 levels is read with `GET /messages/{id}`, and once handled
/// with the page of `GET /outcomes` that holds it.
#[tokio::test]
async fn a_message_at_the_limit_is_read_pending_and_in_the_outcomes() {
    let actors = client().await;
    let a = deep("a").address().unwrap();
    assert!(actors.pause_mailbox(&a).await.unwrap());
    let sent = actors.send_message(&a, "Keep", payload(MAX_DEPTH), SendOptions::new()).await.unwrap();
    let pending = actors.status(&sent.id).await.unwrap().expect("the message");
    assert_eq!((pending.status, &pending.message.payload), (MessageState::Pending, &payload(MAX_DEPTH)));

    assert!(actors.resume_mailbox(&a).await.unwrap());
    eventually(async || actors.status(&sent.id).await.unwrap().unwrap().status == MessageState::Done).await;
    let page = actors.outcomes(Some(OutcomeStatus::Done), "deep/", None, None, None).await.unwrap();
    let outcome = page.items.iter().find(|o| o.id == sent.id).expect("its outcome");
    assert_eq!(outcome.message.payload, payload(MAX_DEPTH));
}

/// Scenario: The deepest view. A payload of 64 levels fails, its `Failed` is 65 levels deep, and the watcher fails
/// that too: `GET /messages/{id}` of the notification wraps it two levels deeper, and the page of `GET /outcomes` that
/// holds its outcome four, 69.
#[tokio::test]
async fn the_deepest_view_a_page_of_outcomes_that_holds_a_failed_at_the_limit_is_read() {
    let actors = client().await;
    actors.create(&Refuser { id: "r".into() }, &NoState {}).await.unwrap();
    let refuser = Refuser { id: "r".into() }.address().unwrap();
    assert!(actors.watch(&WatchTarget::Actor(deep("a").address().unwrap()), &refuser).await.unwrap());
    let failed = actors.send(&deep("a"), DeepMsg::Fail { v: nested(MAX_DEPTH - 1) }).await.unwrap();

    let failures = async || actors.outcomes(Some(OutcomeStatus::Failed), "refuser/", None, None, None).await.unwrap();
    eventually(async || !failures().await.items.is_empty()).await;
    let page = failures().await;
    let [outcome] = page.items.as_slice() else { panic!("one failed notification: {page:?}") };
    assert_eq!(outcome.message.payload["messageId"], failed.id.as_str());
    assert_eq!(outcome.message.payload["payload"], payload(MAX_DEPTH));

    let notification = actors.status(&outcome.id).await.unwrap().expect("the notification");
    assert_eq!(notification.status, MessageState::Failed);
    assert_eq!(notification.message.payload["payload"], payload(MAX_DEPTH));
}

/// Scenario: An actor at the limit. A state of 64 levels is read with `GET /actors/{type}/{key}`.
#[tokio::test]
async fn an_actor_at_the_limit_is_read() {
    let actors = client().await;
    assert!(actors.create(&deep("b"), &nested(MAX_DEPTH)).await.unwrap().created);
    let view = actors.get(&deep("b")).await.unwrap().expect("the actor");
    assert_eq!(view.state, nested(MAX_DEPTH));
    assert_eq!(actors.state::<_, Value>(&deep("b")).await.unwrap(), Some(nested(MAX_DEPTH)));
}

/// Scenario: A maximum-depth current state fits the view. A state of 64 levels is read in a page of
/// `GET /actors/states`, which wraps it three levels deeper, 67.
#[tokio::test]
async fn an_actor_at_the_limit_is_read_in_a_page_of_states() {
    let actors = client().await;
    assert!(actors.create(&deep("b"), &nested(MAX_DEPTH)).await.unwrap().created);
    let page = actors.list_actors_with_state("deep/", None, None).await.unwrap();
    let states: Vec<(&str, &Value)> = page.items.iter().map(|v| (v.address.as_str(), &v.state)).collect();
    assert_eq!(states, vec![("deep/a", &json!({})), ("deep/b", &nested(MAX_DEPTH))]);
}

/// Scenario: HTTP worker watcher. The probe runs in a `Worker` over the client, not on the server: the `Failed` of a
/// payload of 64 levels, 65 deep, is leased over HTTP and decoded, the handler sees the whole payload, and the notice
/// turn ends `done`.
#[tokio::test]
async fn a_worker_over_http_leases_and_decodes_the_failed_of_a_payload_at_the_limit() {
    let system = node_with(|b| b.host(DeepActor)).await;
    let base = serve(actor_api(system.clone())).await;
    let actors = ActorClient::new(&base);
    actors.create(&deep("a"), &json!({})).await.unwrap();
    let config = WorkerConfig { poll_interval: Duration::from_millis(20), ..WorkerConfig::default() };
    let worker = Worker::builder(ActorClient::new(&base))
        .host(ProbeActor { expected: payload(MAX_DEPTH) })
        .config(config)
        .build();
    worker.start().await.expect("the worker starts over HTTP");
    actors.create(&Probe, &Probed::default()).await.unwrap();
    let probe = Probe.address().unwrap();
    assert!(actors.watch(&WatchTarget::Actor(deep("a").address().unwrap()), &probe).await.unwrap());

    let failed = actors.send(&deep("a"), DeepMsg::Fail { v: nested(MAX_DEPTH - 1) }).await.unwrap();
    let probed = async || actors.state::<_, Probed>(&Probe).await.unwrap().unwrap_or_default().seen;
    eventually(async || !probed().await.is_empty()).await;
    assert_eq!(probed().await, [(failed.id.clone(), true)], "the handler saw the whole payload");
    let page = actors.outcomes(None, "probe/", None, None, None).await.unwrap();
    let [notice] = page.items.as_slice() else { panic!("one notice: {page:?}") };
    assert_eq!((notice.status, notice.message.payload["payload"].clone()), (MessageState::Done, payload(MAX_DEPTH)));
    worker.stop().await;
}
