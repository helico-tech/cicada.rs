//! The supervisor, on virtual time with `TestActors` (ADR 0028): its start, the restart of a child at fault, one
//! restart per incident, a child that goes away, the restart limit and both kinds of escalation.

mod common;

use std::borrow::Cow;
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::NodeConfig;
use cicada::store::OutcomeStatus;
use cicada::{FailureCause, StopCause, Stopped, WatchTarget};
use cicada_actors::{
    Child, ChildEntry, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, StoredMessage, Supervisor,
    SupervisorActor, SupervisorMsg, SupervisorState,
};
use cicada_test::{TestActors, claim_and_abandon};
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

fn sup() -> Supervisor {
    Supervisor::named("s")
}

/// A node with the supervisor, the worker (when `workers` is set) and the witness, which watches every supervisor.
async fn node_with(workers: bool, config: Option<NodeConfig>) -> TestActors {
    let test = TestActors::builder()
        .with(move |node| {
            let mut node = node.host(SupervisorActor).host(WitnessActor);
            if workers {
                node = node.host(WorkerActor);
            }
            if let Some(config) = config {
                node = node.config(config);
            }
            node
        })
        .build()
        .await
        .unwrap();
    witness(&test, "cicada.supervisor").await;
    test
}

async fn node() -> TestActors {
    node_with(true, None).await
}

/// A child entry for `worker/{id}` with the default state, and the start message `Begin` when `start` is set.
fn child(id: &str, start: bool) -> Child<Worker> {
    let child = Child::new(&worker(id), &WorkerState::default()).unwrap();
    if start { child.start(WorkerMsg::Begin).unwrap() } else { child }
}

/// Creates the supervisor `s` with `state` and starts it.
async fn started(test: &TestActors, state: SupervisorState) {
    test.create(&sup(), &state).await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.run_until_idle().await;
}

async fn state(test: &TestActors) -> SupervisorState {
    test.state(&sup()).await
}

async fn entry(test: &TestActors, id: &str) -> ChildEntry {
    state(test).await.children.get(&addr(&worker(id))).cloned().unwrap_or_else(|| panic!("worker/{id} is a child"))
}

async fn worker_state(test: &TestActors, id: &str) -> WorkerState {
    test.state(&worker(id)).await
}

/// The targets the supervisor `s` watches.
async fn watched(test: &TestActors) -> Vec<String> {
    let page = test.system().watches(Some(&addr(&sup())), None, None, 100).await.unwrap();
    page.items.into_iter().map(|w| w.target.to_string()).collect()
}

/// The `Failed` the witness was told of messages of the supervisor `s`.
async fn escalations(test: &TestActors) -> Vec<cicada::Failed> {
    told(test).await.into_iter().filter(|f| f.actor == addr(&sup())).collect()
}

/// Sends `message` to `worker/{id}` and runs every due turn.
async fn to_worker(test: &TestActors, id: &str, message: WorkerMsg) -> cicada::store::Message {
    let m = test.send(&worker(id), message).await;
    test.run_until_idle().await;
    m
}

/// A pending message of `worker/{id}`, due in an hour.
async fn pending_work(test: &TestActors, id: &str) -> cicada::store::Message {
    test.send_with(&worker(id), WorkerMsg::Work, SendOptions::new().delay(Duration::from_secs(3600))).await
}

// ---- the start ----

/// Scenario: Start.
#[tokio::test(start_paused = true)]
async fn start_creates_each_child_as_its_own_watches_it_and_sends_the_start_message() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", true)).child(child("b", false))).await;
    for id in ["a", "b"] {
        let snapshot = test.system().get_actor(&addr(&worker(id))).await.unwrap().expect("created");
        assert_eq!(snapshot.parent, Some(addr(&sup())), "{id} is a child of the supervisor");
        assert_eq!(snapshot.state["n"], 0);
        assert!(entry(&test, id).await.running);
    }
    assert_eq!(worker_state(&test, "a").await.log, ["begin"], "the start message");
    assert!(worker_state(&test, "b").await.log.is_empty());
    assert_eq!(watched(&test).await, ["worker/a", "worker/b"]);
}

/// Scenario: A second start.
#[tokio::test(start_paused = true)]
async fn a_second_start_sends_no_second_start_message() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", true))).await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.run_until_idle().await;
    assert_eq!(worker_state(&test, "a").await.log, ["begin"]);
}

