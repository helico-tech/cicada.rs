//! A create by address in a turn, in virtual time (ADR 0028): `Turn::create_raw` and `Turn::create_child_raw`, with a
//! node that hosts the type of the address and one that does not. Public API only.

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::testing::TestActors;
use cicada::turn::TurnError;
use cicada::{Failed, FailureCause, Stopped, WatchTarget};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// `worker/{key}`; a key that starts with `bad` is no address of the type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
struct Worker(String);

impl From<Worker> for String {
    fn from(worker: Worker) -> Self {
        worker.0
    }
}

impl TryFrom<String> for Worker {
    type Error = String;

    fn try_from(key: String) -> Result<Self, String> {
        if key.starts_with("bad") { Err(format!("{key} is a bad key")) } else { Ok(Self(key)) }
    }
}

impl ActorRef for Worker {
    const TYPE: &'static str = "worker";
    type Message = WorkerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum WorkerMsg {
    Work,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct WorkerState {
    done: u32,
}

struct WorkerActor;

impl Actor for WorkerActor {
    type Ref = Worker;
    type State = WorkerState;

    async fn handle(&self, turn: &mut Turn<Self>, _: WorkerMsg) -> HandlerResult {
        turn.state_mut().done += 1;
        Ok(())
    }
}

/// `creator/{id}`: creates actors by address and notes what each call returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Creator {
    id: String,
}

impl ActorRef for Creator {
    const TYPE: &'static str = "creator";
    type Message = CreatorMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
enum CreatorMsg {
    /// Creates the actor at `address` with `state`, as a child when `child` is set, and notes the result.
    Create {
        address: String,
        state: Value,
        child: bool,
    },
    /// Stops.
    Stop,
    Failed(Failed),
    Stopped(Stopped),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Notes {
    /// `ok`, or the kind of the error and its text.
    results: Vec<String>,
    failed: Vec<Failed>,
}

struct CreatorActor;

impl Actor for CreatorActor {
    type Ref = Creator;
    type State = Notes;

