//! A worker runs ordinary Rust actors through a turn source (spec `remote-workers`, ADR 0039), through the public API
//! only, on virtual time: the reference source's error classification and type definitions, contract registration and
//! its conflicts, object safety, initialization that fails and recovers, stop during registration, empty workers,
//! manual and background batches sharing capacity, polling cadence, temporary and permanent lease failures and the
//! stopped waiter. The server here hosts nothing: it only serves the worker.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::runtime::{
    CompleteTurnResult, LeaseTurnsResult, NodeConfig, NodeState, RegisterContractResult, RenewTurnResult, TurnReport,
    WorkerConfig,
};
use cicada::store::{MessageId, NodeId, OutcomeStatus, StoreError, TypeDef};
use cicada::testing::{TokioClock, default_start};
use cicada::{
    OpError, SendResult, SourceError, StartError, StartProblem, Store, TurnSource, TypeName, Worker, WorkerError,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Notify;
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Counter {
    id: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CounterMsg {
    Add { n: i64 },
    Slow { millis: u64 },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct CounterState {
    total: i64,
}

/// How many handlers run at once, and the most that ever did.
#[derive(Default)]
struct Gauge {
    now: AtomicUsize,
    max: AtomicUsize,
}

struct CounterActor(Arc<Gauge>);

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        let running = self.0.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.max.fetch_max(running, Ordering::SeqCst);
        match message {
            CounterMsg::Add { n } => turn.state_mut().total += n,
            CounterMsg::Slow { millis } => {
                sleep(Duration::from_millis(millis)).await;
                turn.state_mut().total += 1;
            }
        }
        self.0.now.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Another type, for a contract that differs from the counter's under the same name.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OtherCounter {
    id: String,
}

impl ActorRef for OtherCounter {
    const TYPE: &'static str = "counter";
    type Message = OtherMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum OtherMsg {
    Reset,
}

struct OtherActor;

impl Actor for OtherActor {
    type Ref = OtherCounter;
    type State = CounterState;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: OtherMsg) -> HandlerResult {
        Ok(())
    }
}

fn counter(id: &str) -> Counter {
    Counter { id: id.into() }
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn store() -> Store {
    Store::memory_on(Arc::new(TokioClock::new(default_start())))
}

/// A server that hosts nothing, with a lease TTL of 30 s.
fn server(store: &Store) -> ActorSystem {
    ActorSystem::builder(store.clone()).config(NodeConfig { lease_ttl: secs(30), ..NodeConfig::default() }).build()
}

fn config(concurrency: usize) -> WorkerConfig {
    WorkerConfig {
        worker_id: NodeId::new("worker").unwrap(),
        concurrency,
        poll_interval: Duration::from_millis(500),
        ..WorkerConfig::default()
    }
}

async fn send(system: &ActorSystem, id: &str, message: CounterMsg) -> cicada::store::Message {
    match system.send(&counter(id), message).await.unwrap() {
        SendResult::Accepted(m) => m,
        other => panic!("expected Accepted, got {other:?}"),
    }
}

async fn total(system: &ActorSystem, id: &str) -> i64 {
    let snapshot = system.get_actor(&counter(id).address().unwrap()).await.unwrap().expect("the counter");
    snapshot.state["total"].as_i64().unwrap()
}

async fn outcome(system: &ActorSystem, id: &MessageId) -> Option<OutcomeStatus> {
    system.outcome(id).await.unwrap().map(|o| o.status)
}

/// A source in front of a server that counts and scripts calls: lease limits and the most slots in flight, failing or
/// hanging registrations, and lease errors.
#[derive(Default)]
struct Probe {
    server: Mutex<Option<ActorSystem>>,
    gauge: Arc<Gauge>,
    concurrency: usize,
    leases: AtomicUsize,
    in_flight: AtomicUsize,
    limits: Mutex<Vec<usize>>,
    registrations: AtomicUsize,
    failing_registrations: AtomicUsize,
    register_gate: Option<Arc<Notify>>,
    lease_error: Mutex<Option<SourceError>>,
    mismatch: AtomicBool,
    releases: AtomicUsize,
}

impl Probe {
    fn server(&self) -> ActorSystem {
        self.server.lock().unwrap().clone().expect("a server")
    }
}

#[async_trait]
impl TurnSource for Probe {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.register_gate {
            gate.notified().await;
        }
        if self.failing_registrations.load(Ordering::SeqCst) > 0 {
            self.failing_registrations.fetch_sub(1, Ordering::SeqCst);
            return Err(SourceError::Unavailable("registry down".into()));
        }
        TurnSource::register_contract(&self.server(), contract).await
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        TurnSource::list_types(&self.server()).await
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        self.leases.fetch_add(1, Ordering::SeqCst);
        self.limits.lock().unwrap().push(limit);
        let in_flight = self.in_flight.fetch_add(limit, Ordering::SeqCst) + limit;
        assert!(
            in_flight + self.gauge.now.load(Ordering::SeqCst) <= self.concurrency.max(1),
            "lease requests in flight plus running handlers exceed the concurrency"
        );
        // A request takes a moment, so that concurrent callers overlap.
        sleep(Duration::from_millis(10)).await;
        let result = if let Some(e) = self.lease_error.lock().unwrap().clone() {
            Err(e)
        } else if self.mismatch.load(Ordering::SeqCst) {
            let (name, hash) = types.iter().next().unwrap();
            Ok(LeaseTurnsResult::ContractMismatch {
                type_name: name.clone(),
                registered: Some("other".into()),
                requested: hash.clone(),
            })
        } else {
            TurnSource::lease_turns(&self.server(), owner, types, limit).await
        };
        self.in_flight.fetch_sub(limit, Ordering::SeqCst);
        result
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        TurnSource::renew_turn(&self.server(), actor, epoch).await
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        TurnSource::release_turn(&self.server(), actor, epoch).await
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        TurnSource::complete_turn(&self.server(), actor, epoch, message_id, report).await
    }
}

fn probe(server: &ActorSystem, concurrency: usize) -> Probe {
    Probe { server: Mutex::new(Some(server.clone())), concurrency, ..Probe::default() }
}

/// A worker hosting the counter through `probe`.
fn worker(probe: &Arc<Probe>, concurrency: usize) -> Worker {
    Worker::builder(probe.clone()).host(CounterActor(probe.gauge.clone())).config(config(concurrency)).build()
}

/// Registers the counter's contract on `server` and creates the counters `ids`.
async fn setup(server: &ActorSystem, ids: &[&str]) {
    server.register_contract(&contract_of::<CounterActor>().unwrap()).await.unwrap();
    for id in ids {
        server.create(&counter(id), &CounterState::default()).await.unwrap();
    }
}

fn only_other(error: &StartError) -> &str {
    match error.problems.as_slice() {
        [StartProblem::Other(why)] => why,
        other => panic!("expected one problem, got {other:?}"),
    }
}

/// Scenario: Errors from the reference source.
#[test]
fn the_reference_source_classifies_invalid_unavailable_corrupt_and_start_errors() {
    let invalid = SourceError::from(OpError::Invalid("bad limit".into()));
    assert_eq!(invalid, SourceError::Invalid("bad limit".into()));
    let refused = SourceError::from(OpError::Store(StoreError::Invalid("out of range".into())));
    assert_eq!(refused, SourceError::Invalid("out of range".into()));
    assert!(matches!(SourceError::from(OpError::Store(StoreError::unavailable("down"))), SourceError::Unavailable(_)));
    assert!(matches!(SourceError::from(OpError::Store(StoreError::Corrupt("x".into()))), SourceError::Unavailable(_)));
    let start = StartError { problems: vec![StartProblem::Other("lease_ttl must be positive".into())] };
    match SourceError::from(OpError::Start(start)) {
        SourceError::Fatal(why) => assert!(why.contains("lease_ttl must be positive"), "{why}"),
        other => panic!("expected Fatal, got {other:?}"),
    }
}

/// Scenario: The in-process reference source.
#[tokio::test(start_paused = true)]
async fn the_reference_source_is_object_safe_and_lists_full_type_definitions() {
    let store = store();
    let server = server(&store);
    setup(&server, &[]).await;
    let source: Arc<dyn TurnSource> = Arc::new(server.clone());
    let types = source.list_types().await.unwrap();
    let contract = contract_of::<CounterActor>().unwrap();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].name.as_str(), "counter");
    assert_eq!(types[0].contract_hash, contract.hash());
    assert_eq!(types[0], server.types().await.unwrap()[0].definition);
    // A worker takes a shared, type-erased source as well.
    let worker = Worker::builder(source).host(CounterActor(Arc::default())).build();
    worker.initialize().await.unwrap();
    assert_eq!(worker.hosted_types().into_iter().map(|t| t.to_string()).collect::<Vec<_>>(), ["counter"]);
}