/// Scenario: One child exists elsewhere.
#[tokio::test(start_paused = true)]
async fn a_child_that_exists_elsewhere_fails_its_own_start_with_the_cause_commit_and_the_other_starts() {
    let test = node().await;
    test.create(&worker("a"), &WorkerState::default()).await;
    started(&test, SupervisorState::new().child(child("a", true)).child(child("b", true))).await;
    assert_eq!(worker_state(&test, "b").await.log, ["begin"], "the second child is started");
    assert!(!entry(&test, "a").await.running, "the first is not running");
    assert!(worker_state(&test, "a").await.log.is_empty());
    let failed = escalations(&test).await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!((failed[0].message_type.as_str(), failed[0].cause), ("StartChild", FailureCause::Commit));
    assert_eq!(failed[0].attempts, 1, "a rejected commit is not tried again");
}

/// Scenario: The supervisor goes away.
#[tokio::test(start_paused = true)]
async fn the_children_go_with_their_supervisor() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false)).child(child("b", false))).await;
    test.system().delete_actor(&addr(&sup())).await.unwrap();
    assert!(!test.exists(&worker("a")).await && !test.exists(&worker("b")).await);
}

// ---- its own messages ----

/// Scenario: An escalation sent by a client.
#[tokio::test(start_paused = true)]
async fn an_escalation_sent_by_a_client_ends_done_and_nothing_above_restarts() {
    let test = node().await;
    let outer = Supervisor::named("outer");
    let inner =
        Child::new(&sup(), &SupervisorState::new().child(child("a", false))).unwrap().start(SupervisorMsg::Start);
    test.create(&outer, &SupervisorState::new().child(inner.unwrap())).await;
    test.send(&outer, SupervisorMsg::Start).await;
    test.run_until_idle().await;
    let escalate = SupervisorMsg::Escalate { actor: addr(&worker("a")), error: "made up".into(), restart: true };
    let m = test.send(&sup(), escalate).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    let outer_state: SupervisorState = test.state(&outer).await;
    assert!(outer_state.children[&addr(&sup())].restarts.is_empty(), "nothing restarted");
    assert!(escalations(&test).await.is_empty());
}

/// Scenario: A start of an actor that is not a child. A `Stopped` from a client takes the child out of the state before
/// the supervisor's own `StartChild` for it runs.
#[tokio::test(start_paused = true)]
async fn a_start_child_of_an_address_the_state_does_not_hold_creates_nothing() {
    let test = node().await;
    test.create(&sup(), &SupervisorState::new().child(child("a", true))).await;
    test.send(&sup(), SupervisorMsg::Start).await;
    let gone = Stopped { actor: addr(&worker("a")), cause: StopCause::Stopped, by: Some(addr(&worker("a"))) };
    test.send(&sup(), SupervisorMsg::Stopped(gone)).await;
    test.run_until_idle().await;
    assert!(state(&test).await.children.is_empty());
    assert!(!test.exists(&worker("a")).await, "nothing is created");
    // A `StartChild` from a client is not the supervisor's own either.
    test.create(&Supervisor::named("t"), &SupervisorState::new().child(child("b", false))).await;
    let m = test.send(&Supervisor::named("t"), SupervisorMsg::StartChild { actor: addr(&worker("b")) }).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(!test.exists(&worker("b")).await);
}

// ---- a start that is tried again ----

/// A node on the store of `test` that hosts the worker, and registers it.
async fn workers_node(test: &TestActors) -> ActorSystem {
    let workers = ActorSystem::builder(test.store().clone()).host(WorkerActor).build();
    workers.initialize().await.unwrap();
    workers
}

/// Scenario: A type that is registered a little later.
#[tokio::test(start_paused = true)]
async fn a_type_that_is_registered_ten_seconds_after_the_start_is_found_by_a_later_attempt() {
    let test = node_with(false, None).await;
    test.create(&sup(), &SupervisorState::new().child(child("a", true))).await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.advance_time_by(Duration::from_secs(10)).await;
    assert!(!test.exists(&worker("a")).await, "not registered yet");
    let workers = workers_node(&test).await;
    test.advance_time_by(Duration::from_secs(60)).await;
    assert!(test.exists(&worker("a")).await, "started by a later attempt");
    let entry = entry(&test, "a").await;
    assert!(entry.running && !entry.given_up, "{entry:?}");
    assert!(escalations(&test).await.is_empty(), "nobody is told of the attempts that failed");
    assert_eq!(workers.run_due_turns(10).await.unwrap(), 1);
    assert_eq!(worker_state(&test, "a").await.log, ["begin"]);
}