    async fn handle(&self, turn: &mut Turn<Self>, message: CreatorMsg) -> HandlerResult {
        match message {
            CreatorMsg::Create { address, state, child } => {
                let address = ActorAddress::parse(&address)?;
                let result = if child {
                    turn.create_child_raw(&address, state).await
                } else {
                    turn.create_raw(&address, state).await
                };
                let note = match result {
                    Ok(()) => "ok".to_owned(),
                    Err(TurnError::Invalid(e)) => format!("invalid: {e}"),
                    Err(e) => format!("other: {e}"),
                };
                turn.state_mut().results.push(note);
            }
            CreatorMsg::Stop => turn.stop(),
            CreatorMsg::Failed(failed) => turn.state_mut().failed.push(failed),
            CreatorMsg::Stopped(_) => {}
        }
        Ok(())
    }
}

fn creator() -> Creator {
    Creator { id: "c".into() }
}

fn w1() -> Worker {
    Worker("w1".into())
}

/// A node that hosts the creator and, when `workers` is set, the worker; with the creator `creator/c`.
async fn node(workers: bool) -> TestActors {
    let test = TestActors::builder()
        .with(move |node| {
            let node = node.host(CreatorActor);
            if workers { node.host(WorkerActor) } else { node }
        })
        .build()
        .await
        .unwrap();
    test.create(&creator(), &Notes::default()).await;
    test
}

/// Asks the creator to create `address` with `state`, runs its turn, and returns the note of the call.
async fn create(test: &TestActors, address: &str, state: Value, child: bool) -> String {
    let m = test.send(&creator(), CreatorMsg::Create { address: address.into(), state, child }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done, "the handler goes on");
    test.state::<_, Notes>(&creator()).await.results.last().cloned().unwrap()
}

/// Scenario: A create by address.
#[tokio::test(start_paused = true)]
async fn a_create_by_address_creates_the_actor_with_that_state_and_no_parent() {
    let test = node(true).await;
    assert_eq!(create(&test, "worker/w1", json!({"done": 0}), false).await, "ok");
    let snapshot = test.system().get_actor(&w1().address().unwrap()).await.unwrap().expect("created");
    assert_eq!(snapshot.state, json!({"done": 0}));
    assert_eq!(snapshot.parent, None);
}

/// Scenario: A child by address.
#[tokio::test(start_paused = true)]
async fn a_child_by_address_is_removed_with_its_parent() {
    let test = node(true).await;
    assert_eq!(create(&test, "worker/w1", json!({"done": 3}), true).await, "ok");
    let snapshot = test.system().get_actor(&w1().address().unwrap()).await.unwrap().expect("created");
    assert_eq!(snapshot.parent, Some(creator().address().unwrap()));
    assert_eq!(snapshot.state, json!({"done": 3}));
    test.send(&creator(), CreatorMsg::Stop).await;
    test.run_until_idle().await;
    assert!(!test.exists(&w1()).await, "removed with its parent");
}

/// Scenario: The actor exists.
#[tokio::test(start_paused = true)]
async fn a_create_by_address_of_an_actor_that_exists_keeps_its_state() {
    let test = node(true).await;
    test.create(&w1(), &WorkerState { done: 7 }).await;
    assert_eq!(create(&test, "worker/w1", json!({"done": 0}), false).await, "ok");
    assert_eq!(test.state::<_, WorkerState>(&w1()).await, WorkerState { done: 7 });
}

/// A child by address of an actor that exists without a parent rejects the commit, as `create_child` does.
#[tokio::test(start_paused = true)]
async fn a_child_by_address_of_an_actor_without_a_parent_rejects_the_commit() {
    let test = node(true).await;
    test.create(&w1(), &WorkerState { done: 7 }).await;
    let m = test
        .send(&creator(), CreatorMsg::Create { address: "worker/w1".into(), state: json!({"done": 0}), child: true })
        .await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Failed);
    assert_eq!(test.state::<_, WorkerState>(&w1()).await, WorkerState { done: 7 });
}

/// Scenario: A type that is not registered.
#[tokio::test(start_paused = true)]
async fn a_create_by_address_of_a_type_that_is_not_registered_is_invalid_and_the_handler_goes_on() {
    let test = node(true).await;
    let note = create(&test, "nobody/x", json!({}), false).await;
    assert!(note.starts_with("invalid: ") && note.contains("no registered actor type nobody"), "{note}");
    assert!(test.list("nobody/").await.is_empty());
}

/// Scenario: A null state.
#[tokio::test(start_paused = true)]
async fn a_create_by_address_with_a_null_state_is_invalid() {
    let test = node(true).await;
    for child in [false, true] {
        let note = create(&test, "worker/w1", Value::Null, child).await;
        assert!(note.starts_with("invalid: "), "{note}");
    }
    assert!(!test.exists(&w1()).await);
}

/// Scenario: A state that does not decode, on a hosting node.
#[tokio::test(start_paused = true)]
async fn a_hosting_node_refuses_a_state_that_does_not_decode() {
    let test = node(true).await;
    let note = create(&test, "worker/w1", json!({"done": "seven"}), false).await;
    assert!(note.starts_with("invalid: ") && note.contains("the initial state cannot be decoded"), "{note}");
    assert!(!test.exists(&w1()).await, "nothing is created");
}

/// Scenario: An address the type cannot read.
#[tokio::test(start_paused = true)]
async fn a_hosting_node_refuses_an_address_the_type_cannot_read() {
    let test = node(true).await;
    let note = create(&test, "worker/bad1", json!({"done": 0}), true).await;
    assert!(note.starts_with("invalid: ") && note.contains("bad1 is a bad key"), "{note}");
    assert!(test.list("worker/").await.is_empty());
}

/// Scenario: The same state on a node that does not host the type.
#[tokio::test(start_paused = true)]
async fn a_node_that_does_not_host_the_type_creates_it_and_its_first_message_fails_with_the_cause_state() {
    let test = node(false).await;
    // Another node hosts the worker: it registers the type.
    let workers = ActorSystem::builder(test.store().clone()).host(WorkerActor).build();
    workers.initialize().await.unwrap();
    test.system().watch(&WatchTarget::from(w1().address().unwrap()), &creator().address().unwrap()).await.unwrap();

    assert_eq!(create(&test, "worker/w1", json!({"done": "seven"}), false).await, "ok");
    let snapshot = test.system().get_actor(&w1().address().unwrap()).await.unwrap().expect("created");
    assert_eq!(snapshot.state, json!({"done": "seven"}));

    let m = test.send(&w1(), WorkerMsg::Work).await;
    assert_eq!(test.run_until_idle().await, 0, "this node does not host the worker");
    assert_eq!(workers.run_due_turns(1).await.unwrap(), 1);
    test.run_until_idle().await;
    let failed = test.state::<_, Notes>(&creator()).await.failed;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!((&failed[0].message_id, failed[0].cause), (&m.id, FailureCause::State));
}
