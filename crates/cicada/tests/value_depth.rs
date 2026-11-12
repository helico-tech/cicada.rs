//! The depth of a payload and a state (value-depth, ADR 0029), on virtual time with `TestActors`: a value deeper than
//! the limit is refused from outside a turn, at the call in a turn, and as the new state of a turn. Public API only; the
//! retry of an operator and the values stored before are tested in `crates/cicada/src/tests/runtime/value_depth.rs`.

use cicada::prelude::*;
use cicada::runtime::limits::MAX_DEPTH;
use cicada::runtime::{CreateActorResult, OpError, SendResult};
use cicada::store::OutcomeStatus;
use cicada::testing::TestActors;
use cicada::turn::TurnError;
use cicada::{Failed, FailureCause, Stopped, WatchTarget};
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
    /// Sets the state to a value `levels` deep.
    Grow { levels: usize },
    /// Sends a payload and creates `deep/made` with a state, each `levels` deep, by a typed ref and as raw JSON, and keeps
    /// how each call ended as the state.
    Try { levels: usize },
}

struct DeepActor;

impl Actor for DeepActor {
    type Ref = Deep;
    type State = Value;

    async fn handle(&self, turn: &mut Turn<Self>, message: DeepMsg) -> HandlerResult {
        match message {
            DeepMsg::Keep { v } => turn.set_state(v),
            DeepMsg::Grow { levels } => turn.set_state(nested(levels)),
            DeepMsg::Try { levels } => {
                let (me, at) = (turn.self_ref().clone(), turn.self_address().clone());
                let made = deep("made");
                let made_at = made.address()?;
                // `{"v": …}` around a value one level less: a payload `levels` deep.
                let send = ended(turn.send(&me, DeepMsg::Keep { v: nested(levels - 1) }));
                let send_raw = ended(turn.send_raw(&at, "Keep", keep(nested(levels - 1)), SendOptions::new()).await);
                let create = ended(turn.create(&made, &nested(levels)));
                let create_raw = ended(turn.create_raw(&made_at, nested(levels)).await);
                turn.set_state(json!({"send": send, "send_raw": send_raw, "create": create, "create_raw": create_raw}));
            }
        }
        Ok(())
    }
}

/// How a call of a turn ended: `ok`, `invalid: <why>`, or another error.
fn ended<T>(result: Result<T, TurnError>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(TurnError::Invalid(why)) => format!("invalid: {why}"),
        Err(other) => format!("other: {other}"),
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

/// A value `levels` deep: arrays and objects in turn around a number, built without recursion.
fn nested(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, level| if level % 2 == 0 { json!([inner]) } else { keep(inner) })
}

/// `{"v": inner}`, one level deeper, without copying `inner`.
fn keep(inner: Value) -> Value {
    Value::Object(Map::from_iter([("v".to_owned(), inner)]))
}

/// A test with `deep/a`, whose state is `{}`, and the witness, which watches every type.
async fn test() -> TestActors {
    let test = TestActors::builder().with(|node| node.host(DeepActor).host(WitnessActor)).build().await.unwrap();
    test.create(&Witness, &Told::default()).await;
    test.system().watch(&WatchTarget::Every, &Witness.address().unwrap()).await.unwrap();
    test.create(&deep("a"), &json!({})).await;
    test
}

fn invalid<T: std::fmt::Debug>(result: Result<T, OpError>) -> String {
    match result {
        Err(OpError::Invalid(why)) => why,
        other => panic!("expected OpError::Invalid, got {other:?}"),
    }
}

const TOO_DEEP_PAYLOAD: &str = "The payload is nested deeper than 64 levels";
const TOO_DEEP_STATE: &str = "The state is nested deeper than 64 levels";