/// Scenario: A type that never comes.
#[tokio::test(start_paused = true)]
async fn a_type_that_never_comes_gives_the_child_up_with_an_escalation_that_is_refused() {
    let test = node_with(false, None).await;
    test.create(&sup(), &SupervisorState::new().child(child("a", true))).await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.advance_time_by(Duration::from_secs(62)).await;
    assert!(!entry(&test, "a").await.given_up, "attempts are left until 63 seconds");
    test.advance_time_by(Duration::from_secs(2)).await;
    let entry = entry(&test, "a").await;
    assert!(entry.given_up && !entry.running, "{entry:?}");
    assert!(entry.last_error.as_deref().unwrap().contains("no registered actor type worker"), "{entry:?}");
    let failed = escalations(&test).await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!((failed[0].message_type.as_str(), failed[0].cause), ("Escalate", FailureCause::Refused));
    assert!(failed[0].error.starts_with("cannot start worker/a: "), "{}", failed[0].error);
}

/// Scenario: A supervisor above is not disturbed.
#[tokio::test(start_paused = true)]
async fn a_supervisor_above_does_not_restart_one_that_cannot_start_a_child() {
    let test = node_with(false, None).await;
    let outer = Supervisor::named("outer");
    let inner = Child::new(&sup(), &SupervisorState::new().child(child("a", false))).unwrap();
    test.create(&outer, &SupervisorState::new().child(inner.start(SupervisorMsg::Start).unwrap())).await;
    test.send(&outer, SupervisorMsg::Start).await;
    test.advance_time_by(Duration::from_secs(120)).await;
    assert!(entry(&test, "a").await.given_up, "the inner supervisor gave up on its child");
    let outer_state: SupervisorState = test.state(&outer).await;
    let inner_entry = &outer_state.children[&addr(&sup())];
    assert!(inner_entry.running && inner_entry.restarts.is_empty() && !inner_entry.given_up, "{inner_entry:?}");
    let failed = escalations(&test).await;
    assert_eq!(failed.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused]);
}

/// Scenario: A start message the type does not take.
#[tokio::test(start_paused = true)]
async fn a_start_message_the_type_does_not_take_leaves_the_child_running_without_it_and_given_up() {
    let test = node().await;
    let nope = StoredMessage { message_type: "Nope".into(), payload: json!({}) };
    let entry = ChildEntry { start: Some(nope), ..ChildEntry::new(json!({"n": 0, "log": []})) };
    let mut state = SupervisorState::new();
    state.children.insert(addr(&worker("a")), entry);
    test.create(&sup(), &state).await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.advance_time_by(Duration::from_secs(70)).await;
    let entry = self::entry(&test, "a").await;
    assert!(entry.given_up && entry.running, "{entry:?}");
    assert!(test.exists(&worker("a")).await, "created, and runs without its start message");
    assert_eq!(watched(&test).await, ["worker/a"]);
    let failed = escalations(&test).await;
    assert_eq!(failed.len(), 1);
    assert!(failed[0].error.starts_with("cannot start worker/a: ") && failed[0].error.contains("Nope"));
}

/// Scenario: A child that was given up gets a new try.
#[tokio::test(start_paused = true)]
async fn a_start_tries_a_child_again_that_was_given_up_and_is_not_running() {
    let test = node_with(false, None).await;
    test.create(&sup(), &SupervisorState::new().child(child("a", true))).await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.advance_time_by(Duration::from_secs(70)).await;
    assert!(entry(&test, "a").await.given_up);
    workers_node(&test).await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.run_until_idle().await;
    let entry = entry(&test, "a").await;
    assert!(entry.running && !entry.given_up && entry.last_error.is_none(), "{entry:?}");
    assert!(test.exists(&worker("a")).await);
}

