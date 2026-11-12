//! Values deeper than the limit that were stored before it (value-depth, ADR 0029). Every write is checked from this
//! release on, so these tests write such values through the private backend, past the checks: a node delivers and
//! decodes them as before, a turn that writes such a state back fails, and an operator's retry of such a payload is
//! refused. The limit itself is tested through the public API in `crates/cicada/tests/value_depth.rs`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::common::*;
use crate::prelude::*;
use crate::runtime::limits::MAX_DEPTH;
use crate::runtime::{BulkSelection, RetryResult};
use crate::store::{Backend, CreateResult, Message, MessageStatus, OutcomeStatus, OutgoingMessage};
use crate::testing::TestActors;
use crate::{Failed, FailureCause, Stopped, WatchTarget};

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
    /// Keeps how deep `v` is as the state, `{"depth": n}`.
    Measure { v: Value },
    /// Leaves the state as it is.
    Look,
    /// Writes the state back as it is.
    Touch,
    /// Fails.
    Fail { v: Value },
}

struct DeepActor;

impl Actor for DeepActor {
    type Ref = Deep;
    type State = Value;

    async fn handle(&self, turn: &mut Turn<Self>, message: DeepMsg) -> HandlerResult {
        match message {
            DeepMsg::Measure { v } => turn.set_state(json!({"depth": depth(&v)})),
            DeepMsg::Look => {}
            DeepMsg::Touch => {
                turn.state_mut();
            }
            DeepMsg::Fail { .. } => return Err(HandlerError::msg("declined")),
        }
        Ok(())
    }
}

/// `witness/_`: watches every type and keeps the `Failed` it is told.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Witness;

impl ActorRef for Witness {
    const TYPE: &'static str = "witness";
    type Message = WitnessMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
enum WitnessMsg {
    Failed(Failed),
    Stopped(Stopped),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Told {
    failed: Vec<Failed>,
}

struct WitnessActor;

impl Actor for WitnessActor {
    type Ref = Witness;
    type State = Told;

    async fn handle(&self, turn: &mut Turn<Self>, message: WitnessMsg) -> HandlerResult {
        if let WitnessMsg::Failed(failed) = message {
            turn.state_mut().failed.push(failed);
        }
        Ok(())
    }
}

/// A chain of arrays `levels` deep around a number, built without recursion.
fn nested(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, _| Value::Array(vec![inner]))
}

/// How deep a chain of [`nested`] is, counted without recursion.
fn depth(mut value: &Value) -> usize {
    let mut levels = 0;
    while let Some(inner) = value.as_array().and_then(|a| a.first()) {
        (levels, value) = (levels + 1, inner);
    }
    levels
}

/// The payload of a message whose field `v` is `levels - 1` deep: `levels` deep.
fn payload(levels: usize) -> Value {
    Value::Object(Map::from_iter([("v".to_owned(), nested(levels - 1))]))
}

/// A test with `deep/a`, whose state is `{}`, and the witness, which watches every type.
async fn test() -> TestActors {
    let test = TestActors::builder().with(|node| node.host(DeepActor).host(WitnessActor)).build().await.unwrap();
    test.create(&Witness, &Told::default()).await;
    test.system().watch(&WatchTarget::Every, &addr(&Witness)).await.unwrap();
    test.create(&deep("a"), &json!({})).await;
    test
}

/// Writes a message of `message_type` with `payload` to `deep/a` through the backend, as a node of an earlier release
/// did.
async fn from_before(test: &TestActors, message_type: &str, payload: Value) -> Message {
    enqueue(test.store(), OutgoingMessage::new(addr(&deep("a")), message_type, payload)).await
}

/// The cause and the error of the `Failed` the witness was told of `message`.
async fn told(test: &TestActors, message: &Message) -> (FailureCause, String) {
    let told: Told = test.state(&Witness).await;
    let failed = told.failed.iter().find(|f| f.message_id == message.id).expect("the witness was told");
    (failed.cause, failed.error.clone())
}

/// The messages that a retry of `message` sent.
async fn retries_of(test: &TestActors, message: &Message) -> usize {
    test.system().caused_by(&message.id, None, 10).await.unwrap().unwrap().items.len()
}

/// Scenario: A deep message from before. A node delivers a pending message 100 levels deep, and the handler gets it.
#[tokio::test(start_paused = true)]
async fn a_deep_message_from_before_is_delivered_and_decoded() {
    let test = test().await;
    let m = from_before(&test, "Measure", payload(100)).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state_json(&deep("a")).await, Some(json!({"depth": 99})), "the handler got all of it");
}

