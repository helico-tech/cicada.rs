//! Operator actions on the runtime, in virtual time: the node table, turn cancel, pause, retry and delivering now.

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cicada::plugin::{HandlerErrorContext, HookResult, OutcomeContext, Plugin};
use cicada::runtime::{BulkSelection, NodeHealth, RetryResult};
use cicada::store::{CommitRequest, CommitResult, DeliverNowResult, MessageStatus, OutcomeStatus, Store, StoreResult};
use cicada::{ActorSystem, SendOptions, Unhandled};
use common::*;
use tokio::time::{Instant, sleep};

fn a() -> Counter {
    Counter::new("a")
}

// The counter type with a maximum turn of 5 minutes.
counter_type!(SlowCounter, Unhandled::Fail, Duration::from_secs(300));

fn slow_node(store: Arc<dyn Store>, name: &str) -> ActorSystem {
    node(store, name).host(SlowCounter).build()
}

/// Runs the system's one due turn in a task of its own.
fn run_one(system: &ActorSystem) -> tokio::task::JoinHandle<usize> {
    let system = system.clone();
    tokio::spawn(async move { system.run_due_turns(1).await.unwrap() })
}

struct RecordHooks(Events);

#[async_trait]
impl Plugin for RecordHooks {
    fn name(&self) -> &str {
        "record"
    }

    async fn on_handler_error(&self, _: &HandlerErrorContext<'_>) -> HookResult {
        self.0.push("on_handler_error");
        Ok(())
    }

    async fn on_outcome(&self, ctx: &OutcomeContext<'_>) -> HookResult {
        self.0.push(format!("on_outcome:{}", outcome_text(ctx.outcome())));
        Ok(())
    }
}

/// Delays every commit by `0`, as a slow store would.
struct SlowCommit(Duration);

#[async_trait]
impl Faults for SlowCommit {
    async fn commit(&self, inner: &dyn Store, commit: CommitRequest) -> StoreResult<CommitResult> {
        sleep(self.0).await;
        inner.commit(commit).await
    }
}

// ---- nodes ----

#[tokio::test(start_paused = true)]
async fn a_started_node_is_listed_alive_goes_suspect_and_dead_and_its_row_goes_on_stop() {
    let memory = virtual_store();
    let system = node(memory.clone(), "n1").host(CounterActor).build();
    system.start().await.unwrap();
    settle().await;
    let nodes = system.nodes().await.unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id.as_str(), "n1");
    assert_eq!(nodes[0].hosted, [type_name::<Counter>()]);
    assert_eq!(nodes[0].status, NodeHealth::Alive);
    assert_eq!(nodes[0].started_at, nodes[0].last_seen);
    assert!(nodes[0].running_turns.is_empty());
    assert_eq!(system.node(&nodes[0].id).await.unwrap().unwrap().id.as_str(), "n1");

    // Heartbeats stop: the status follows the age of the last one.
    system.stop().await;
    assert!(system.nodes().await.unwrap().is_empty(), "stop removes the row");
    memory.heartbeat_node(&cicada::store::NodeId::new("gone").unwrap(), &Default::default()).await.unwrap();
    sleep(Duration::from_secs(29)).await;
    assert_eq!(system.nodes().await.unwrap()[0].status, NodeHealth::Alive);
    sleep(Duration::from_secs(1)).await;
    assert_eq!(system.nodes().await.unwrap()[0].status, NodeHealth::Suspect);
    sleep(Duration::from_secs(90)).await;
    assert_eq!(system.nodes().await.unwrap()[0].status, NodeHealth::Dead, "dead from 2 minutes");
}

#[tokio::test(start_paused = true)]
async fn a_node_heartbeats_every_10_seconds_also_when_it_hosts_nothing() {
    let memory = virtual_store();
    let system = ActorSystem::builder(memory.clone()).config(node_config("sender")).build();
    system.start().await.unwrap();
    settle().await;
    let first = system.nodes().await.unwrap().remove(0);
    assert!(first.hosted.is_empty(), "a send-only node has a row too");
    sleep(Duration::from_secs(25)).await;
    let later = system.nodes().await.unwrap().remove(0);
    assert_eq!(later.status, NodeHealth::Alive);
    assert_eq!(cicada::time::between(first.last_seen, later.last_seen), Duration::from_secs(20));
    assert_eq!(later.started_at, first.started_at);
    system.stop().await;
}