/// Scenario: A stuck restart is ended by a start.
#[tokio::test(start_paused = true)]
async fn a_start_ends_a_restart_whose_restarted_was_purged() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    test.send(&worker("a"), WorkerMsg::Fail).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the failure");
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the restart");
    assert!(entry(&test, "a").await.restart_id.is_some());
    let purged = test.system().purge_mailbox(&addr(&sup())).await.unwrap();
    assert_eq!(purged, Some(2), "the Restarted goes, with the Stopped of the old life");
    to_worker(&test, "a", WorkerMsg::Fail).await;
    assert_eq!(entry(&test, "a").await.restarts.len(), 1, "a failure during the stuck restart is ignored");
    test.send(&sup(), SupervisorMsg::Start).await;
    test.run_until_idle().await;
    assert_eq!(entry(&test, "a").await.restart_id, None, "the restart is ended");
    to_worker(&test, "a", WorkerMsg::Fail).await;
    assert_eq!(entry(&test, "a").await.restarts.len(), 2, "the next failure restarts it");
}

/// Restarts `worker/{id}` until the supervisor gives up on it: four failures, each after a restart.
async fn fail_until_given_up(test: &TestActors, id: &str) {
    for _ in 0..4 {
        to_worker(test, id, WorkerMsg::Fail).await;
    }
    assert!(entry(test, id).await.given_up);
}

/// Scenario: A child that was given up and still runs is taken back.
#[tokio::test(start_paused = true)]
async fn a_start_takes_back_a_child_that_was_given_up_and_still_runs() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    fail_until_given_up(&test, "a").await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    let pending = pending_work(&test, "a").await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.run_until_idle().await;
    let entry = entry(&test, "a").await;
    assert!(entry.running && !entry.given_up && entry.restarts.is_empty() && entry.last_error.is_none(), "{entry:?}");
    assert_eq!(worker_state(&test, "a").await.n, 1, "it keeps its state");
    assert_eq!(status(&test, &pending).await, None, "and its mailbox");
    to_worker(&test, "a", WorkerMsg::Fail).await;
    assert_eq!(self::entry(&test, "a").await.restarts.len(), 1, "its next failure restarts it");
    assert_eq!(worker_state(&test, "a").await.n, 0);
}

// ---- a restart ----

/// Scenario: A restart.
#[tokio::test(start_paused = true)]
async fn a_child_that_fails_is_restarted_with_the_state_of_its_entry_and_its_start_message() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", true))).await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    let pending = [pending_work(&test, "a").await, pending_work(&test, "a").await];
    let failed = to_worker(&test, "a", WorkerMsg::Fail).await;
    assert_eq!(status(&test, &failed).await, Some(OutcomeStatus::Failed));
    for m in &pending {
        assert_eq!(status(&test, m).await, Some(OutcomeStatus::Purged), "a restart empties the mailbox");
    }
    assert_eq!(worker_state(&test, "a").await, WorkerState { log: vec!["begin".into()], ..WorkerState::default() });
    assert_eq!(watched(&test).await, ["worker/a"], "the watch stays");
    let entry = entry(&test, "a").await;
    assert_eq!((entry.restarts.len(), entry.restart_id, entry.running), (1, None, true));
}

/// Scenario: A turn that ran too long.
#[tokio::test(start_paused = true)]
async fn a_child_whose_turn_ran_too_long_is_restarted() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    to_worker(&test, "a", WorkerMsg::Hang).await;
    assert_eq!(entry(&test, "a").await.restarts.len(), 1);
    assert_eq!(worker_state(&test, "a").await.n, 0);
}

/// Scenario: A state that cannot be read.
#[tokio::test(start_paused = true)]
async fn a_child_whose_state_cannot_be_read_is_given_up_and_shown_not_restarted() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    to_worker(&test, "a", WorkerMsg::Poison).await;
    let pending = pending_work(&test, "a").await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    let entry = entry(&test, "a").await;
    assert!(entry.given_up && entry.running && entry.restarts.is_empty(), "{entry:?}");
    assert_eq!(test.state_json(&worker("a")).await.unwrap()["poisoned"], "poisoned", "it keeps its state");
    assert_eq!(status(&test, &pending).await, None, "and its mailbox");
    let failed = escalations(&test).await;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].cause, FailureCause::Refused);
    assert!(failed[0].error.starts_with("cannot read worker/a: "), "{}", failed[0].error);
}