/// Scenario: The in-process reference source.
#[tokio::test(start_paused = true)]
async fn a_worker_registers_its_contract_leases_and_runs_the_unchanged_handler_through_a_server_that_hosts_nothing() {
    let store = store();
    let server = server(&store);
    let worker = Worker::builder(server.clone()).host(CounterActor(Arc::default())).config(config(4)).build();
    worker.initialize().await.unwrap();
    // The contract is registered through the source, exactly as generated.
    let registered = server.types().await.unwrap();
    assert_eq!(registered[0].definition.contract_hash, contract_of::<CounterActor>().unwrap().hash());
    server.create(&counter("a"), &CounterState::default()).await.unwrap();
    let first = send(&server, "a", CounterMsg::Add { n: 2 }).await;
    send(&server, "a", CounterMsg::Add { n: 3 }).await;
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 1);
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 1);
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 0);
    assert_eq!(total(&server, "a").await, 5);
    assert_eq!(outcome(&server, &first.id).await, Some(OutcomeStatus::Done));
    // Started, it polls by itself.
    worker.start().await.unwrap();
    send(&server, "a", CounterMsg::Add { n: 10 }).await;
    sleep(secs(1)).await;
    assert_eq!(total(&server, "a").await, 15);
    worker.stop().await;
    assert_eq!(worker.state(), NodeState::Stopped);
    assert_eq!(worker.stopped().await.map(|e| e.to_string()), None);
    // The worker never stops its source.
    assert_eq!(server.state(), NodeState::New);
}

