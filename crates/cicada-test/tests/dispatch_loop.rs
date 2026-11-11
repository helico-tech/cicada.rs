//! The dispatch loop in real time on an in-memory store.

mod common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::ActorSystem;
use cicada::address::{ActorAddress, TypeName};
use cicada::prelude::*;
use cicada::runtime::{ActorSystemBuilder, NodeConfig, NodeState, SendResult};
use cicada::store::{Claim, MessageStatus, NodeId, OutcomeStatus, OutgoingMessage, Store, StoreError, StoreResult};
use cicada_test::InMemoryStore;
use common::*;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use tokio::time::Instant;

const SECOND: Duration = Duration::from_secs(1);

/// A lease TTL of 3 s and a poll interval of 50 ms.
fn config() -> NodeConfig {
    NodeConfig { lease_ttl: Duration::from_secs(3), poll_interval: Duration::from_millis(50), ..NodeConfig::default() }
}

/// A node with a lease TTL of 3 s and a poll interval of 50 ms.
fn system(store: Arc<dyn Store>) -> ActorSystemBuilder {
    ActorSystem::builder(store).config(config())
}

async fn total(store: &dyn Store, r: &Counter) -> i32 {
    state_of::<CounterState>(store, &r.address()).await.total
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Worker {
    id: i32,
}

impl ActorRef for Worker {
    const TYPE: &'static str = "worker";
    type Message = WorkerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum WorkerMsg {
    Job {
        millis: u64,
        #[serde(default, rename = "firstAttemptOnly")]
        first_attempt_only: bool,
    },
}

#[derive(Default)]
struct Gauges {
    running: AtomicUsize,
    max_running: AtomicUsize,
    done: AtomicUsize,
}

/// Decrements `running` however the handler ends (also when it is cancelled).
struct Running<'a>(&'a Gauges);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::SeqCst);
    }
}

struct WorkerActor(Arc<Gauges>);

impl Actor for WorkerActor {
    type Ref = Worker;
    type State = NoState;
    type Message = WorkerMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: WorkerMsg) -> HandlerResult {
        let WorkerMsg::Job { millis, first_attempt_only } = message;
        let now = self.0.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.max_running.fetch_max(now, Ordering::SeqCst);
        {
            let _running = Running(&self.0);
            if !first_attempt_only || turn.message().attempt == 1 {
                tokio::time::sleep(Duration::from_millis(millis)).await;
            }
        }
        self.0.done.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

async fn stop_all(systems: &[&ActorSystem]) {
    for s in systems {
        s.stop_with_grace(Duration::ZERO).await;
    }
}

#[tokio::test]
async fn turns_run_end_to_end_across_instances() {
    let store = Arc::new(InMemoryStore::default());
    let system = system(store.clone()).host(CounterActor).build();
    system.start().await.unwrap();
    let (a, b, c) = (Counter::new("a"), Counter::new("b"), Counter::new("c"));
    system.create(&a, &CounterState::default()).await.unwrap();
    system.create(&b, &CounterState::default()).await.unwrap();
    system.send(&a, CounterMsg::Burst { count: 20 }).await.unwrap();
    system.send(&a, CounterMsg::Forward { to: "counter/b".into(), n: 5 }).await.unwrap();
    system.send(&a, CounterMsg::Spawn { child: "c".into(), n: 1 }).await.unwrap();
    eventually(5 * SECOND, || async {
        total(&*store, &a).await == 210
            && total(&*store, &b).await == 5
            && store.get(&c.address()).await.unwrap().is_some()
    })
    .await;
    eventually(5 * SECOND, || async { total(&*store, &c).await == 101 }).await;
    stop_all(&[&system]).await;
}

#[tokio::test]
async fn concurrency_is_bounded_by_the_per_node_limit() {
    let gauges = Arc::new(Gauges::default());
    let store = Arc::new(InMemoryStore::default());
    let system = system(store.clone())
        .host(WorkerActor(gauges.clone()))
        .config(NodeConfig { concurrency: 3, ..config() })
        .build();
    system.start().await.unwrap();
    for id in 0..12 {
        system.create(&Worker { id }, &NoState {}).await.unwrap();
        system.send(&Worker { id }, WorkerMsg::Job { millis: 100, first_attempt_only: false }).await.unwrap();
    }
    eventually(5 * SECOND, || async { gauges.done.load(Ordering::SeqCst) == 12 }).await;
    assert_eq!(gauges.max_running.load(Ordering::SeqCst), 3);
    stop_all(&[&system]).await;
}

#[tokio::test]
async fn own_enqueues_wake_the_loop_without_waiting_for_the_poll_interval() {
    let store = Arc::new(InMemoryStore::default());
    let system = system(store.clone())
        .host(CounterActor)
        .config(NodeConfig { poll_interval: Duration::from_secs(3600), ..config() })
        .build();
    system.start().await.unwrap();
    let a = Counter::new("a");
    system.create(&a, &CounterState::default()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await; // the loop is asleep now
    let started = Instant::now();
    system.send(&a, CounterMsg::Burst { count: 5 }).await.unwrap(); // the turn's own sends wake the loop at commit, too
    eventually(2 * SECOND, || async { total(&*store, &a).await == 15 }).await;
    assert!(started.elapsed() < 2 * SECOND);
    stop_all(&[&system]).await;
}

#[tokio::test]
async fn delayed_messages_are_delivered_at_their_time_not_before() {
    let store = Arc::new(InMemoryStore::default());
    let system = system(store.clone()).host(CounterActor).build();
    system.start().await.unwrap();
    let a = Counter::new("a");
    system.create(&a, &CounterState::default()).await.unwrap();
    let started = Instant::now();
    system.send_with(&a, CounterMsg::Add { n: 1 }, SendOptions::new().delay(Duration::from_millis(400))).await.unwrap();
    eventually(5 * SECOND, || async { total(&*store, &a).await == 1 }).await;
    assert!(started.elapsed() >= Duration::from_millis(400));
    stop_all(&[&system]).await;
}

/// A store with a wake-up source: every call subscribes to `signals`; the types asked for are recorded.
struct WakeUps {
    signals: broadcast::Sender<()>,
    seen: Mutex<Vec<BTreeSet<TypeName>>>,
}

#[async_trait]
impl Faults for WakeUps {
    fn wake_ups(&self, _: &dyn Store, types: &BTreeSet<TypeName>) -> Option<BoxStream<'static, ()>> {
        self.seen.lock().unwrap().push(types.clone());
        let rx = self.signals.subscribe();
        Some(Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.ok().map(|()| ((), rx)) })))
    }
}