/// The scenarios of a message that is at fault, a commit that is rejected and a failure of the store: the child keeps its
/// state and its pending messages, and no restart is counted.
async fn not_restarted_for(message: WorkerMsg, cause: FailureCause) {
    let test = node().await;
    witness_of_workers(&test).await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    let pending = pending_work(&test, "a").await;
    let m = to_worker(&test, "a", message).await;
    assert_eq!(told(&test).await.iter().find(|f| f.message_id == m.id).unwrap().cause, cause);
    assert_eq!(worker_state(&test, "a").await.n, 1, "{cause}: it keeps its state");
    assert_eq!(status(&test, &pending).await, None, "{cause}: and its pending messages");
    assert!(entry(&test, "a").await.restarts.is_empty(), "{cause}");
}

/// Lets the witness watch the workers too.
async fn witness_of_workers(test: &TestActors) {
    test.system().watch(&WatchTarget::parse("worker").unwrap(), &addr(&Witness)).await.unwrap();
}

/// Scenario: A message the child refused.
#[tokio::test(start_paused = true)]
async fn a_message_the_child_refused_restarts_nothing() {
    not_restarted_for(WorkerMsg::Refuse, FailureCause::Refused).await;
}

/// Scenario: A commit that is rejected.
#[tokio::test(start_paused = true)]
async fn a_commit_that_is_rejected_restarts_nothing() {
    not_restarted_for(WorkerMsg::Orphan, FailureCause::Commit).await;
}

/// Scenario: A failure of the store.
#[tokio::test(start_paused = true)]
async fn a_failure_of_the_store_restarts_nothing() {
    not_restarted_for(WorkerMsg::StoreDown, FailureCause::Node).await;
}

/// Scenario: A message that cannot be decoded. A node that does not host the worker takes the message without decoding
/// it.
#[tokio::test(start_paused = true)]
async fn a_message_the_child_cannot_decode_restarts_nothing() {
    let test = node().await;
    witness_of_workers(&test).await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    let pending = pending_work(&test, "a").await;
    let other = ActorSystem::builder(test.store().clone()).build();
    other.initialize().await.unwrap();
    let sent = other.send_message(&addr(&worker("a")), "Note", json!({"text": 7}), SendOptions::new()).await.unwrap();
    let cicada::SendResult::Accepted(m) = sent else { panic!("{sent:?}") };
    test.run_until_idle().await;
    assert_eq!(told(&test).await.iter().find(|f| f.message_id == m.id).unwrap().cause, FailureCause::Message);
    assert_eq!(worker_state(&test, "a").await.n, 1);
    assert_eq!(status(&test, &pending).await, None);
    assert!(entry(&test, "a").await.restarts.is_empty());
}

/// Scenario: The claim limit.
#[tokio::test(start_paused = true)]
async fn the_claim_limit_restarts_nothing_and_counts_no_restart() {
    let config = NodeConfig { max_lost_claims: Some(2), ..NodeConfig::default() };
    let test = node_with(true, Some(config)).await;
    witness_of_workers(&test).await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    let pending = pending_work(&test, "a").await;
    let m = test.send(&worker("a"), WorkerMsg::Work).await;
    for _ in 0..2 {
        let lease = Duration::from_secs(30);
        assert_eq!(claim_and_abandon(test.store(), &types::<Worker>(), lease, 1).await.unwrap().len(), 1);
        tokio::time::sleep(lease + Duration::from_secs(1)).await;
    }
    test.run_until_idle().await;
    assert_eq!(told(&test).await.iter().find(|f| f.message_id == m.id).unwrap().cause, FailureCause::ClaimLimit);
    assert_eq!(worker_state(&test, "a").await.n, 1);
    assert_eq!(status(&test, &pending).await, None);
    assert!(entry(&test, "a").await.restarts.is_empty(), "no restart is counted");
}

/// Scenario: An actor that is not a child.
#[tokio::test(start_paused = true)]
async fn a_failure_of_an_actor_that_is_not_a_child_changes_nothing() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    test.create(&worker("other"), &WorkerState::default()).await;
    test.system().watch(&WatchTarget::from(addr(&worker("other"))), &addr(&sup())).await.unwrap();
    let before = test.version(&sup()).await;
    to_worker(&test, "other", WorkerMsg::Work).await;
    to_worker(&test, "other", WorkerMsg::Fail).await;
    assert_eq!(test.version(&sup()).await, before, "the state is not written");
    assert_eq!(worker_state(&test, "other").await.n, 1, "and the other actor is not restarted");
}

// ---- one incident, one restart ----