/// Scenario: Conflicts and bad definitions.
#[tokio::test(start_paused = true)]
async fn a_conflicting_contract_fails_initialization_with_the_existing_conflict_fields() {
    let store = store();
    let server = server(&store);
    server.register_contract(&contract_of::<OtherActor>().unwrap()).await.unwrap();
    let worker = Worker::builder(server.clone()).host(CounterActor(Arc::default())).build();
    let error = worker.initialize().await.unwrap_err();
    match error.problems.as_slice() {
        [StartProblem::ContractConflict { type_name, registered_hash, node_hash, .. }] => {
            assert_eq!(type_name.as_str(), "counter");
            assert_eq!(registered_hash, contract_of::<OtherActor>().unwrap().hash());
            assert_eq!(node_hash, contract_of::<CounterActor>().unwrap().hash());
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert!(worker.start().await.is_err());
    assert_eq!(worker.running_turns(), 0);
}

/// A source that answers a registration with another hash than the one it was given.
struct Liar(ActorSystem);

#[async_trait]
impl TurnSource for Liar {
    async fn register_contract(&self, _contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        TurnSource::register_contract(&self.0, &contract_of::<OtherActor>().unwrap()).await
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        TurnSource::list_types(&self.0).await
    }

    async fn lease_turns(
        &self,
        _owner: &NodeId,
        _types: &BTreeMap<TypeName, String>,
        _limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        panic!("no lease without a matching registration")
    }

    async fn renew_turn(&self, _actor: &ActorAddress, _epoch: i64) -> Result<RenewTurnResult, SourceError> {
        unreachable!()
    }

    async fn release_turn(&self, _actor: &ActorAddress, _epoch: i64) -> Result<(), SourceError> {
        unreachable!()
    }

    async fn complete_turn(
        &self,
        _actor: &ActorAddress,
        _epoch: i64,
        _message_id: &MessageId,
        _report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        unreachable!()
    }
}

/// Scenario: Conflicts and bad definitions.
#[tokio::test(start_paused = true)]
async fn a_registration_answered_with_another_hash_is_a_contract_conflict() {
    let store = store();
    let worker = Worker::builder(Liar(server(&store))).host(CounterActor(Arc::default())).build();
    let error = worker.start().await.unwrap_err();
    match error.problems.as_slice() {
        [StartProblem::ContractConflict { registered_hash, node_hash, .. }] => {
            assert_eq!(registered_hash, contract_of::<OtherActor>().unwrap().hash());
            assert_eq!(node_hash, contract_of::<CounterActor>().unwrap().hash());
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
}

/// Scenario: Conflicts and bad definitions.
#[tokio::test(start_paused = true)]
async fn bad_settings_a_type_hosted_twice_and_more_than_100_types_fail_initialization() {
    let store = store();
    let server = server(&store);
    let zero = WorkerConfig { concurrency: 0, poll_interval: Duration::ZERO, ..config(1) };
    let error = Worker::builder(server.clone()).config(zero).build().initialize().await.unwrap_err();
    let problems: Vec<String> = error.problems.iter().map(|p| p.to_string()).collect();
    assert_eq!(problems, ["poll_interval must be positive", "concurrency must be at least 1"]);
    let nul = WorkerConfig { worker_id: NodeId::new("a\0b").unwrap(), ..config(1) };
    let error = Worker::builder(server.clone()).config(nul).build().initialize().await.unwrap_err();
    assert_eq!(only_other(&error), "worker_id must not contain a NUL character");
    let twice =
        Worker::builder(server.clone()).host(CounterActor(Arc::default())).host(CounterActor(Arc::default())).build();
    assert!(only_other(&twice.initialize().await.unwrap_err()).ends_with("type counter is hosted twice"));
    let mut many = Worker::builder(server.clone());
    for _ in 0..101 {
        many = many.host(CounterActor(Arc::default()));
    }
    let error = many.build().initialize().await.unwrap_err();
    assert_eq!(only_other(&error), "A worker hosts at most 100 types, got 101");
    assert!(server.types().await.unwrap().is_empty(), "nothing was registered");
}

/// Scenario: Initialization fails then recovers.
#[tokio::test(start_paused = true)]
async fn a_failed_initialization_can_be_retried_and_only_a_success_is_kept() {
    let store = store();
    let server = server(&store);
    let probe = Arc::new(probe(&server, 2));
    probe.failing_registrations.store(1, Ordering::SeqCst);
    let worker = worker(&probe, 2);
    let error = worker.initialize().await.unwrap_err();
    assert!(only_other(&error).contains("registering the contract failed"), "{error}");
    worker.initialize().await.unwrap();
    worker.initialize().await.unwrap();
    assert_eq!(probe.registrations.load(Ordering::SeqCst), 2, "a success is kept");
    worker.start().await.unwrap();
    worker.stop().await;
}

/// Scenario: Empty Worker.
#[tokio::test(start_paused = true)]
async fn an_empty_worker_starts_without_a_lease_request_and_stops() {
    let store = store();
    let server = server(&store);
    let probe = Arc::new(probe(&server, 2));
    let worker = Worker::builder(probe.clone()).config(config(2)).build();
    worker.start().await.unwrap();
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 0);
    sleep(secs(10)).await;
    worker.stop().await;
    assert_eq!(probe.leases.load(Ordering::SeqCst), 0);
    assert_eq!(probe.registrations.load(Ordering::SeqCst), 0);
    assert!(worker.stopped().await.is_none());
}

/// Scenario: Stop during registration.
#[tokio::test(start_paused = true)]
async fn a_stop_during_registration_marks_the_worker_stopped_and_a_late_registration_dispatches_nothing() {
    let store = store();
    let server = server(&store);
    setup(&server, &["a"]).await;
    send(&server, "a", CounterMsg::Add { n: 1 }).await;
    let gate = Arc::new(Notify::new());
    let probe = Arc::new(Probe { register_gate: Some(gate.clone()), ..probe(&server, 2) });
    let worker = worker(&probe, 2);
    let starting = tokio::spawn({
        let worker = worker.clone();
        async move { worker.start().await }
    });
    sleep(Duration::from_millis(1)).await;
    assert_eq!(probe.registrations.load(Ordering::SeqCst), 1, "start waits for the registration");
    worker.stop().await;
    assert_eq!(worker.state(), NodeState::Stopped);
    gate.notify_one();
    let error = starting.await.unwrap().unwrap_err();
    assert_eq!(only_other(&error), "Worker is STOPPED");
    sleep(secs(5)).await;
    assert_eq!(probe.leases.load(Ordering::SeqCst), 0, "nothing is leased after the stop");
    assert!(matches!(worker.run_due_turns(1).await, Err(WorkerError::Start(_))));
    assert_eq!(total(&server, "a").await, 0);
}

/// Scenario: A manual batch.
#[tokio::test(start_paused = true)]
async fn a_manual_batch_asks_for_at_most_its_limit_and_free_capacity_and_zero_asks_nothing() {
    let store = store();
    let server = server(&store);
    setup(&server, &["a", "b", "c"]).await;
    for id in ["a", "b", "c"] {
        send(&server, id, CounterMsg::Add { n: 1 }).await;
    }
    let probe = Arc::new(probe(&server, 2));
    let worker = worker(&probe, 2);
    assert_eq!(worker.run_due_turns(0).await.unwrap(), 0);
    assert_eq!(probe.leases.load(Ordering::SeqCst), 0, "a zero limit asks nothing");
    assert_eq!(worker.run_due_turns(500).await.unwrap(), 2, "capped by the concurrency");
    assert_eq!(worker.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(*probe.limits.lock().unwrap(), [2, 1]);
    worker.stop().await;
    assert!(matches!(worker.run_due_turns(1).await, Err(WorkerError::Start(_))));
    assert_eq!(probe.leases.load(Ordering::SeqCst), 2);
}

/// Scenario: Mixed manual and background work.
#[tokio::test(start_paused = true)]
async fn concurrent_manual_batches_and_polling_never_exceed_the_concurrency() {
    let store = store();
    let server = server(&store);
    let ids: Vec<String> = (0..12).map(|i| format!("c{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    setup(&server, &refs).await;
    for id in &refs {
        send(&server, id, CounterMsg::Slow { millis: 300 }).await;
    }
    let probe = Arc::new(probe(&server, 3));
    let worker = worker(&probe, 3);
    worker.start().await.unwrap();
    let manual: Vec<_> = (0..3)
        .map(|_| {
            let worker = worker.clone();
            tokio::spawn(async move { worker.run_due_turns(2).await })
        })
        .collect();
    for _ in 0..40 {
        assert!(worker.running_turns() <= 3);
        sleep(Duration::from_millis(50)).await;
    }
    for call in manual {
        call.await.unwrap().unwrap();
    }
    worker.stop().await;
    for id in &refs {
        assert_eq!(total(&server, id).await, 1, "{id} ran once");
    }
    assert!(probe.gauge.max.load(Ordering::SeqCst) <= 3);
}

/// Scenario: Polling cadence.
#[tokio::test(start_paused = true)]
async fn polling_repeats_at_once_after_a_full_batch_and_waits_the_interval_after_a_sparse_one() {
    let store = store();
    let server = server(&store);
    setup(&server, &["a", "b", "c"]).await;
    for id in ["a", "b", "c"] {
        send(&server, id, CounterMsg::Add { n: 1 }).await;
    }
    let probe = Arc::new(probe(&server, 1));
    let worker = worker(&probe, 1);
    worker.start().await.unwrap();
    // Each full batch of one is followed at once by the next request, until one comes back empty.
    sleep(Duration::from_millis(100)).await;
    for id in ["a", "b", "c"] {
        assert_eq!(total(&server, id).await, 1);
    }
    assert_eq!(probe.leases.load(Ordering::SeqCst), 4);
    // Empty rounds: one request per poll interval of 500 ms (each request takes 10 ms).
    sleep(Duration::from_millis(1000)).await;
    assert_eq!(probe.leases.load(Ordering::SeqCst), 6);
    worker.stop().await;
}

/// Scenario: A temporary manual leasing failure.
#[tokio::test(start_paused = true)]
async fn an_unavailable_source_is_returned_by_a_manual_batch_and_the_worker_stays_usable() {
    let store = store();
    let server = server(&store);
    setup(&server, &["a"]).await;
    send(&server, "a", CounterMsg::Add { n: 1 }).await;
    let probe = Arc::new(probe(&server, 2));
    let worker = worker(&probe, 2);
    *probe.lease_error.lock().unwrap() = Some(SourceError::Unavailable("down".into()));
    let error = worker.run_due_turns(5).await.unwrap_err();
    assert!(matches!(error, WorkerError::Source(SourceError::Unavailable(_))), "{error:?}");
    assert_eq!(worker.state(), NodeState::New);
    *probe.lease_error.lock().unwrap() = None;
    assert_eq!(worker.run_due_turns(5).await.unwrap(), 1);
    assert_eq!(total(&server, "a").await, 1);
}

/// Scenario: A permanent leasing failure.
#[tokio::test(start_paused = true)]
async fn a_permanent_lease_failure_stops_the_worker_and_every_waiter_gets_the_first_error() {
    for (label, invalid) in [("invalid", true), ("mismatch", false)] {
        let store = store();
        let server = server(&store);
        setup(&server, &["a"]).await;
        let probe = Arc::new(probe(&server, 2));
        if invalid {
            *probe.lease_error.lock().unwrap() = Some(SourceError::Invalid("bad owner".into()));
        } else {
            probe.mismatch.store(true, Ordering::SeqCst);
        }
        let worker = worker(&probe, 2);
        let waiters: Vec<_> = (0..2)
            .map(|_| {
                let worker = worker.clone();
                tokio::spawn(async move { worker.stopped().await })
            })
            .collect();
        worker.start().await.unwrap();
        for waiter in waiters {
            let error = waiter.await.unwrap().expect("an automatic stop reports its error");
            match (invalid, error) {
                (true, WorkerError::Source(SourceError::Invalid(why))) => assert_eq!(why, "bad owner"),
                (false, WorkerError::ContractMismatch { registered, .. }) => {
                    assert_eq!(registered.as_deref(), Some("other"))
                }
                (_, other) => panic!("{label}: unexpected {other:?}"),
            }
        }
        assert_eq!(worker.state(), NodeState::Stopped);
        let leases = probe.leases.load(Ordering::SeqCst);
        sleep(secs(5)).await;
        assert_eq!(probe.leases.load(Ordering::SeqCst), leases, "{label}: no request after the stop");
        assert_eq!(server.state(), NodeState::New, "{label}: the source is not stopped");
    }
}

/// Scenario: A permanent leasing failure.
#[tokio::test(start_paused = true)]
async fn a_permanent_failure_of_a_manual_batch_stops_a_new_worker_too() {
    let store = store();
    let server = server(&store);
    setup(&server, &["a"]).await;
    let probe = Arc::new(probe(&server, 2));
    *probe.lease_error.lock().unwrap() = Some(SourceError::Fatal("gone".into()));
    let worker = worker(&probe, 2);
    let error = worker.run_due_turns(1).await.unwrap_err();
    assert!(matches!(error, WorkerError::Source(SourceError::Fatal(_))));
    let stopped = worker.stopped().await.expect("the first error");
    assert_eq!(stopped.to_string(), "turn source failed: gone");
    assert_eq!(json!(worker.state().name()), json!("STOPPED"));
}

/// A worker's turns through a server on `store`, on real time: registration, manual batches and polling.
async fn works_on(store: Store) {
    let server = ActorSystem::builder(store.clone()).build();
    let worker = Worker::builder(server.clone()).host(CounterActor(Arc::default())).config(config(4)).build();
    worker.initialize().await.unwrap();
    for id in ["a", "b"] {
        server.create(&counter(id), &CounterState::default()).await.unwrap();
    }
    let first = send(&server, "a", CounterMsg::Add { n: 2 }).await;
    send(&server, "b", CounterMsg::Add { n: 3 }).await;
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 2);
    assert_eq!(outcome(&server, &first.id).await, Some(OutcomeStatus::Done));
    worker.start().await.unwrap();
    send(&server, "a", CounterMsg::Add { n: 5 }).await;
    let deadline = tokio::time::Instant::now() + secs(10);
    while total(&server, "a").await != 7 {
        assert!(tokio::time::Instant::now() < deadline, "the worker did not poll the turn");
        sleep(Duration::from_millis(20)).await;
    }
    worker.stop().await;
    assert_eq!(total(&server, "b").await, 3);
    store.close().await;
}

/// A worker on SQLite.
#[tokio::test]
async fn a_worker_runs_its_turns_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cicada.db");
    works_on(Store::sqlite(path, cicada::store::SqliteOptions::default()).await.unwrap()).await;
}

/// A worker on PostgreSQL (skipped without `CICADA_PG_URL`).
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_runs_its_turns_on_postgres() {
    let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
        return;
    };
    let schema = format!("worker_{}_{}", std::process::id(), &MessageId::random().as_str()[..8]);
    works_on(Store::postgres(&url, cicada::store::PostgresOptions::for_schema(&schema)).await.unwrap()).await;
    use sqlx::Connection;
    let mut c = sqlx::PgConnection::connect(&url).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA \"{schema}\" CASCADE"))).execute(&mut c).await.unwrap();
}
