//! Watches on a node, in virtual time (ADR 0018, ADR 0023): `Turn::watch` and `Turn::unwatch` with their checks, the
//! notifications a watcher handles for a failure and for a removal, and `ActorSystem::watch`, `unwatch` and `watches`
//! with their refusals. Public API only.

use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::{BulkSelection, NodeConfig, RetryResult, SendResult, WatchResult};
use cicada::store::{Message, MessageStatus, OutcomeRecord, OutcomeStatus};
use cicada::testing::{TestActors, TokioClock, default_start};
use cicada::{Failed, StopCause, Stopped, Store, WatchTarget};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// A supervisor: watches what it is told to, and notes every notification it handles.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Supervisor {
    id: String,
}

impl ActorRef for Supervisor {
    const TYPE: &'static str = "supervisor";
    type Message = SupervisorMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SupervisorMsg {
    Failed(Failed),
    Stopped(Stopped),
    /// Watches the target.
    Watch {
        target: String,
    },
    /// Removes the watch of the target.
    Unwatch {
        target: String,
    },
    /// Watches the target and removes the watch again, in one turn.
    WatchThenUnwatch {
        target: String,
    },
    /// Watches the target and then fails.
    WatchThenFail {
        target: String,
    },
    /// Tries to watch the target, and notes how the call ended.
    TryWatch {
        target: String,
    },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Notes {
    #[serde(default)]
    failed: Vec<Failed>,
    #[serde(default)]
    stopped: Vec<Stopped>,
    /// Fails on every notification.
    #[serde(default)]
    refuse: bool,
    #[serde(default)]
    calls: Vec<String>,
}

struct SupervisorActor;

impl Actor for SupervisorActor {
    type Ref = Supervisor;
    type State = Notes;

    async fn handle(&self, turn: &mut Turn<Self>, message: SupervisorMsg) -> HandlerResult {
        let refuse = turn.state().refuse;
        match message {
            SupervisorMsg::Failed(_) | SupervisorMsg::Stopped(_) if refuse => return Err(turn.fail("refused")),
            SupervisorMsg::Failed(failed) => turn.state_mut().failed.push(failed),
            SupervisorMsg::Stopped(stopped) => turn.state_mut().stopped.push(stopped),
            SupervisorMsg::Watch { target } => turn.watch(target.as_str())?,
            SupervisorMsg::Unwatch { target } => turn.unwatch(target.as_str())?,
            SupervisorMsg::WatchThenUnwatch { target } => {
                turn.watch(target.as_str())?;
                turn.unwatch(target.as_str())?;
            }
            SupervisorMsg::WatchThenFail { target } => {
                turn.watch(target.as_str())?;
                return Err(HandlerError::msg("failed after the watch"));
            }
            SupervisorMsg::TryWatch { target } => {
                let call = match turn.watch(target.as_str()) {
                    Ok(()) => "watching".to_owned(),
                    Err(e) => format!("refused: {e}"),
                };
                turn.state_mut().calls.push(call);
            }
        }
        Ok(())
    }
}

/// A worker: its messages succeed or fail, it stops, and it creates children.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Worker {
    id: String,
}

impl ActorRef for Worker {
    const TYPE: &'static str = "worker";
    type Message = WorkerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum WorkerMsg {
    /// Fails when `fail`.
    Work { fail: bool },
    /// Stops.
    Stop,
    /// Creates the worker `key` as a child.
    Spawn { key: String },
}

struct WorkerActor;

impl Actor for WorkerActor {
    type Ref = Worker;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: WorkerMsg) -> HandlerResult {
        match message {
            WorkerMsg::Work { fail: true } => return Err(HandlerError::msg("boom")),
            WorkerMsg::Work { fail: false } => {}
            WorkerMsg::Stop => turn.stop(),
            WorkerMsg::Spawn { key } => turn.create_child(&Worker { id: key }, &NoState {})?,
        }
        Ok(())
    }
}

/// A type whose family declares `Failed` but not `Stopped`: it may not watch.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Lonely;

impl ActorRef for Lonely {
    const TYPE: &'static str = "lonely";
    type Message = LonelyMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
enum LonelyMsg {
    Failed(Failed),
    TryWatch { target: String },
}

struct LonelyActor;

impl Actor for LonelyActor {
    type Ref = Lonely;
    type State = Notes;