/// Scenario: A child with a backlog, and Scenario: A clock that stands still.
#[tokio::test(start_paused = true)]
async fn five_queued_failures_restart_a_child_once_while_the_clock_stands_still() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", true))).await;
    let at = test.now();
    test.system().pause_mailbox(&addr(&sup())).await.unwrap();
    for _ in 0..5 {
        test.send(&worker("a"), WorkerMsg::Fail).await;
    }
    test.run_until_idle().await;
    assert_eq!(test.pending(&sup()).await.len(), 5, "five Failed wait for the supervisor");
    test.system().resume_mailbox(&addr(&sup())).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(test.now(), at, "the time of the store did not move");
    let entry = entry(&test, "a").await;
    assert_eq!((entry.restarts.len(), entry.restart_id, entry.running), (1, None, true), "restarted once");
    assert_eq!(worker_state(&test, "a").await.log, ["begin"], "one start message");
    assert_eq!(watched(&test).await, ["worker/a"], "it still holds and watches the child");
}

/// Scenario: The old life of a restarted child.
#[tokio::test(start_paused = true)]
async fn the_stopped_of_the_life_a_restart_deleted_is_ignored() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    to_worker(&test, "a", WorkerMsg::Fail).await;
    assert_eq!(entry(&test, "a").await.restarts.len(), 1);
    assert_eq!(watched(&test).await, ["worker/a"], "it still watches the child");
    assert!(test.exists(&worker("a")).await);
}

/// Scenario: The new life fails.
#[tokio::test(start_paused = true)]
async fn a_failure_of_the_new_life_restarts_the_child_again() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    to_worker(&test, "a", WorkerMsg::Fail).await;
    to_worker(&test, "a", WorkerMsg::Fail).await;
    assert_eq!(entry(&test, "a").await.restarts.len(), 2);
}

/// Scenario: A `Restarted` of another restart. A `Start` ends the first restart before its `Restarted` is handled, and a
/// `Failed` from a client starts a second one.
#[tokio::test(start_paused = true)]
async fn a_restarted_of_another_restart_does_not_end_the_restart_under_way() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    test.send(&worker("a"), WorkerMsg::Fail).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the failure");
    let failure = test.pending(&sup()).await[0].payload.clone();
    test.send(&sup(), SupervisorMsg::Start).await;
    test.send(&sup(), SupervisorMsg::Failed(serde_json::from_value(failure).unwrap())).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the first restart");
    let first = entry(&test, "a").await.restart_id.unwrap();
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the start ends it");
    assert_eq!(entry(&test, "a").await.restart_id, None);
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the second restart");
    let second = entry(&test, "a").await.restart_id.unwrap();
    assert_ne!(first, second);
    assert_eq!(test.pending(&sup()).await[0].id, first, "the Restarted of the first restart comes next");
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(entry(&test, "a").await.restart_id, Some(second), "the restart under way is not ended");
    test.run_until_idle().await;
    assert_eq!(entry(&test, "a").await.restart_id, None, "its own Restarted ends it");
}

// ---- a child that goes away ----

/// Scenario: A child stops itself.
#[tokio::test(start_paused = true)]
async fn a_child_that_stops_itself_is_forgotten() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false)).child(child("b", false))).await;
    to_worker(&test, "a", WorkerMsg::StopSelf).await;
    assert_eq!(state(&test).await.children.keys().map(ToString::to_string).collect::<Vec<_>>(), ["worker/b"]);
    assert_eq!(watched(&test).await, ["worker/b"]);
}

/// Scenario: An operator deletes a child.
#[tokio::test(start_paused = true)]
async fn a_child_an_operator_deletes_is_forgotten_and_not_created_again() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", true))).await;
    test.system().delete_actor(&addr(&worker("a"))).await.unwrap();
    test.run_until_idle().await;
    test.send(&sup(), SupervisorMsg::Start).await;
    test.run_until_idle().await;
    assert!(state(&test).await.children.is_empty());
    assert!(!test.exists(&worker("a")).await);
}

/// Scenario: An operator deletes a child while one of its failures waits.
#[tokio::test(start_paused = true)]
async fn a_delete_by_an_operator_holds_while_a_failure_of_the_child_waits() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", true))).await;
    test.system().pause_mailbox(&addr(&sup())).await.unwrap();
    to_worker(&test, "a", WorkerMsg::Fail).await;
    test.system().delete_actor(&addr(&worker("a"))).await.unwrap();
    test.system().resume_mailbox(&addr(&sup())).await.unwrap();
    test.run_until_idle().await;
    assert!(!test.exists(&worker("a")).await);
    assert!(state(&test).await.children.is_empty());
    assert!(watched(&test).await.is_empty());
}