#[tokio::test]
async fn a_store_wake_up_source_wakes_the_loop() {
    let memory = Arc::new(InMemoryStore::default());
    let (signals, _) = broadcast::channel(16);
    let store = Faulty::new(memory.clone(), WakeUps { signals, seen: Mutex::new(Vec::new()) });
    let system = system(store.clone())
        .host(CounterActor)
        .config(NodeConfig { poll_interval: Duration::from_secs(3600), ..config() })
        .build();
    system.start().await.unwrap();
    let a = Counter::new("a");
    system.create(&a, &CounterState::default()).await.unwrap();
    eventually(5 * SECOND, || async { !store.faults.seen.lock().unwrap().is_empty() }).await;
    assert_eq!(*store.faults.seen.lock().unwrap(), vec![BTreeSet::from([type_name::<Counter>()])]);
    // `start` asks for the wake-ups before the loop first ran: let the loop
    // run and fall asleep first.
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Another node's enqueue: no in-process wake-up here.
    enqueue(&*memory, OutgoingMessage::new(a.address(), "Add", json!({"n": 1}))).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(total(&*memory, &a).await, 0);
    store.faults.signals.send(()).unwrap();
    eventually(2 * SECOND, || async { total(&*memory, &a).await == 1 }).await;
    stop_all(&[&system]).await;
}

#[tokio::test]
async fn graceful_stop_lets_running_turns_finish_within_the_grace_period() {
    let gauges = Arc::new(Gauges::default());
    let store = Arc::new(InMemoryStore::default());
    let system = system(store.clone()).host(WorkerActor(gauges.clone())).build();
    system.start().await.unwrap();
    system.create(&Worker { id: 1 }, &NoState {}).await.unwrap();
    let m = accepted(
        system.send(&Worker { id: 1 }, WorkerMsg::Job { millis: 300, first_attempt_only: false }).await.unwrap(),
    );
    eventually(5 * SECOND, || async { gauges.running.load(Ordering::SeqCst) == 1 }).await;
    system.stop_with_grace(5 * SECOND).await;
    assert_eq!(system.state(), NodeState::Stopped);
    assert_eq!(outcome_of(&*store, &m.id).await.unwrap().status, OutcomeStatus::Done);
    assert!(!system.health().await);
}