    async fn handle(&self, turn: &mut Turn<Self>, message: LonelyMsg) -> HandlerResult {
        if let LonelyMsg::TryWatch { target } = message {
            let call = match turn.watch(target.as_str()) {
                Ok(()) => "watching".to_owned(),
                Err(e) => format!("refused: {e}"),
            };
            turn.state_mut().calls.push(call);
        }
        Ok(())
    }
}

/// A type with a `Failed` of its own: it may not watch either.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Imposter;

impl ActorRef for Imposter {
    const TYPE: &'static str = "imposter";
    type Message = ImposterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ImposterMsg {
    Failed { reason: String },
    Stopped(Stopped),
}

struct ImposterActor;

impl Actor for ImposterActor {
    type Ref = Imposter;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: ImposterMsg) -> HandlerResult {
        Ok(())
    }
}

fn supervisor(key: &str) -> Supervisor {
    Supervisor { id: key.to_owned() }
}

fn worker(key: &str) -> Worker {
    Worker { id: key.to_owned() }
}

fn address<R: ActorRef>(r: &R) -> ActorAddress {
    r.address().unwrap()
}

fn on<R: ActorRef>(r: &R) -> WatchTarget {
    WatchTarget::Actor(address(r))
}

async fn node() -> TestActors {
    TestActors::builder()
        .with(|node| node.host(SupervisorActor).host(WorkerActor).host(LonelyActor).host(ImposterActor))
        .build()
        .await
        .unwrap()
}

/// A node with the supervisor `key` (refusing notifications when `refuse`) and the worker `w1`.
async fn with_actors(refuse: bool) -> TestActors {
    let test = node().await;
    test.create(&supervisor("a"), &Notes { refuse, ..Notes::default() }).await;
    test.create(&worker("w1"), &NoState {}).await;
    test
}

/// Every stored watch, as `(target, watcher)` text.
async fn watches(test: &TestActors) -> Vec<(String, String)> {
    let page = test.system().watches(None, None, None, 100).await.unwrap();
    page.items.into_iter().map(|w| (w.target.to_string(), w.watcher.to_string())).collect()
}

async fn notes(test: &TestActors, key: &str) -> Notes {
    test.state(&supervisor(key)).await
}

async fn watch_from_outside(test: &TestActors, target: WatchTarget, watcher: &Supervisor) {
    assert_eq!(test.system().watch(&target, &address(watcher)).await.unwrap(), WatchResult::Created);
}

/// The retained outcomes of `actor`, oldest first.
async fn outcomes_of(test: &TestActors, actor: &ActorAddress) -> Vec<OutcomeRecord> {
    let page = test.system().outcomes(None, actor.as_str(), None, None, 100).await.unwrap();
    page.items.into_iter().filter(|o| o.actor == *actor).collect()
}

/// Scenario: A turn sets a watch.
#[tokio::test(start_paused = true)]
async fn a_turn_sets_a_watch() {
    let test = with_actors(false).await;
    test.send(&supervisor("a"), SupervisorMsg::Watch { target: "worker/w1".into() }).await;
    test.run_until_idle().await;
    assert_eq!(watches(&test).await, [("worker/w1".to_owned(), "supervisor/a".to_owned())]);
}

/// Scenario: A turn removes a watch.
#[tokio::test(start_paused = true)]
async fn a_turn_removes_a_watch_and_is_not_told_any_more() {
    let test = with_actors(false).await;
    test.send(&supervisor("a"), SupervisorMsg::Watch { target: "worker/w1".into() }).await;
    test.send(&supervisor("a"), SupervisorMsg::Unwatch { target: "worker/w1".into() }).await;
    test.run_until_idle().await;
    assert!(watches(&test).await.is_empty());
    test.send(&worker("w1"), WorkerMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    assert!(notes(&test, "a").await.failed.is_empty(), "the actor is not told");
}

/// Scenario: A turn removes a watch it does not hold.
#[tokio::test(start_paused = true)]
async fn a_turn_removes_a_watch_it_does_not_hold() {
    let test = with_actors(false).await;
    let sent = test.send(&supervisor("a"), SupervisorMsg::Unwatch { target: "worker/w1".into() }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&sent).await.unwrap().status, OutcomeStatus::Done, "the commit succeeds");
    assert!(watches(&test).await.is_empty(), "nothing changes");
}

/// Scenario: A turn sets and removes one watch.
#[tokio::test(start_paused = true)]
async fn a_turn_sets_and_removes_one_watch_and_the_last_call_stands() {
    let test = with_actors(false).await;
    test.send(&supervisor("a"), SupervisorMsg::WatchThenUnwatch { target: "worker/w1".into() }).await;
    test.run_until_idle().await;
    assert!(watches(&test).await.is_empty());
}

/// Scenario: A turn that fails sets no watch.
#[tokio::test(start_paused = true)]
async fn a_turn_that_fails_sets_no_watch() {
    let test = with_actors(false).await;
    let sent = test.send(&supervisor("a"), SupervisorMsg::WatchThenFail { target: "worker/w1".into() }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&sent).await.unwrap().status, OutcomeStatus::Failed);
    assert!(watches(&test).await.is_empty());
}

/// Scenarios: A turn of an actor without the two messages; A text that is no target (in a turn).
#[tokio::test(start_paused = true)]
async fn a_watch_of_an_actor_without_the_two_messages_or_of_no_target_fails_at_the_call() {
    let test = with_actors(false).await;
    test.create(&Lonely, &Notes::default()).await;
    let sent = test.send(&Lonely, LonelyMsg::TryWatch { target: "worker/w1".into() }).await;
    test.send(&supervisor("a"), SupervisorMsg::TryWatch { target: "billing..x/".into() }).await;
    test.run_until_idle().await;
    let calls = test.state::<_, Notes>(&Lonely).await.calls;
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("has no message type Stopped"), "{calls:?}");
    assert_eq!(test.outcome(&sent).await.unwrap().status, OutcomeStatus::Done, "the turn can go on");
    let calls = notes(&test, "a").await.calls;
    assert!(calls[0].starts_with("refused: Invalid watch target 'billing..x/'"), "{calls:?}");
    assert!(watches(&test).await.is_empty(), "no watch is stored");
}