/// Scenarios: A deep state from before, left as it is; … written back. A turn that leaves a state of 100 levels as it is
/// ends `done`; one that writes it back ends `failed` with the cause `handler`, and the stored state is as it was.
#[tokio::test(start_paused = true)]
async fn a_deep_state_from_before_is_read_as_before_and_cannot_be_written_back() {
    let test = test().await;
    let store: &dyn Backend = test.store();
    let old = deep("old").address().unwrap();
    assert_eq!(store.create(&old, nested(100)).await.unwrap(), CreateResult::Created);

    let looked = test.send(&deep("old"), DeepMsg::Look).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&looked).await.unwrap().status, OutcomeStatus::Done);

    let touched = test.send(&deep("old"), DeepMsg::Touch).await;
    test.run_until_idle().await;
    let error = format!("State cannot be stored: The state is nested deeper than {MAX_DEPTH} levels");
    let outcome = test.outcome(&touched).await.unwrap();
    assert_eq!((outcome.status, outcome.error.as_deref()), (OutcomeStatus::Failed, Some(error.as_str())));
    assert_eq!(told(&test, &touched).await, (FailureCause::Handler, error));
    let record = store.get(&old).await.unwrap().unwrap();
    assert_eq!((depth(&record.state), record.version), (100, 0), "the stored state is as it was");
}

/// Scenario: A retry of a deep message from before. A failed message with a payload 100 levels deep is refused with
/// `RetryResult::TooDeep`, and no message is written.
#[tokio::test(start_paused = true)]
async fn a_retry_of_a_deep_message_from_before_is_refused_and_writes_nothing() {
    let test = test().await;
    let m = from_before(&test, "Fail", payload(100)).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Failed);

    let retried = test.system().retry_message(&m.id).await.unwrap();
    assert_eq!(retried, RetryResult::TooDeep("The payload is nested deeper than 64 levels".to_owned()));
    assert_eq!(retries_of(&test, &m).await, 0);
    assert!(test.pending(&deep("a")).await.is_empty(), "no message is written");
    assert!(matches!(test.system().status(&m.id).await.unwrap(), Some(MessageStatus::Finished(_))));
}

/// Scenario: A bulk retry. Of the deep message and two failed messages within the limit, by ids the two are retried and
/// counted, and the deep one is not; by a filter the deep one is skipped again, and the batch goes on.
#[tokio::test(start_paused = true)]
async fn a_bulk_retry_skips_a_deep_message_and_retries_the_others() {
    let test = test().await;
    let too_deep = from_before(&test, "Fail", payload(100)).await;
    let at_limit = test.send(&deep("a"), DeepMsg::Fail { v: nested(MAX_DEPTH - 1) }).await;
    let shallow = test.send(&deep("a"), DeepMsg::Fail { v: json!(1) }).await;
    test.run_until_idle().await;
    for m in [&too_deep, &at_limit, &shallow] {
        assert_eq!(test.outcome(m).await.unwrap().status, OutcomeStatus::Failed);
    }

    let ids = vec![too_deep.id.clone(), at_limit.id.clone(), shallow.id.clone()];
    let result = test.system().bulk_retry(BulkSelection::Ids(ids)).await.unwrap();
    assert_eq!((result.affected, result.more), (2, false));
    let retried =
        [retries_of(&test, &too_deep).await, retries_of(&test, &at_limit).await, retries_of(&test, &shallow).await];
    assert_eq!(retried, [0, 1, 1]);

    let filter = BulkSelection::Filter { prefix: "deep/".to_owned(), message_type: None };
    let result = test.system().bulk_retry(filter).await.unwrap();
    assert_eq!((result.affected, result.more), (0, false), "the two are retried already, the deep one is refused");
    assert_eq!(retries_of(&test, &too_deep).await, 0);
}