/// Scenario: A child that stopped itself just before its restart.
#[tokio::test(start_paused = true)]
async fn a_child_that_stops_itself_right_after_a_failure_is_not_brought_back() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    test.system().pause_mailbox(&addr(&sup())).await.unwrap();
    test.send(&worker("a"), WorkerMsg::Fail).await;
    test.send(&worker("a"), WorkerMsg::StopSelf).await;
    test.run_until_idle().await;
    test.system().resume_mailbox(&addr(&sup())).await.unwrap();
    test.run_until_idle().await;
    assert!(!test.exists(&worker("a")).await);
    assert!(state(&test).await.children.is_empty());
}

// ---- the restart limit ----

/// Scenario: The fourth failure in a minute.
#[tokio::test(start_paused = true)]
async fn the_fourth_failure_in_a_minute_gives_the_child_up_and_fails_a_message_as_a_handler_does() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    for n in 1..=3 {
        to_worker(&test, "a", WorkerMsg::Fail).await;
        assert_eq!(entry(&test, "a").await.restarts.len(), n);
        test.advance_time_by(Duration::from_secs(5)).await;
    }
    assert!(escalations(&test).await.is_empty());
    to_worker(&test, "a", WorkerMsg::Fail).await;
    let entry = entry(&test, "a").await;
    assert!(entry.given_up && entry.running && entry.restarts.len() == 3, "{entry:?}");
    let failed = escalations(&test).await;
    assert_eq!(failed.len(), 1);
    assert_eq!((failed[0].message_type.as_str(), failed[0].cause), ("Escalate", FailureCause::Handler));
    assert!(failed[0].error.starts_with("restart limit: worker/a failed 4 times in PT1M: boom"), "{}", failed[0].error);
}

/// Scenario: A supervisor above is told.
#[tokio::test(start_paused = true)]
async fn a_supervisor_above_restarts_one_that_gave_up_after_too_many_restarts() {
    let test = node().await;
    let outer = Supervisor::named("outer");
    let inner = Child::new(&sup(), &SupervisorState::new().child(child("a", true))).unwrap();
    test.create(&outer, &SupervisorState::new().child(inner.start(SupervisorMsg::Start).unwrap())).await;
    test.send(&outer, SupervisorMsg::Start).await;
    test.run_until_idle().await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    for _ in 0..4 {
        to_worker(&test, "a", WorkerMsg::Fail).await;
    }
    let outer_state: SupervisorState = test.state(&outer).await;
    assert_eq!(outer_state.children[&addr(&sup())].restarts.len(), 1, "the supervisor above restarted it");
    let entry = entry(&test, "a").await;
    assert!(entry.running && !entry.given_up && entry.restarts.is_empty(), "built again from its entry: {entry:?}");
    assert_eq!(worker_state(&test, "a").await, WorkerState { log: vec!["begin".into()], ..WorkerState::default() });
}

/// Scenario: Failures that are far apart.
#[tokio::test(start_paused = true)]
async fn failures_thirty_seconds_apart_restart_the_child_each_time() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    for n in 1..=4 {
        to_worker(&test, "a", WorkerMsg::Work).await;
        to_worker(&test, "a", WorkerMsg::Fail).await;
        assert_eq!(worker_state(&test, "a").await.n, 0, "restart {n}");
        test.advance_time_by(Duration::from_secs(30)).await;
    }
    let entry = entry(&test, "a").await;
    assert!(!entry.given_up, "{entry:?}");
    assert!(escalations(&test).await.is_empty());
    assert_eq!(entry.restarts.len(), 2, "the window keeps the restarts of the last minute");
}

/// Scenario: A child that is given up fails again.
#[tokio::test(start_paused = true)]
async fn a_child_that_is_given_up_and_fails_again_is_left_running() {
    let test = node().await;
    started(&test, SupervisorState::new().child(child("a", false))).await;
    fail_until_given_up(&test, "a").await;
    to_worker(&test, "a", WorkerMsg::Work).await;
    let before = test.version(&sup()).await;
    to_worker(&test, "a", WorkerMsg::Fail).await;
    assert_eq!(test.version(&sup()).await, before, "nothing is done");
    assert_eq!(worker_state(&test, "a").await.n, 1, "the child keeps running");
    assert_eq!(escalations(&test).await.len(), 1);
}