// ---- turn cancel ----

#[tokio::test(start_paused = true)]
async fn cancel_takes_effect_within_one_heartbeat_with_no_effects_and_on_outcome_only() {
    let memory = virtual_store();
    let events = Events::default();
    let system = node(memory.clone(), "n1").host(SlowCounter).install(RecordHooks(events.clone())).build();
    system.create(&a(), &CounterState::total(1)).await.unwrap();
    let running = accepted(system.send(&a(), CounterMsg::Sleep { millis: 300_000 }).await.unwrap());
    let next = accepted(system.send(&a(), CounterMsg::Add { n: 1 }).await.unwrap());
    let started = Instant::now();
    let runner = run_one(&system);
    settle().await;

    let turn = system.running_turn(&a().address()).await.unwrap().expect("a turn is running");
    assert_eq!(turn.message_id, running.id);
    assert!(!turn.cancel_requested);
    assert_eq!(system.nodes().await.unwrap().len(), 0, "no row without a started node");

    assert!(system.cancel_turn(&a().address()).await.unwrap());
    assert!(system.running_turn(&a().address()).await.unwrap().unwrap().cancel_requested);
    assert_eq!(runner.await.unwrap(), 1);
    assert_eq!(started.elapsed(), Duration::from_secs(10), "cancelled at the first heartbeat");

    let outcome = outcome_of(&*memory, &running.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Cancelled);
    assert_eq!(outcome.error, None);
    assert_eq!(state_of::<CounterState>(&*memory, &a().address()).await.total, 1, "no effects");
    assert_eq!(events.all(), ["on_outcome:Cancelled"], "on_handler_error is not called");
    assert_eq!(system.running_turn(&a().address()).await.unwrap(), None);

    // The mailbox moves on to the next message, in a new epoch.
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(outcome_of(&*memory, &next.id).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(state_of::<CounterState>(&*memory, &a().address()).await.total, 2);
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_turn_with_retry_left_is_not_retried() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let options = SendOptions::new().retry(
        cicada::retry::RetryPolicy::new(5, cicada::retry::Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap(),
    );
    let m = accepted(system.send_with(&a(), CounterMsg::Sleep { millis: 300_000 }, options).await.unwrap());
    let runner = run_one(&system);
    settle().await;
    system.cancel_turn(&a().address()).await.unwrap();
    runner.await.unwrap();
    assert_eq!(outcome_of(&*memory, &m.id).await.unwrap().status, OutcomeStatus::Cancelled);
    assert!(memory.pending(&a().address(), None, 10).await.unwrap().items.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_handler_that_already_returned_is_not_cancelled_and_commits_done() {
    let memory = virtual_store();
    let store = Faulty::new(memory.clone(), SlowCommit(Duration::from_secs(15)));
    let system = slow_node(store.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(system.send(&a(), CounterMsg::Add { n: 7 }).await.unwrap());
    let runner = run_one(&system);
    // The handler returned at once; the commit takes 15 s, so a heartbeat (at 10 s) sees the request while it waits.
    sleep(Duration::from_secs(1)).await;
    assert!(system.cancel_turn(&a().address()).await.unwrap());
    runner.await.unwrap();
    assert_eq!(outcome_of(&*memory, &m.id).await.unwrap().status, OutcomeStatus::Done, "G6: the handler returned");
    assert_eq!(state_of::<CounterState>(&*memory, &a().address()).await.total, 7);
}

#[tokio::test(start_paused = true)]
async fn a_request_from_an_earlier_epoch_does_not_cancel_a_later_turn() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    accepted(system.send(&a(), CounterMsg::Sleep { millis: 1_000 }).await.unwrap());
    let later = accepted(system.send(&a(), CounterMsg::Sleep { millis: 30_000 }).await.unwrap());

    // The request lands before the first turn ends, but it ends before its next heartbeat: the request stays recorded
    // under the first turn's epoch.
    let runner = run_one(&system);
    settle().await;
    assert!(system.cancel_turn(&a().address()).await.unwrap());
    runner.await.unwrap();

    let started = Instant::now();
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(started.elapsed(), Duration::from_secs(30), "the later turn runs to its end");
    assert_eq!(outcome_of(&*memory, &later.id).await.unwrap().status, OutcomeStatus::Done);
}

// ---- pause, purge ----

#[tokio::test(start_paused = true)]
async fn a_paused_mailbox_runs_nothing_until_resumed_and_the_snapshot_says_so() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    assert!(system.pause_mailbox(&a().address()).await.unwrap());
    assert!(system.get_actor(&a().address()).await.unwrap().unwrap().paused);
    let m = accepted(system.send(&a(), CounterMsg::Add { n: 3 }).await.unwrap());
    assert_eq!(system.run_due_turns(10).await.unwrap(), 0, "paused: not claimed");
    assert!(!system.mailbox(&a().address(), None, 10).await.unwrap().unwrap().items[0].running);

    assert!(system.resume_mailbox(&a().address()).await.unwrap());
    assert_eq!(system.run_due_turns(10).await.unwrap(), 1);
    assert_eq!(outcome_of(&*memory, &m.id).await.unwrap().status, OutcomeStatus::Done);

    assert!(!system.pause_mailbox(&Counter::new("nope").address()).await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn the_mailbox_marks_the_running_message_and_purge_spares_it() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let running = accepted(system.send(&a(), CounterMsg::Sleep { millis: 30_000 }).await.unwrap());
    let queued = accepted(system.send(&a(), CounterMsg::Add { n: 1 }).await.unwrap());
    let runner = run_one(&system);
    settle().await;

    let mailbox = system.mailbox(&a().address(), None, 10).await.unwrap().unwrap();
    assert_eq!(mailbox.items.len(), 2);
    assert!(mailbox.items[0].running && mailbox.items[0].message.id == running.id);
    assert!(!mailbox.items[1].running);

    assert_eq!(system.purge_mailbox(&a().address()).await.unwrap(), Some(1));
    assert_eq!(outcome_of(&*memory, &queued.id).await.unwrap().status, OutcomeStatus::Purged);
    runner.await.unwrap();
    assert_eq!(
        outcome_of(&*memory, &running.id).await.unwrap().status,
        OutcomeStatus::Done,
        "the running one finishes"
    );
}

// ---- retry and deliver now ----

#[tokio::test(start_paused = true)]
async fn retry_sends_a_failed_or_cancelled_message_again_as_a_new_message_and_refuses_the_rest() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let options = SendOptions::new().id(cicada::store::MessageId::new("orig").unwrap()).correlation_id("corr");
    let failed = accepted(system.send_with(&a(), CounterMsg::Boom { reason: "no".into() }, options).await.unwrap());
    system.run_due_turns(1).await.unwrap();
    assert_eq!(outcome_of(&*memory, &failed.id).await.unwrap().status, OutcomeStatus::Failed);

    let again = match system.retry_message(&failed.id).await.unwrap() {
        RetryResult::Accepted(m) => m,
        other => panic!("expected Accepted, got {other:?}"),
    };
    assert_ne!(again.id, failed.id);
    assert_eq!(again.caused_by, Some(failed.id.clone()));
    assert_eq!(again.to, failed.to);
    assert_eq!(again.message_type, failed.message_type);
    assert_eq!(again.payload, failed.payload);
    assert_eq!(again.correlation_id.as_deref(), Some("corr"));
    assert_eq!(again.attempt, 0, "a new message");
    assert_eq!(again.from, None);

    // Refused while it is pending, and once done.
    assert!(matches!(system.retry_message(&again.id).await.unwrap(), RetryResult::NotRetryable { .. }));
    let done = accepted(system.send(&a(), CounterMsg::Add { n: 1 }).await.unwrap());
    system.run_due_turns(10).await.unwrap(); // the boom again, then the add
    assert!(matches!(system.retry_message(&done.id).await.unwrap(), RetryResult::NotRetryable { .. }));

    assert_eq!(system.retry_message(&cicada::store::MessageId::random()).await.unwrap(), RetryResult::NotFound);
    system.delete_actor(&a().address()).await.unwrap();
    assert_eq!(system.retry_message(&failed.id).await.unwrap(), RetryResult::NoSuchInstance(a().address()));
}

#[tokio::test(start_paused = true)]
async fn deliver_now_makes_a_delayed_message_run_now() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let delayed = accepted(
        system
            .send_with(&a(), CounterMsg::Add { n: 1 }, SendOptions::new().delay(Duration::from_secs(3600)))
            .await
            .unwrap(),
    );
    assert_eq!(system.run_due_turns(1).await.unwrap(), 0, "not due yet");
    assert!(matches!(system.deliver_now(&delayed.id).await.unwrap(), DeliverNowResult::Delivered(_)));
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1, "due at once");
    assert_eq!(outcome_of(&*memory, &delayed.id).await.unwrap().status, OutcomeStatus::Done);
    assert!(matches!(system.deliver_now(&delayed.id).await.unwrap(), DeliverNowResult::AlreadyFinished(_)));
}

// ---- bulk actions ----

async fn pending_ids(memory: &dyn Store) -> Vec<cicada::store::MessageId> {
    memory.list_pending("counter/", None, 1000).await.unwrap().items.into_iter().map(|m| m.id).collect()
}

#[tokio::test(start_paused = true)]
async fn bulk_cancel_by_filter_and_ids_counts_only_what_it_cancelled() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    for id in ["a", "b"] {
        system.create(&Counter::new(id), &CounterState::default()).await.unwrap();
    }
    for id in ["a", "b"] {
        system.send(&Counter::new(id), CounterMsg::Add { n: 1 }).await.unwrap();
        system.send(&Counter::new(id), CounterMsg::Boom { reason: "x".into() }).await.unwrap();
    }
    let result = system
        .bulk_cancel(BulkSelection::Filter { prefix: "counter/a".to_owned(), message_type: Some("Add".to_owned()) })
        .await
        .unwrap();
    assert_eq!((result.affected, result.more), (1, false));

    let ids = pending_ids(&*memory).await;
    assert_eq!(ids.len(), 3);
    let result = system.bulk_cancel(BulkSelection::Ids(ids)).await.unwrap();
    assert_eq!((result.affected, result.more), (3, false));

    let too_many = (0..1001).map(|i| cicada::store::MessageId::new(format!("m{i}")).unwrap()).collect();
    assert!(system.bulk_cancel(BulkSelection::Ids(too_many)).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn bulk_retry_by_filter_skips_already_retried_messages() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    for _ in 0..3 {
        system.send(&a(), CounterMsg::Boom { reason: "no".into() }).await.unwrap();
    }
    system.run_due_turns(1).await.unwrap();
    system.run_due_turns(1).await.unwrap();
    system.run_due_turns(1).await.unwrap();
    let everything = || BulkSelection::Filter { prefix: "counter/".to_owned(), message_type: None };

    let first = system.bulk_retry(everything()).await.unwrap();
    assert_eq!((first.affected, first.more), (3, false));
    assert_eq!(memory.pending(&a().address(), None, 10).await.unwrap().items.len(), 3);

    // Calling again sends nothing twice: each failure has a retry already.
    let again = system.bulk_retry(everything()).await.unwrap();
    assert_eq!((again.affected, again.more), (0, false));
    assert_eq!(memory.pending(&a().address(), None, 10).await.unwrap().items.len(), 3);

    // A failed message of another type is not matched; a filter of its type retries it.
    let result = system
        .bulk_retry(BulkSelection::Filter { prefix: "".to_owned(), message_type: Some("Nope".to_owned()) })
        .await
        .unwrap();
    assert_eq!((result.affected, result.more), (0, false));
}

#[tokio::test(start_paused = true)]
async fn a_turn_cancel_commit_after_a_request_never_runs_the_message_again() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(system.send(&a(), CounterMsg::Sleep { millis: 300_000 }).await.unwrap());
    let runner = run_one(&system);
    settle().await;
    system.cancel_turn(&a().address()).await.unwrap();
    runner.await.unwrap();
    sleep(Duration::from_secs(300)).await;
    assert_eq!(system.run_due_turns(1).await.unwrap(), 0, "G6: a cancelled turn does not run again");
    assert!(matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Finished(_))));
}