/// Scenario: A message fails for good.
#[tokio::test(start_paused = true)]
async fn a_message_that_fails_for_good_tells_each_watcher() {
    let test = with_actors(false).await;
    test.create(&supervisor("b"), &Notes::default()).await;
    for key in ["a", "b"] {
        watch_from_outside(&test, on(&worker("w1")), &supervisor(key)).await;
    }
    let options = SendOptions::new().correlation_id("run-7");
    let failed = test.send_with(&worker("w1"), WorkerMsg::Work { fail: true }, options).await;
    test.run_until_idle().await;
    let expected = Failed {
        actor: address(&worker("w1")),
        message_id: failed.id.clone(),
        message_type: "Work".into(),
        payload: json!({"fail": true}),
        correlation_id: Some("run-7".into()),
        from: None,
        error: "boom".into(),
        attempts: 1,
    };
    for key in ["a", "b"] {
        assert_eq!(notes(&test, key).await.failed, std::slice::from_ref(&expected), "{key}");
        let told = outcomes_of(&test, &address(&supervisor(key))).await;
        assert_eq!(told.len(), 1);
        let envelope = &told[0].envelope;
        assert!(envelope.notification, "marked");
        assert_eq!(envelope.from.as_ref(), Some(&address(&worker("w1"))), "sent by the actor");
        assert_eq!(envelope.caused_by, None, "no causedBy");
        assert_eq!(envelope.message_type, "Failed");
    }
}