/// Scenario of the limit, from outside a turn: a payload and a state of 64 levels are taken, of 65 refused with
/// `OpError::Invalid`, by an address and by a typed ref, and nothing is stored.
#[tokio::test(start_paused = true)]
async fn a_send_and_a_create_from_outside_a_turn_are_refused_beyond_64_levels() {
    let test = test().await;
    let system = test.system();
    let a = deep("a").address().unwrap();
    let sent = system.send_message(&a, "Keep", keep(nested(MAX_DEPTH)), SendOptions::new()).await;
    assert_eq!(invalid(sent), TOO_DEEP_PAYLOAD);
    assert_eq!(invalid(system.send(&deep("a"), DeepMsg::Keep { v: nested(MAX_DEPTH) }).await), TOO_DEEP_PAYLOAD);
    let b = deep("b").address().unwrap();
    assert_eq!(invalid(system.create_actor(&b, nested(MAX_DEPTH + 1)).await), TOO_DEEP_STATE);
    assert_eq!(invalid(system.create(&deep("b"), &nested(MAX_DEPTH + 1)).await), TOO_DEEP_STATE);
    assert!(test.pending(&deep("a")).await.is_empty() && !test.exists(&deep("b")).await, "nothing is stored");

    let sent = system.send_message(&a, "Keep", keep(nested(MAX_DEPTH - 1)), SendOptions::new()).await.unwrap();
    assert!(matches!(sent, SendResult::Accepted(_)), "{sent:?}");
    assert_eq!(system.create_actor(&b, nested(MAX_DEPTH)).await.unwrap(), CreateActorResult::Created);
    test.run_until_idle().await;
    assert_eq!(test.state_json(&deep("a")).await, Some(nested(MAX_DEPTH - 1)), "handled");
    assert_eq!(test.state_json(&deep("b")).await, Some(nested(MAX_DEPTH)));
}

/// Scenario: A send and a create in a turn. Each call with a value of 65 levels returns `TurnError::Invalid`, by a typed
/// ref and as raw JSON, and the turn goes on; with 64 levels each call is taken.
#[tokio::test(start_paused = true)]
async fn a_send_and_a_create_in_a_turn_are_refused_at_the_call_beyond_64_levels() {
    let test = test().await;
    let m = test.send(&deep("a"), DeepMsg::Try { levels: MAX_DEPTH + 1 }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done, "the handler handled the errors");
    let payload = format!("invalid: {TOO_DEEP_PAYLOAD}");
    let state = "invalid: The initial state is nested deeper than 64 levels";
    let expected = json!({"send": payload, "send_raw": payload, "create": state, "create_raw": state});
    assert_eq!(test.state_json(&deep("a")).await, Some(expected));
    assert!(!test.exists(&deep("made")).await);

    let m = test.send(&deep("a"), DeepMsg::Try { levels: MAX_DEPTH }).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done);
    let expected = json!({"send": "ok", "send_raw": "ok", "create": "ok", "create_raw": "ok"});
    assert_eq!(test.state_json(&deep("a")).await, Some(expected));
    assert_eq!(test.state_json(&deep("made")).await, Some(nested(MAX_DEPTH)));
    assert_eq!(test.pending(&deep("a")).await.len(), 2, "both sends");
}

/// Scenario: A new state that is too deep. The message ends `failed` with the cause `handler`, the error says the state
/// cannot be stored, and the stored state is as it was; a new state of 64 levels is stored.
#[tokio::test(start_paused = true)]
async fn a_new_state_beyond_64_levels_fails_the_turn_with_the_cause_handler() {
    let test = test().await;
    let m = test.send(&deep("a"), DeepMsg::Grow { levels: MAX_DEPTH + 1 }).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&m).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    let error = format!("State cannot be stored: {TOO_DEEP_STATE}");
    assert_eq!(outcome.error.as_deref(), Some(error.as_str()));
    let told = test.state::<_, Told>(&Witness).await.failed;
    let failed = told.iter().find(|f| f.message_id == m.id).expect("the witness was told");
    assert_eq!((failed.cause, failed.error.as_str()), (FailureCause::Handler, error.as_str()));
    assert_eq!((test.state_json(&deep("a")).await, test.version(&deep("a")).await), (Some(json!({})), 0));

    test.send(&deep("a"), DeepMsg::Grow { levels: MAX_DEPTH }).await;
    test.run_until_idle().await;
    assert_eq!(test.state_json(&deep("a")).await, Some(nested(MAX_DEPTH)));
}

/// Scenario: A deep value sent from code. 1,000 levels, which a walk that calls itself for every level would take
/// further than the check does, are refused with `OpError::Invalid`, and the node goes on serving.
#[tokio::test(start_paused = true)]
async fn a_value_1000_levels_deep_sent_from_code_is_refused_and_the_node_goes_on() {
    let test = test().await;
    let system = test.system();
    let a = deep("a").address().unwrap();
    let sent = system.send_message(&a, "Keep", nested(1_000), SendOptions::new()).await;
    assert_eq!(invalid(sent), TOO_DEEP_PAYLOAD);
    let created = system.create_actor(&deep("b").address().unwrap(), nested(1_000)).await;
    assert_eq!(invalid(created), TOO_DEEP_STATE);

    test.send(&deep("a"), DeepMsg::Keep { v: json!("still here") }).await;
    test.run_until_idle().await;
    assert_eq!(test.state_json(&deep("a")).await, Some(json!("still here")));
}
