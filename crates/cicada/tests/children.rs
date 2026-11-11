//! Child actors on a node, in virtual time (ADR 0019): `Turn::create_child` and its checks, the parent in a snapshot,
//! `ActorSystem::list_children`, and a delete or a stop that removes the descendants with their mailboxes. Public API
//! only.

use std::sync::Arc;

use cicada::prelude::*;
use cicada::store::{DeleteResult, Message, MessageStatus, OutcomeStatus};
use cicada::testing::TestActors;
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Notify;

/// A batch: its turns create items, as children or not.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Batch {
    id: String,
}

impl ActorRef for Batch {
    const TYPE: &'static str = "batches";
    type Message = Cmd;
}

/// An item; it can create items below itself, and wait for a gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item {
    id: String,
}

impl ActorRef for Item {
    const TYPE: &'static str = "items";
    type Message = Cmd;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Cmd {
    /// Creates the items of `keys` as children, and notes it in the state.
    Spawn { keys: Vec<String> },
    /// Creates the item `key` as an independent actor.
    Independent { key: String },
    /// Creates the item `key` as a child, then fails.
    SpawnThenFail { key: String },
    /// Creates the item `key` as a child, sends it `Work`, and stops.
    SpawnSendStop { key: String },
    /// Calls `create` and then `create_child` for the item `key`, and notes the error of the second call.
    Both { key: String },
    /// Creates its own address as a child.
    SelfChild,
    /// Stops.
    Stop,
    /// Does nothing: a message to leave pending.
    Work,
    /// Notes that it runs, waits for the gate, and then creates the item `key` as a child.
    Wait { key: String },
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Log {
    #[serde(default)]
    notes: Vec<String>,
}

fn item(key: &str) -> Item {
    Item { id: key.to_owned() }
}

fn batch(key: &str) -> Batch {
    Batch { id: key.to_owned() }
}

/// What both types do with a command.
async fn handle<A: Actor<State = Log>>(turn: &mut Turn<A>, cmd: Cmd, gate: &Gate) -> HandlerResult {
    match cmd {
        Cmd::Spawn { keys } => {
            for key in &keys {
                turn.create_child(&item(key), &Log::default())?;
            }
            turn.state_mut().notes.push(format!("spawned {}", keys.join(",")));
        }
        Cmd::Independent { key } => turn.create(&item(&key), &Log::default())?,
        Cmd::SpawnThenFail { key } => {
            turn.create_child(&item(&key), &Log::default())?;
            return Err(HandlerError::msg("failed after the create"));
        }
        Cmd::SpawnSendStop { key } => {
            turn.create_child(&item(&key), &Log::default())?;
            turn.send(&item(&key), Cmd::Work)?;
            turn.stop();
        }
        Cmd::Both { key } => {
            turn.create(&item(&key), &Log::default())?;
            let second = turn.create_child(&item(&key), &Log::default());
            let note = match second {
                Err(TurnError::Invalid(reason)) => format!("refused: {reason}"),
                other => format!("unexpected: {other:?}"),
            };
            turn.state_mut().notes.push(note);
        }
        Cmd::SelfChild => {
            let me = turn.self_ref().clone();
            turn.create_child(&me, &Log::default())?;
        }
        Cmd::Stop => turn.stop(),
        Cmd::Work => {}
        Cmd::Wait { key } => {
            gate.running.notify_one();
            gate.open.notified().await;
            turn.create_child(&item(&key), &Log::default())?;
        }
    }
    Ok(())
}

/// Lets a test hold a turn while it runs.
#[derive(Default)]
struct Gate {
    running: Notify,
    open: Notify,
}

struct BatchActor(Arc<Gate>);

impl Actor for BatchActor {
    type Ref = Batch;
    type State = Log;

    async fn handle(&self, turn: &mut Turn<Self>, cmd: Cmd) -> HandlerResult {
        handle(turn, cmd, &self.0).await
    }
}

struct ItemActor(Arc<Gate>);

impl Actor for ItemActor {
    type Ref = Item;
    type State = Log;

    async fn handle(&self, turn: &mut Turn<Self>, cmd: Cmd) -> HandlerResult {
        handle(turn, cmd, &self.0).await
    }
}

async fn node() -> (TestActors, Arc<Gate>) {
    let gate = Arc::new(Gate::default());
    let (b, i) = (gate.clone(), gate.clone());
    let test = TestActors::builder()
        .with(move |n| n.host(BatchActor(b)).host(ItemActor(i)))
        .build()
        .await
        .expect("the node starts");
    (test, gate)
}

fn address<R: ActorRef>(r: &R) -> ActorAddress {
    r.address().unwrap()
}

/// The parent of `r`; panics when `r` does not exist.
async fn parent_of<R: ActorRef>(test: &TestActors, r: &R) -> Option<ActorAddress> {
    test.system().get_actor(&address(r)).await.unwrap().expect("exists").parent
}

/// The direct children of `r`, in address order.
async fn children_of<R: ActorRef>(test: &TestActors, r: &R) -> Vec<ActorAddress> {
    test.system().list_children(&address(r), None, 100).await.unwrap().expect("exists").items
}

async fn status_of(test: &TestActors, message: &Message) -> Option<OutcomeStatus> {
    test.outcome(message).await.map(|o| o.status)
}

/// `batches/a` with the child `items/b`, which has the child `items/c`.
async fn three_levels(test: &TestActors) {
    test.create(&batch("a"), &Log::default()).await;
    test.send(&batch("a"), Cmd::Spawn { keys: vec!["b".into()] }).await;
    test.run_until_idle().await;
    test.send(&item("b"), Cmd::Spawn { keys: vec!["c".into()] }).await;
    test.run_until_idle().await;
}

/// Scenario: A child is created.
#[tokio::test(start_paused = true)]
async fn a_child_is_created() {
    let (test, _) = node().await;
    test.create(&batch("b1"), &Log::default()).await;
    let m = test.send(&batch("b1"), Cmd::Spawn { keys: vec!["b1:0".into()] }).await;
    test.run_until_idle().await;
    assert_eq!(status_of(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(test.exists(&item("b1:0")).await);
    assert_eq!(parent_of(&test, &item("b1:0")).await, Some(address(&batch("b1"))));
    assert_eq!(children_of(&test, &batch("b1")).await, [address(&item("b1:0"))]);
    assert_eq!(test.system().list_children(&address(&batch("nosuch")), None, 10).await.unwrap(), None);
}

/// Scenario: An independent actor is created.
#[tokio::test(start_paused = true)]
async fn an_independent_actor_is_created() {
    let (test, _) = node().await;
    test.create(&batch("b1"), &Log::default()).await;
    test.send(&batch("b1"), Cmd::Independent { key: "solo".into() }).await;
    test.run_until_idle().await;
    assert_eq!(parent_of(&test, &item("solo")).await, None, "a create in a turn");
    assert_eq!(parent_of(&test, &batch("b1")).await, None, "a create from outside a turn");
    assert!(children_of(&test, &batch("b1")).await.is_empty());
    test.system().create_actor(&address(&item("put")), json!({})).await.unwrap();
    assert_eq!(parent_of(&test, &item("put")).await, None, "a create of the API");
}

/// Scenario: The turn fails.
#[tokio::test(start_paused = true)]
async fn a_turn_that_fails_creates_no_child() {
    let (test, _) = node().await;
    test.create(&batch("b1"), &Log::default()).await;
    let m = test.send(&batch("b1"), Cmd::SpawnThenFail { key: "b1:0".into() }).await;
    test.run_until_idle().await;
    assert_eq!(status_of(&test, &m).await, Some(OutcomeStatus::Failed));
    assert!(!test.exists(&item("b1:0")).await);
}

/// Scenario: The child exists already.
#[tokio::test(start_paused = true)]
async fn a_child_that_exists_already_keeps_its_state_and_its_mailbox() {
    let (test, _) = node().await;
    test.create(&batch("b1"), &Log::default()).await;
    test.send(&batch("b1"), Cmd::Spawn { keys: vec!["b1:0".into()] }).await;
    test.run_until_idle().await;
    test.send(&item("b1:0"), Cmd::Spawn { keys: vec![] }).await;
    test.run_until_idle().await;
    test.system().pause_mailbox(&address(&item("b1:0"))).await.unwrap();
    let waiting = test.send(&item("b1:0"), Cmd::Work).await;

    let again = test.send(&batch("b1"), Cmd::Spawn { keys: vec!["b1:0".into()] }).await;
    test.run_until_idle().await;
    assert_eq!(status_of(&test, &again).await, Some(OutcomeStatus::Done));
    assert_eq!(test.state::<Item, Log>(&item("b1:0")).await.notes, ["spawned "], "the state stays");
    assert_eq!(test.pending(&item("b1:0")).await.iter().map(|m| &m.id).collect::<Vec<_>>(), [&waiting.id]);
}

/// Scenario: The actor belongs to nobody or to someone else.
#[tokio::test(start_paused = true)]
async fn a_child_create_of_an_actor_of_nobody_or_of_another_parent_fails_the_message() {
    let (test, _) = node().await;
    for key in ["mine", "other"] {
        test.create(&batch(key), &Log::default()).await;
    }
    test.create(&item("solo"), &Log { notes: vec!["solo".into()] }).await;
    test.send(&batch("other"), Cmd::Spawn { keys: vec!["theirs".into()] }).await;
    test.run_until_idle().await;

    for (taken, parent) in [("solo", None), ("theirs", Some(address(&batch("other"))))] {
        let m = test.send(&batch("mine"), Cmd::Spawn { keys: vec!["new".into(), taken.into()] }).await;
        test.run_until_idle().await;
        let outcome = test.outcome(&m).await.expect("finished");
        assert_eq!(outcome.status, OutcomeStatus::Failed, "{taken}");
        assert!(outcome.error.as_deref().unwrap().contains(&format!("items/{taken}")), "{outcome:?}");
        assert!(!test.exists(&item("new")).await, "none of the turn's effects are applied");
        assert!(test.state::<Batch, Log>(&batch("mine")).await.notes.is_empty());
        assert_eq!(parent_of(&test, &item(taken)).await, parent, "{taken} is unchanged");
    }
    assert_eq!(test.state::<Item, Log>(&item("solo")).await.notes, ["solo"]);
}

/// Scenario: One turn creates the same address both ways.
#[tokio::test(start_paused = true)]
async fn one_turn_that_creates_an_address_both_ways_is_refused_at_the_second_call() {
    let (test, _) = node().await;
    test.create(&batch("b1"), &Log::default()).await;
    let m = test.send(&batch("b1"), Cmd::Both { key: "x".into() }).await;
    test.run_until_idle().await;
    assert_eq!(status_of(&test, &m).await, Some(OutcomeStatus::Done), "the handler handled the error");
    let notes = test.state::<Batch, Log>(&batch("b1")).await.notes;
    assert!(notes[0].starts_with("refused: ") && notes[0].contains("items/x"), "{notes:?}");
    assert_eq!(parent_of(&test, &item("x")).await, None, "only the first call was buffered");
}

/// Scenario: An actor names itself.
#[tokio::test(start_paused = true)]
async fn an_actor_that_creates_itself_as_its_child_fails_the_message() {
    let (test, _) = node().await;
    test.create(&batch("b1"), &Log::default()).await;
    let m = test.send(&batch("b1"), Cmd::SelfChild).await;
    test.run_until_idle().await;
    assert_eq!(status_of(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!(parent_of(&test, &batch("b1")).await, None);
}

/// Scenario: An operator deletes a parent.
#[tokio::test(start_paused = true)]
async fn an_operator_delete_of_a_parent_removes_every_descendant() {
    let (test, _) = node().await;
    three_levels(&test).await;
    test.create(&item("other"), &Log::default()).await;
    let deleted = test.system().delete_actor(&address(&batch("a"))).await.unwrap();
    assert_eq!(deleted, DeleteResult::Deleted { purged: 0 });
    assert!(!test.exists(&batch("a")).await);
    assert!(!test.exists(&item("b")).await);
    assert!(!test.exists(&item("c")).await);
    assert_eq!(test.list("").await, [address(&item("other"))], "no other actor changed");
}

/// Scenario: A parent stops.
#[tokio::test(start_paused = true)]
async fn a_parent_that_stops_removes_its_children_and_the_new_one() {
    let (test, _) = node().await;
    test.create(&batch("p"), &Log::default()).await;
    test.send(&batch("p"), Cmd::Spawn { keys: vec!["old".into()] }).await;
    test.run_until_idle().await;
    let m = test.send(&batch("p"), Cmd::SpawnSendStop { key: "new".into() }).await;
    test.run_until_idle().await;
    assert_eq!(status_of(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(test.list("").await.is_empty(), "the parent, its old child and the new one are gone");
}

/// Scenario: A child stops.
#[tokio::test(start_paused = true)]
async fn a_child_that_stops_takes_its_descendants_and_leaves_its_parent_and_siblings() {
    let (test, _) = node().await;
    test.create(&batch("p"), &Log::default()).await;
    test.send(&batch("p"), Cmd::Spawn { keys: vec!["b1".into(), "b2".into()] }).await;
    test.run_until_idle().await;
    test.send(&item("b1"), Cmd::Spawn { keys: vec!["c".into()] }).await;
    test.run_until_idle().await;
    test.send(&item("b1"), Cmd::Stop).await;
    test.run_until_idle().await;
    assert!(!test.exists(&item("b1")).await);
    assert!(!test.exists(&item("c")).await);
    assert!(test.exists(&batch("p")).await);
    assert_eq!(children_of(&test, &batch("p")).await, [address(&item("b2"))]);
}

/// Scenario: Pending messages in the tree.
#[tokio::test(start_paused = true)]
async fn the_pending_messages_of_a_deleted_tree_are_purged_and_counted() {
    let (test, _) = node().await;
    test.create(&batch("p"), &Log::default()).await;
    test.send(&batch("p"), Cmd::Spawn { keys: vec!["kid".into()] }).await;
    test.run_until_idle().await;
    let on_parent = test.send(&batch("p"), Cmd::Work).await;
    let on_child = test.send(&item("kid"), Cmd::Work).await;
    let deleted = test.system().delete_actor(&address(&batch("p"))).await.unwrap();
    assert_eq!(deleted, DeleteResult::Deleted { purged: 2 });
    assert_eq!(status_of(&test, &on_parent).await, Some(OutcomeStatus::Purged));
    assert_eq!(status_of(&test, &on_child).await, Some(OutcomeStatus::Purged));
}

/// Scenario: A message sent by the stopping turn.
#[tokio::test(start_paused = true)]
async fn a_message_that_the_stopping_turn_sends_to_its_child_is_purged() {
    let (test, _) = node().await;
    test.create(&batch("p"), &Log::default()).await;
    test.send(&batch("p"), Cmd::Spawn { keys: vec!["kid".into()] }).await;
    test.run_until_idle().await;
    let m = test.send(&batch("p"), Cmd::SpawnSendStop { key: "kid".into() }).await;
    test.run_until_idle().await;
    assert_eq!(status_of(&test, &m).await, Some(OutcomeStatus::Done));
    let caused = test.system().caused_by(&m.id, None, 10).await.unwrap().expect("m is known").items;
    assert_eq!(caused.len(), 1);
    let MessageStatus::Finished(sent) = &caused[0] else { panic!("not finished: {caused:?}") };
    assert_eq!((sent.status, sent.actor.clone()), (OutcomeStatus::Purged, address(&item("kid"))));
}

/// Scenario: A turn runs on a descendant.
#[tokio::test(start_paused = true)]
async fn a_turn_that_runs_on_a_descendant_while_its_parent_is_deleted_applies_nothing() {
    let (test, gate) = node().await;
    test.create(&batch("p"), &Log::default()).await;
    test.send(&batch("p"), Cmd::Spawn { keys: vec!["kid".into()] }).await;
    test.run_until_idle().await;
    let m = test.send(&item("kid"), Cmd::Wait { key: "late".into() }).await;
    let system = test.system().clone();
    let running = tokio::spawn(async move { system.run_due_turns(1).await.unwrap() });
    gate.running.notified().await;
    assert_eq!(
        test.system().delete_actor(&address(&batch("p"))).await.unwrap(),
        DeleteResult::Deleted { purged: 1 },
        "the message being handled is purged"
    );
    gate.open.notify_one();
    running.await.unwrap();
    assert_eq!(status_of(&test, &m).await, Some(OutcomeStatus::Purged));
    assert!(test.list("").await.is_empty(), "the turn's commit applied nothing");
}