/// Scenario: An attempt that is retried.
#[tokio::test(start_paused = true)]
async fn no_watcher_is_told_until_the_last_attempt_fails() {
    let test = with_actors(false).await;
    watch_from_outside(&test, on(&worker("w1")), &supervisor("a")).await;
    let policy = RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    test.send_with(&worker("w1"), WorkerMsg::Work { fail: true }, SendOptions::new().retry(policy)).await;
    test.run_until_idle().await;
    assert!(notes(&test, "a").await.failed.is_empty(), "the attempt is retried");
    assert!(test.pending(&supervisor("a")).await.is_empty());
    test.advance_time_by(Duration::from_secs(1)).await;
    let failed = notes(&test, "a").await.failed;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].attempts, 2);
}

/// Scenario: A message that succeeds.
#[tokio::test(start_paused = true)]
async fn a_message_that_succeeds_tells_nobody() {
    let test = with_actors(false).await;
    watch_from_outside(&test, on(&worker("w1")), &supervisor("a")).await;
    watch_from_outside(&test, WatchTarget::Every, &supervisor("a")).await;
    test.send(&worker("w1"), WorkerMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    assert!(outcomes_of(&test, &address(&supervisor("a"))).await.is_empty());
}

/// Scenario: An actor stops itself.
#[tokio::test(start_paused = true)]
async fn an_actor_that_stops_itself_tells_its_watchers() {
    let test = with_actors(false).await;
    watch_from_outside(&test, on(&worker("w1")), &supervisor("a")).await;
    let stop = test.send(&worker("w1"), WorkerMsg::Stop).await;
    test.run_until_idle().await;
    let w1 = address(&worker("w1"));
    assert_eq!(
        notes(&test, "a").await.stopped,
        [Stopped { actor: w1.clone(), cause: StopCause::Stopped, by: Some(w1.clone()) }]
    );
    let told = outcomes_of(&test, &address(&supervisor("a"))).await;
    assert_eq!(told[0].envelope.caused_by.as_ref(), Some(&stop.id), "caused by the message whose turn stopped it");
}

/// Scenarios: A child is removed with a parent that stops; A child is removed with its parent (a delete).
#[tokio::test(start_paused = true)]
async fn a_child_removed_with_its_parent_tells_the_watchers_of_the_child() {
    let test = with_actors(false).await;
    for parent in ["p", "q"] {
        test.create(&worker(parent), &NoState {}).await;
        test.send(&worker(parent), WorkerMsg::Spawn { key: format!("{parent}-kid") }).await;
    }
    test.run_until_idle().await;
    for parent in ["p", "q"] {
        watch_from_outside(&test, on(&worker(&format!("{parent}-kid"))), &supervisor("a")).await;
    }
    test.send(&worker("p"), WorkerMsg::Stop).await;
    test.run_until_idle().await;
    test.system().delete_actor(&address(&worker("q"))).await.unwrap();
    test.run_until_idle().await;
    let (p, kid_p, kid_q) = (address(&worker("p")), address(&worker("p-kid")), address(&worker("q-kid")));
    assert_eq!(
        notes(&test, "a").await.stopped,
        [
            Stopped { actor: kid_p, cause: StopCause::Parent, by: Some(p) },
            Stopped { actor: kid_q, cause: StopCause::Parent, by: None },
        ]
    );
}

/// Scenario: Two actors that watch each other.
#[tokio::test(start_paused = true)]
async fn two_actors_that_watch_each_other_cannot_loop() {
    let test = with_actors(true).await;
    test.create(&supervisor("b"), &Notes { refuse: true, ..Notes::default() }).await;
    watch_from_outside(&test, on(&supervisor("b")), &supervisor("a")).await;
    watch_from_outside(&test, on(&supervisor("a")), &supervisor("b")).await;
    test.send(&supervisor("a"), SupervisorMsg::WatchThenFail { target: "worker/x".into() }).await;
    let turns = test.run_until_idle().await;
    assert_eq!(turns, 2, "the failing message and the notification of b, which fails too");
    let told_b = outcomes_of(&test, &address(&supervisor("b"))).await;
    assert_eq!((told_b.len(), told_b[0].status), (1, OutcomeStatus::Failed));
    assert_eq!(outcomes_of(&test, &address(&supervisor("a"))).await.len(), 1, "a gets no message about that");
    assert!(test.pending(&supervisor("a")).await.is_empty() && test.pending(&supervisor("b")).await.is_empty());
}

/// Scenario: A target that does not exist yet.
#[tokio::test(start_paused = true)]
async fn a_watch_on_a_target_that_does_not_exist_yet_tells_of_it_later() {
    let test = with_actors(false).await;
    watch_from_outside(&test, on(&worker("w9")), &supervisor("a")).await;
    test.create(&worker("w9"), &NoState {}).await;
    test.send(&worker("w9"), WorkerMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    assert_eq!(notes(&test, "a").await.failed.len(), 1);
}

/// Scenarios: The target is deleted and created again; An actor is deleted.
#[tokio::test(start_paused = true)]
async fn a_watch_survives_the_delete_and_re_create_of_its_target() {
    let test = with_actors(false).await;
    watch_from_outside(&test, on(&worker("w1")), &supervisor("a")).await;
    test.system().delete_actor(&address(&worker("w1"))).await.unwrap();
    test.create(&worker("w1"), &NoState {}).await;
    test.send(&worker("w1"), WorkerMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    let notes = notes(&test, "a").await;
    let w1 = address(&worker("w1"));
    assert_eq!(notes.stopped, [Stopped { actor: w1.clone(), cause: StopCause::Deleted, by: None }]);
    assert_eq!(notes.failed.len(), 1);
    assert_eq!(notes.failed[0].actor, w1);
}

/// Scenario: A delete wakes the watcher.
#[tokio::test(start_paused = true)]
async fn a_delete_wakes_the_node_of_the_watcher() {
    let store = Store::memory_on(Arc::new(TokioClock::new(default_start())));
    let config = NodeConfig { poll_interval: Duration::from_secs(3600), ..NodeConfig::default() };
    let system = ActorSystem::builder(store.clone()).config(config).host(SupervisorActor).host(WorkerActor).build();
    system.start().await.unwrap();
    system.create(&supervisor("a"), &Notes::default()).await.unwrap();
    system.create(&worker("w1"), &NoState {}).await.unwrap();
    assert_eq!(system.watch(&on(&worker("w1")), &address(&supervisor("a"))).await.unwrap(), WatchResult::Created);
    tokio::time::sleep(Duration::from_millis(10)).await; // the dispatch loop sleeps for its poll interval now
    system.delete_actor(&address(&worker("w1"))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let snapshot = system.get_actor(&address(&supervisor("a"))).await.unwrap().unwrap();
    let notes: Notes = serde_json::from_value(snapshot.state).unwrap();
    assert_eq!(notes.stopped.len(), 1, "handled at once, not after the next poll");
    system.stop().await;
    store.close().await;
}

/// Scenarios: A watcher that does not exist; A watcher whose type is not registered; A watcher whose type lacks a
/// message; A watcher with a `Failed` of its own.
#[tokio::test(start_paused = true)]
async fn a_watch_from_outside_needs_an_existing_watcher_whose_type_may_watch() {
    let test = with_actors(false).await;
    test.create(&Imposter, &NoState {}).await;
    let system = test.system();
    let target = on(&worker("w1"));
    assert_eq!(system.watch(&target, &address(&supervisor("nosuch"))).await.unwrap(), WatchResult::NoSuchInstance);
    let unregistered = ActorAddress::parse("nosuch/x").unwrap();
    assert_eq!(system.watch(&target, &unregistered).await.unwrap(), WatchResult::NoSuchType);
    let lacks = system.watch(&target, &address(&worker("w1"))).await.unwrap();
    assert!(matches!(&lacks, WatchResult::NotAWatcher(why) if why.contains("has no message type Failed")), "{lacks:?}");
    let own = system.watch(&target, &address(&Imposter)).await.unwrap();
    assert!(matches!(&own, WatchResult::NotAWatcher(why) if why.contains("not cicada::Failed")), "{own:?}");
    assert!(watches(&test).await.is_empty(), "no watch is stored");

    let watcher = address(&supervisor("a"));
    assert_eq!(system.watch(&target, &watcher).await.unwrap(), WatchResult::Created);
    assert_eq!(system.watch(&target, &watcher).await.unwrap(), WatchResult::Exists, "idempotent");
    assert_eq!(system.watches(Some(&watcher), Some(&target), None, 10).await.unwrap().items.len(), 1);
    assert!(system.unwatch(&target, &watcher).await.unwrap());
    assert!(!system.unwatch(&target, &watcher).await.unwrap());
}

/// The failed notification in the mailbox of `watcher` that its turn refused.
async fn refused_notification(test: &TestActors, watcher: &Supervisor) -> OutcomeRecord {
    let failures = test.failures().await;
    failures
        .into_iter()
        .find(|o| o.actor == address(watcher) && o.envelope.notification)
        .expect("a failed notification")
}

/// Scenario: A failed notification is sent again from outside.
#[tokio::test(start_paused = true)]
async fn a_failed_notification_sent_again_from_outside_is_an_ordinary_message() {
    let test = with_actors(true).await;
    test.create(&supervisor("b"), &Notes::default()).await;
    watch_from_outside(&test, on(&worker("w1")), &supervisor("a")).await;
    watch_from_outside(&test, on(&supervisor("a")), &supervisor("b")).await;
    test.send(&worker("w1"), WorkerMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    assert!(notes(&test, "b").await.failed.is_empty(), "a failed notification tells nobody");

    let notification = refused_notification(&test, &supervisor("a")).await;
    let retried = match test.system().retry_message(&notification.message_id).await.unwrap() {
        RetryResult::Accepted(m) => m,
        other => panic!("{other:?}"),
    };
    assert!(!retried.notification, "a retry is not marked");
    assert_eq!(retried.caused_by.as_ref(), Some(&notification.message_id));
    test.run_until_idle().await;
    let told = notes(&test, "b").await.failed;
    assert_eq!(told.len(), 1, "the watchers of the watcher are told of the retry's failure");
    assert_eq!(told[0].message_id, retried.id);
    let marked = outcomes_of(&test, &address(&supervisor("b"))).await;
    assert!(marked.iter().all(|o| o.envelope.notification), "with a marked message");
    assert!(test.pending(&supervisor("a")).await.is_empty() && test.pending(&supervisor("b")).await.is_empty());
}

/// Scenario: A sender cannot mark a message.
#[tokio::test(start_paused = true)]
async fn a_message_of_the_type_failed_from_a_sender_is_not_marked() {
    let test = with_actors(true).await;
    test.create(&supervisor("b"), &Notes::default()).await;
    watch_from_outside(&test, on(&supervisor("a")), &supervisor("b")).await;
    let fake = serde_json::to_value(Failed {
        actor: address(&worker("w1")),
        message_id: MessageId::new("m1").unwrap(),
        message_type: "Work".into(),
        payload: json!({}),
        correlation_id: None,
        from: None,
        error: "made up".into(),
        attempts: 1,
    })
    .unwrap();
    let sent = match test.send_raw(&address(&supervisor("a")), "Failed", fake, SendOptions::new()).await {
        SendResult::Accepted(m) => m,
        other => panic!("{other:?}"),
    };
    assert!(!sent.notification);
    test.run_until_idle().await;
    let told = notes(&test, "b").await.failed;
    assert_eq!(told.len(), 1, "its failure tells the watchers: it is not marked");
    assert_eq!(told[0].message_id, sent.id);
}

/// Scenario: A retry by filter after a watcher was told.
#[tokio::test(start_paused = true)]
async fn a_retry_by_filter_after_a_watcher_was_told_sends_the_failed_message_again() {
    let test = with_actors(false).await;
    watch_from_outside(&test, on(&worker("w1")), &supervisor("a")).await;
    let failed: Message = test.send(&worker("w1"), WorkerMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    assert_eq!(notes(&test, "a").await.failed.len(), 1);
    let selection = BulkSelection::Filter { prefix: "worker/".into(), message_type: None };
    let retried = test.system().bulk_retry(selection).await.unwrap();
    assert_eq!(retried.affected, 1, "the notification does not make it count as retried");
    let caused = test.system().caused_by(&failed.id, None, 10).await.unwrap().unwrap();
    assert!(matches!(&caused.items[..], [MessageStatus::Pending(m)] if m.message_type == "Work"));
}