#[tokio::test(start_paused = true)]
async fn bulk_filter_caps_at_a_thousand_and_reports_more() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    for _ in 0..1003 {
        system.send(&a(), CounterMsg::Add { n: 1 }).await.unwrap();
    }
    let all = || BulkSelection::Filter { prefix: "".to_owned(), message_type: Some("Add".to_owned()) };
    let first = system.bulk_cancel(all()).await.unwrap();
    assert_eq!((first.affected, first.more), (1000, true));
    let second = system.bulk_cancel(all()).await.unwrap();
    assert_eq!((second.affected, second.more), (3, false));
}

#[tokio::test(start_paused = true)]
async fn bulk_retry_by_filter_caps_at_a_thousand_and_does_not_repeat_on_the_next_call() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    for _ in 0..1003 {
        system.send(&a(), CounterMsg::Boom { reason: "no".into() }).await.unwrap();
    }
    while system.run_due_turns(1).await.unwrap() > 0 {}
    let all = || BulkSelection::Filter { prefix: "counter/".to_owned(), message_type: Some("Boom".to_owned()) };
    let first = system.bulk_retry(all()).await.unwrap();
    assert_eq!((first.affected, first.more), (1000, true));
    let second = system.bulk_retry(all()).await.unwrap();
    assert_eq!((second.affected, second.more), (3, false), "the first thousand are not retried again");
    assert_eq!(system.bulk_retry(all()).await.unwrap().affected, 0);
}