#[tokio::test]
async fn turns_still_running_after_the_grace_period_are_cancelled_and_their_leases_released() {
    let gauges = Arc::new(Gauges::default());
    let store = Arc::new(InMemoryStore::default());
    let first = system(store.clone())
        .config(NodeConfig { node_id: NodeId::new("first").unwrap(), ..config() })
        .host(WorkerActor(gauges.clone()))
        .build();
    first.start().await.unwrap();
    first.create(&Worker { id: 1 }, &NoState {}).await.unwrap();
    let m = accepted(
        first.send(&Worker { id: 1 }, WorkerMsg::Job { millis: 60_000, first_attempt_only: true }).await.unwrap(),
    );
    eventually(5 * SECOND, || async { gauges.running.load(Ordering::SeqCst) == 1 }).await;
    let started = Instant::now();
    first.stop_with_grace(Duration::from_millis(200)).await;
    assert!(started.elapsed() < 2 * SECOND);
    match store.status(&m.id).await.unwrap() {
        // Released: not running, not finished.
        Some(MessageStatus::Pending(message)) => assert_eq!(message.attempt, 1),
        other => panic!("not pending: {other:?}"),
    }

    let second = system(store.clone())
        .config(NodeConfig { node_id: NodeId::new("second").unwrap(), ..config() })
        .host(WorkerActor(gauges.clone()))
        .build();
    second.start().await.unwrap();
    // Claimed again at once, without waiting for the TTL.
    eventually(5 * SECOND, || async { outcome_of(&*store, &m.id).await.is_some() }).await;
    assert_eq!(outcome_of(&*store, &m.id).await.unwrap().envelope.attempt, 2);
    stop_all(&[&second]).await;
}

#[derive(Default)]
struct CountSweeps(AtomicUsize);

#[async_trait]
impl Faults for CountSweeps {
    async fn sweep_expired(&self, inner: &dyn Store) -> StoreResult<u64> {
        self.0.fetch_add(1, Ordering::SeqCst);
        inner.sweep_expired().await
    }
}

#[tokio::test]
async fn the_retention_sweep_runs_periodically() {
    let store = Faulty::new(Arc::new(InMemoryStore::default()), CountSweeps::default());
    let system =
        system(store.clone()).config(NodeConfig { sweep_interval: Duration::from_millis(50), ..config() }).build();
    system.start().await.unwrap();
    eventually(5 * SECOND, || async { store.faults.0.load(Ordering::SeqCst) >= 3 }).await;
    stop_all(&[&system]).await;
}

/// Claims fail while `failures` is positive (counting down).
struct FailingClaims(AtomicI32);

#[async_trait]
impl Faults for FailingClaims {
    async fn claim_due(
        &self,
        inner: &dyn Store,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        if self.0.fetch_sub(1, Ordering::SeqCst) > 0 {
            return Err(StoreError::unavailable("store down"));
        }
        inner.claim_due(types, node, ttl, limit).await
    }
}

#[tokio::test]
async fn the_loop_survives_store_failures() {
    let memory = Arc::new(InMemoryStore::default());
    let store = Faulty::new(memory.clone(), FailingClaims(AtomicI32::new(3)));
    let system = system(store).host(CounterActor).build();
    let a = Counter::new("a");
    system.create(&a, &CounterState::default()).await.unwrap();
    system.send(&a, CounterMsg::Add { n: 3 }).await.unwrap();
    system.start().await.unwrap();
    eventually(5 * SECOND, || async { total(&*memory, &a).await == 3 }).await;
    stop_all(&[&system]).await;
}

#[tokio::test]
async fn life_cycle_start_once_stop_is_idempotent_a_send_only_node_needs_no_loop() {
    let store = Arc::new(InMemoryStore::default());
    let system = system(store.clone()).host(CounterActor).build();
    system.start().await.unwrap();
    assert_eq!(system.state(), NodeState::Started);
    assert!(system.health().await);
    assert!(system.start().await.is_err());
    system.stop().await;
    system.stop().await;
    assert_eq!(system.state(), NodeState::Stopped);

    let send_only = self::system(store.clone()).build();
    send_only.start().await.unwrap();
    assert!(send_only.health().await);
    let none = ActorAddress::parse("counter/none").unwrap();
    assert_eq!(
        send_only.send_message(&none, "Add", json!({"n": 1}), SendOptions::new()).await.unwrap(),
        SendResult::NoSuchInstance
    );
    stop_all(&[&send_only]).await;
}