/// The worker of a later deploy: the same contract, but a state whose `n` is 100 or more no longer decodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "WorkerState", into = "WorkerState")]
struct StrictState(WorkerState);

impl TryFrom<WorkerState> for StrictState {
    type Error = String;

    fn try_from(state: WorkerState) -> Result<Self, String> {
        if state.n >= 100 { Err(format!("n {} is too large", state.n)) } else { Ok(Self(state)) }
    }
}

impl From<StrictState> for WorkerState {
    fn from(state: StrictState) -> Self {
        state.0
    }
}

impl JsonSchema for StrictState {
    fn schema_name() -> Cow<'static, str> {
        WorkerState::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        WorkerState::json_schema(generator)
    }
}

struct StrictWorkerActor;

impl Actor for StrictWorkerActor {
    type Ref = Worker;
    type State = StrictState;
    const MAX_TURN: Option<Duration> = Some(WORKER_MAX_TURN);

    async fn handle(&self, _: &mut Turn<Self>, _: WorkerMsg) -> HandlerResult {
        Ok(())
    }
}

/// Scenario: A state that can no longer be created. The restart runs on a node of a later deploy, whose worker reads
/// the state of the entry no more.
#[tokio::test(start_paused = true)]
async fn a_restart_whose_create_is_refused_deletes_the_child_and_gives_it_up_without_a_restart_above() {
    let test = node().await;
    let large = Child::new(&worker("a"), &WorkerState { n: 500, ..WorkerState::default() }).unwrap();
    started(&test, SupervisorState::new().child(large)).await;
    let later = ActorSystem::builder(test.store().clone()).host(SupervisorActor).host(StrictWorkerActor).build();
    later.initialize().await.unwrap();
    test.send(&worker("a"), WorkerMsg::Fail).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the failure, on the first node");
    assert_eq!(later.run_due_turns(1).await.unwrap(), 1, "the restart, on the later node");
    test.run_until_idle().await;
    assert!(!test.exists(&worker("a")).await, "deleted and not created again");
    let entry = entry(&test, "a").await;
    assert!(entry.given_up && !entry.running, "{entry:?}");
    let failed = escalations(&test).await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].cause, FailureCause::Refused, "nobody restarts for it");
    assert!(failed[0].error.starts_with("cannot start worker/a: ") && failed[0].error.contains("too large"));
}

/// Scenario: A redrive of an escalation.
#[tokio::test(start_paused = true)]
async fn a_redrive_of_an_escalation_by_dead_letters_ends_done_and_restarts_nothing() {
    let test = TestActors::builder()
        .with(|node| node.host(SupervisorActor).host(WorkerActor).host(DeadLettersActor::default()))
        .build()
        .await
        .unwrap();
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    test.system().watch(&WatchTarget::Every, &addr(&DeadLetters::default())).await.unwrap();
    let outer = Supervisor::named("outer");
    let inner = Child::new(&sup(), &SupervisorState::new().child(child("a", false))).unwrap();
    test.create(&outer, &SupervisorState::new().child(inner.start(SupervisorMsg::Start).unwrap())).await;
    test.send(&outer, SupervisorMsg::Start).await;
    test.run_until_idle().await;
    for _ in 0..4 {
        to_worker(&test, "a", WorkerMsg::Fail).await;
    }
    let restarts = |state: SupervisorState| state.children[&addr(&sup())].restarts.len();
    assert_eq!(restarts(test.state(&outer).await), 1, "the escalation restarted the supervisor");
    let letters: DeadLettersState = test.state(&DeadLetters::default()).await;
    let (id, _) = letters.letters.iter().find(|(_, l)| l.message_type == "Escalate").expect("a letter of the Escalate");
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: id.clone() }).await;
    test.run_until_idle().await;
    let letters: DeadLettersState = test.state(&DeadLetters::default()).await;
    let redriven = letters.letters[id].redriven_as.clone().expect("redriven");
    let outcome = test.system().outcome(&redriven).await.unwrap().expect("an outcome");
    assert_eq!(outcome.status, OutcomeStatus::Done, "the redriven Escalate ends done");
    assert_eq!(restarts(test.state(&outer).await), 1, "nothing is restarted");
}