// ---- real time: blocking code cannot be stopped, only followed ----

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Blocker {
    id: String,
}

impl cicada::ActorRef for Blocker {
    const TYPE: &'static str = "blocker";
    type Message = BlockerMsg;
}

#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "type")]
enum BlockerMsg {
    /// Blocks its thread (does not yield) for `millis`.
    Block { millis: u64 },
}

struct BlockerActor(Arc<std::sync::atomic::AtomicBool>);

impl cicada::Actor for BlockerActor {
    type Ref = Blocker;
    type State = cicada::Stateless;
    type Message = BlockerMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _turn: &mut cicada::Turn<Self>, message: BlockerMsg) -> cicada::HandlerResult {
        let BlockerMsg::Block { millis } = message;
        std::thread::sleep(Duration::from_millis(millis));
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocking_code_runs_until_it_returns_and_then_the_commit_is_cancelled() {
    let store = Arc::new(cicada_test::InMemoryStore::default());
    let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut config = node_config("n1");
    config.lease_ttl = Duration::from_secs(4);
    config.heartbeat_interval = Some(Duration::from_millis(50));
    let system = ActorSystem::builder(store.clone()).config(config).host(BlockerActor(returned.clone())).build();
    let blocker = Blocker { id: "b".into() };
    system.create_stateless(&blocker).await.unwrap();
    let m = accepted(system.send(&blocker, BlockerMsg::Block { millis: 500 }).await.unwrap());
    let runner = run_one(&system);
    let address = cicada::ActorRef::address(&blocker).unwrap();
    while store.running_turn(&address).await.unwrap().is_none() {
        tokio::task::yield_now().await;
    }
    assert!(system.cancel_turn(&address).await.unwrap(), "the request comes before the handler ended");
    runner.await.unwrap();
    assert!(returned.load(std::sync::atomic::Ordering::SeqCst), "blocking code ran to its end");
    assert_eq!(
        outcome_of(&*store, &m.id).await.unwrap().status,
        OutcomeStatus::Cancelled,
        "yet the commit is cancelled"
    );
}
