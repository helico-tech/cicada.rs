//! A `Worker` over `ActorClient` runs unchanged Rust actors through two HTTP servers that share a store (spec
//! `remote-turn-http`, Scenario: Two-server and fault verification): it leases on one, renews and completes on the
//! other, keeps going when the first stops, takes an operator's cancel through a renewal, repeats a completion whose
//! answer was lost without running the handler again, keeps a rejected turn's failure report for good, abandons an
//! oversized completion and an unknown renewal answer without a release or a hidden report. A balancer in front of the
//! two routers routes each path and injects the faults. Real sockets on virtual time: every call of the worker is held
//! at a barrier that stops the paused clock while it is in flight, so the clock moves only between requests and the
//! deadline, renewal and fault assertions are exact. The SQLite and PostgreSQL peers are framing smoke tests on real
//! time, with no timing assertion.

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use cicada::actor::contract_of;
use cicada::address::TypeName;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::runtime::{
    CompleteTurnResult, LeaseTurnsResult, NodeConfig, RegisterContractResult, RenewTurnResult, SourceError, TurnReport,
    TurnSource, Worker, WorkerConfig,
};
use cicada::store::{MessageStatus, NodeId, OutcomeStatus, TypeDef};
use cicada::{FailureCause, Store};
use cicada_axum::{ApiConfig, actor_api, actor_api_with};
use cicada_client::ActorClient;
use cicada_test::{TokioClock, default_start};
use common::{eventually, serve};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use tokio::time::Instant;
use tower::ServiceExt;

// ---- an ordinary actor ----

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ledger {
    id: String,
}

impl ActorRef for Ledger {
    const TYPE: &'static str = "ledger";
    type Message = LedgerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum LedgerMsg {
    Add {
        n: i64,
        #[serde(default)]
        note: String,
    },
    /// Waits until the test opens the gate, then adds `n`.
    Hold { n: i64 },
    /// Sends `Add(n)` to the ledger `to`.
    Forward { to: String, n: i64 },
    /// Sends the ledger `to` an `Add` with a note of `bytes` characters.
    Big { to: String, bytes: usize },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
struct Balance {
    total: i64,
}

/// What the test sees of the handler: how often it ran, when a `Hold` started, and the gate it waits on.
#[derive(Clone)]
struct Probe {
    runs: Arc<AtomicUsize>,
    started: Arc<Notify>,
    gate: watch::Sender<bool>,
}

impl Default for Probe {
    fn default() -> Self {
        Self { runs: Arc::default(), started: Arc::default(), gate: watch::channel(false).0 }
    }
}

struct LedgerActor(Probe);

impl Actor for LedgerActor {
    type Ref = Ledger;
    type State = Balance;

    async fn handle(&self, turn: &mut Turn<Self>, message: LedgerMsg) -> HandlerResult {
        self.0.runs.fetch_add(1, Ordering::SeqCst);
        match message {
            LedgerMsg::Add { n, .. } => turn.state_mut().total += n,
            LedgerMsg::Hold { n } => {
                self.0.started.notify_one();
                let _ = self.0.gate.subscribe().wait_for(|open| *open).await;
                turn.state_mut().total += n;
            }
            LedgerMsg::Forward { to, n } => {
                turn.send(&Ledger { id: to }, LedgerMsg::Add { n, note: String::new() })?;
            }
            LedgerMsg::Big { to, bytes } => {
                turn.send(&Ledger { id: to }, LedgerMsg::Add { n: 1, note: "x".repeat(bytes) })?;
            }
        }
        Ok(())
    }
}

// ---- the balancer ----

/// Routes `/leases` and `/types` to the first server while it is up, everything else to the second, and injects
/// faults; records each request.
struct Balancer {
    first: Router,
    second: Router,
    /// Where `/leases/complete` goes instead of the second server, e.g. one with a small body limit.
    complete: Mutex<Option<Router>>,
    first_up: AtomicBool,
    /// The requests, by path and number from 1, whose answer is lost after the server handled them (the worker gets a
    /// `502`).
    lose: Mutex<Vec<(String, usize)>>,
    /// Renewals are answered with a result this version does not know.
    unknown_renewal: AtomicBool,
    requests: Mutex<Vec<(String, Value)>>,
}

impl Balancer {
    fn count(&self, path: &str) -> usize {
        self.requests.lock().unwrap().iter().filter(|(p, _)| p == path).count()
    }

    fn bodies(&self, path: &str) -> Vec<Value> {
        self.requests.lock().unwrap().iter().filter(|(p, _)| p == path).map(|(_, b)| b.clone()).collect()
    }
}

async fn balance(State(balancer): State<Arc<Balancer>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path().to_owned();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    let recorded = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    balancer.requests.lock().unwrap().push((path.clone(), recorded));
    let number = balancer.count(&path);
    let to_first = balancer.first_up.load(Ordering::SeqCst) && (path == "/leases" || path.starts_with("/types"));
    let router = match (to_first, path.as_str()) {
        (true, _) => balancer.first.clone(),
        (false, "/leases/complete") => balancer.complete.lock().unwrap().clone().unwrap_or(balancer.second.clone()),
        (false, _) => balancer.second.clone(),
    };
    let forwarded = Request::from_parts(parts, Body::from(bytes));
    let response = router.oneshot(forwarded).await.unwrap();
    if balancer.lose.lock().unwrap().contains(&(path.clone(), number)) {
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        return StatusCode::BAD_GATEWAY.into_response();
    }
    if path == "/leases/renew" && balancer.unknown_renewal.load(Ordering::SeqCst) {
        return (StatusCode::OK, [("content-type", "application/json")], r#"{"result":"extended"}"#).into_response();
    }
    response
}

// ---- the barrier ----

/// Holds the paused clock while a call is in flight: a blocking task stops tokio's auto-advance on a current-thread
/// runtime until the call ends (`spawn_blocking` inhibits it while the task runs). So virtual time moves only while no
/// request is on a socket, and a request never meets a timer that fired while it travelled.
struct InFlight(#[allow(dead_code)] std::sync::mpsc::Sender<()>);

fn in_flight() -> InFlight {
    let (held, released) = std::sync::mpsc::channel::<()>();
    tokio::task::spawn_blocking(move || {
        let _ = released.recv();
    });
    InFlight(held)
}

/// `ActorClient` with every call held at the barrier of [`in_flight`].
struct Barrier(ActorClient);

#[async_trait]
impl TurnSource for Barrier {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        let _held = in_flight();
        self.0.register_contract(contract).await
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        let _held = in_flight();
        self.0.list_types().await
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        let _held = in_flight();
        self.0.lease_turns(owner, types, limit).await
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        let _held = in_flight();
        self.0.renew_turn(actor, epoch).await
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        let _held = in_flight();
        self.0.release_turn(actor, epoch).await
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        let _held = in_flight();
        self.0.complete_turn(actor, epoch, message_id, report).await
    }
}

// ---- the setup ----

/// Two servers on `stores` (the first with a 3 s TTL, the second with `peer_ttl`), neither hosting anything, behind a
/// balancer, and a worker of the ledger type over an explicitly configured client of the balancer, held at the
/// barrier.
struct Setup {
    first: ActorSystem,
    second: ActorSystem,
    balancer: Arc<Balancer>,
    worker: Worker,
    probe: Probe,
}

async fn setup(first_store: Store, second_store: Store, peer_ttl: Duration) -> Setup {
    let first = ActorSystem::builder(first_store)
        .config(NodeConfig { lease_ttl: Duration::from_secs(3), ..NodeConfig::default() })
        .build();
    let second =
        ActorSystem::builder(second_store).config(NodeConfig { lease_ttl: peer_ttl, ..NodeConfig::default() }).build();
    first.register_contract(&contract_of::<LedgerActor>().unwrap()).await.unwrap();
    let balancer = Arc::new(Balancer {
        first: actor_api(first.clone()),
        second: actor_api(second.clone()),
        complete: Mutex::new(None),
        first_up: AtomicBool::new(true),
        lose: Mutex::new(Vec::new()),
        unknown_renewal: AtomicBool::new(false),
        requests: Mutex::new(Vec::new()),
    });
    let base = serve(Router::new().fallback(balance).with_state(balancer.clone())).await;
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let source = ActorClient::with_client(base, http).turn_timeout(Duration::from_secs(5));
    let probe = Probe::default();
    let worker = Worker::builder(Barrier(source))
        .host(LedgerActor(probe.clone()))
        .config(WorkerConfig {
            poll_interval: Duration::from_millis(20),
            shutdown_grace_period: Duration::from_secs(1),
            ..WorkerConfig::default()
        })
        .build();
    worker.start().await.expect("the worker starts over HTTP");
    Setup { first, second, balancer, worker, probe }
}

/// An in-memory store on tokio's clock, shared by both servers: store time is virtual time.
fn virtual_store() -> Store {
    Store::memory_on(Arc::new(TokioClock::new(default_start())))
}

async fn ledger(system: &ActorSystem, id: &str) {
    system.create(&Ledger { id: id.into() }, &Balance::default()).await.unwrap();
}

async fn send(system: &ActorSystem, id: &str, message: LedgerMsg) -> MessageId {
    match system.send(&Ledger { id: id.into() }, message).await.unwrap() {
        cicada::SendResult::Accepted(m) => m.id,
        other => panic!("not accepted: {other:?}"),
    }
}

async fn total(system: &ActorSystem, id: &str) -> i64 {
    system.get_actor(&format!("ledger/{id}").parse().unwrap()).await.unwrap().unwrap().state["total"].as_i64().unwrap()
}

async fn finished(system: &ActorSystem, id: &MessageId) -> Option<(OutcomeStatus, Option<String>)> {
    match system.status(id).await.unwrap()? {
        MessageStatus::Finished(o) => Some((o.status, o.error)),
        _ => None,
    }
}

async fn ended(system: &ActorSystem, id: &MessageId) -> (OutcomeStatus, Option<String>) {
    eventually(async || finished(system, id).await.is_some()).await;
    finished(system, id).await.unwrap()
}

async fn running(system: &ActorSystem, id: &MessageId) -> bool {
    matches!(system.status(id).await.unwrap(), Some(MessageStatus::Running { .. }))
}

// ---- on virtual time ----

/// Lease on the first server, renew and complete on the second, with the first stopped mid-turn: the turn is held for
/// longer than both TTLs on renewals through the second server alone, with no sticky session and no second run.
#[tokio::test(start_paused = true)]
async fn a_turn_leased_on_one_server_is_renewed_and_completed_through_another_after_it_stopped() {
    let store = virtual_store();
    let s = setup(store.clone(), store, Duration::from_secs(2)).await;
    ledger(&s.first, "a").await;
    let held = send(&s.first, "a", LedgerMsg::Hold { n: 5 }).await;
    s.probe.started.notified().await;
    let leased = Instant::now();
    s.first.stop().await;
    s.balancer.first_up.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(leased.elapsed(), Duration::from_secs(4), "virtual time");
    assert!(running(&s.second, &held).await, "still held, 4 s after a lease of 3 s");
    assert_eq!(s.worker.running_turns(), 1);
    // A first renewal after a third of the first TTL (1 s), then every third of the second's (2 s): five by 4 s.
    assert!(s.balancer.count("/leases/renew") >= 5, "renewed {} times", s.balancer.count("/leases/renew"));
    s.probe.gate.send_replace(true);
    assert_eq!(ended(&s.second, &held).await.0, OutcomeStatus::Done);
    assert_eq!(total(&s.second, "a").await, 5);
    assert_eq!((s.balancer.count("/leases/complete"), s.balancer.count("/leases/release")), (1, 0));
    // The worker goes on leasing through the second server.
    let next = send(&s.second, "a", LedgerMsg::Add { n: 1, note: String::new() }).await;
    assert_eq!(ended(&s.second, &next).await.0, OutcomeStatus::Done);
    assert_eq!(s.probe.runs.load(Ordering::SeqCst), 2, "no handler ran twice");
    s.worker.stop().await;
    assert!(s.worker.stopped().await.is_none());
}

/// An operator's cancel reaches the worker through the next renewal, on the other server, a third of the TTL after the
/// lease; the turn ends `cancelled`.
#[tokio::test(start_paused = true)]
async fn an_operator_cancel_reaches_the_worker_through_a_renewal() {
    let store = virtual_store();
    let s = setup(store.clone(), store, Duration::from_secs(2)).await;
    ledger(&s.first, "c").await;
    let held = send(&s.first, "c", LedgerMsg::Hold { n: 1 }).await;
    s.probe.started.notified().await;
    let leased = Instant::now();
    assert!(s.second.cancel_turn(&"ledger/c".parse().unwrap()).await.unwrap());
    tokio::time::sleep(Duration::from_millis(990)).await;
    assert!(running(&s.first, &held).await, "no renewal yet");
    assert_eq!(s.balancer.count("/leases/renew"), 0);
    assert_eq!(ended(&s.first, &held).await.0, OutcomeStatus::Cancelled);
    assert_eq!(s.balancer.count("/leases/renew"), 1, "the first renewal brought the cancel");
    assert!(leased.elapsed() < Duration::from_millis(1_100), "{:?}", leased.elapsed());
    assert_eq!(s.balancer.bodies("/leases/complete")[0]["report"], json!({"kind": "cancelled"}));
    assert_eq!(total(&s.first, "c").await, 0);
    assert_eq!(s.balancer.count("/leases/release"), 0);
    s.worker.stop().await;
}

/// A completion that applied but whose answer was lost is repeated with the same report: one set of effects, no
/// second run of the handler, no release.
#[tokio::test(start_paused = true)]
async fn a_lost_completion_answer_is_repeated_and_applies_once() {
    let store = virtual_store();
    let s = setup(store.clone(), store, Duration::from_secs(10)).await;
    ledger(&s.first, "from").await;
    ledger(&s.first, "to").await;
    s.balancer.lose.lock().unwrap().push(("/leases/complete".into(), 1));
    let forward = send(&s.first, "from", LedgerMsg::Forward { to: "to".into(), n: 7 }).await;
    assert_eq!(ended(&s.first, &forward).await.0, OutcomeStatus::Done);
    eventually(async || total(&s.first, "to").await == 7).await;
    let of_forward = || -> Vec<Value> {
        s.balancer.bodies("/leases/complete").into_iter().filter(|b| b["actor"] == "ledger/from").collect()
    };
    eventually(async || of_forward().len() >= 2).await;
    // Well past the completion budget (one TTL): nothing more comes.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let reports = of_forward();
    assert_eq!(reports.len(), 2, "repeated once, then answered leaseLost");
    assert_eq!(reports[0], reports[1], "the same report again, with the same send id");
    assert_eq!(total(&s.first, "to").await, 7, "the send applied once");
    assert_eq!(s.probe.runs.load(Ordering::SeqCst), 2, "the forward ran once, and the add once");
    assert_eq!(s.balancer.count("/leases/release"), 0);
    s.worker.stop().await;
}

/// A rejected commit switches the worker to the failure report for good: when that report's answer is lost, it repeats
/// the failure, never the original effects, and releases nothing.
#[tokio::test(start_paused = true)]
async fn a_rejected_completion_is_followed_by_the_failure_report_for_good() {
    let store = virtual_store();
    let s = setup(store.clone(), store, Duration::from_secs(10)).await;
    ledger(&s.first, "from").await;
    // The first completion is rejected (its send goes to a ledger that does not exist); the second, the failure report,
    // applies and its answer is lost.
    s.balancer.lose.lock().unwrap().push(("/leases/complete".into(), 2));
    let forward = send(&s.first, "from", LedgerMsg::Forward { to: "absent".into(), n: 1 }).await;
    let (status, error) = ended(&s.first, &forward).await;
    assert_eq!(status, OutcomeStatus::Failed);
    assert!(error.is_some());
    eventually(async || s.balancer.count("/leases/complete") >= 3).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let reports = s.balancer.bodies("/leases/complete");
    assert_eq!(reports.len(), 3, "rejected, lost, then answered leaseLost");
    assert_eq!(reports[0]["report"]["kind"], "returned");
    for later in &reports[1..] {
        assert_eq!(later["report"]["kind"], "failed", "never the original effects again: {later}");
        assert_eq!(later["report"]["cause"], FailureCause::Commit.name());
        assert_eq!(later["report"], reports[1]["report"]);
    }
    assert_eq!(s.balancer.count("/leases/release"), 0);
    assert_eq!(s.probe.runs.load(Ordering::SeqCst), 1);
    s.worker.stop().await;
}

/// A completion larger than the server's body limit is `413`: the worker abandons the turn without a release and
/// without a failure report of its own; the lease expires after its TTL, and lost-claim recovery runs the handler
/// again.
#[tokio::test(start_paused = true)]
async fn an_oversized_completion_is_abandoned_without_release_or_a_hidden_failure_report() {
    let store = virtual_store();
    let s = setup(store.clone(), store, Duration::from_secs(10)).await;
    *s.balancer.complete.lock().unwrap() =
        Some(actor_api_with(s.second.clone(), ApiConfig { max_request_body_bytes: 2_048 }));
    ledger(&s.first, "from").await;
    ledger(&s.first, "to").await;
    let big = send(&s.first, "from", LedgerMsg::Big { to: "to".into(), bytes: 4_000 }).await;
    eventually(async || s.balancer.count("/leases/complete") >= 1).await;
    let refused = Instant::now();
    tokio::time::sleep(Duration::from_millis(2_900)).await;
    assert_eq!(s.balancer.count("/leases/complete"), 1, "no hidden replacement report");
    assert_eq!(s.balancer.count("/leases/release"), 0);
    assert_eq!(s.balancer.count("/leases/renew"), 0, "abandoned: not renewed");
    assert!(running(&s.first, &big).await, "left to expire");
    assert_eq!(total(&s.first, "to").await, 0);
    // The 3 s lease expires; the next claim counts a lost claim and runs the handler again.
    eventually(async || s.balancer.count("/leases/complete") >= 2).await;
    assert!(refused.elapsed() >= Duration::from_millis(2_900), "{:?}", refused.elapsed());
    assert_eq!(s.probe.runs.load(Ordering::SeqCst), 2);
    assert_eq!(s.balancer.count("/leases/release"), 0);
    s.worker.stop().await;
}

/// A renewal answered with a result this version does not know is no renewal: the turn is abandoned at its local
/// deadline (the request time plus the first server's TTL of 3 s, less 0.3 s), without a completion or a release.
#[tokio::test(start_paused = true)]
async fn an_unknown_renewal_answer_is_no_renewal_and_the_turn_is_abandoned_at_its_deadline() {
    let store = virtual_store();
    let s = setup(store.clone(), store, Duration::from_secs(3)).await;
    ledger(&s.first, "u").await;
    s.balancer.unknown_renewal.store(true, Ordering::SeqCst);
    send(&s.first, "u", LedgerMsg::Hold { n: 1 }).await;
    s.probe.started.notified().await;
    tokio::time::sleep(Duration::from_millis(2_650)).await;
    assert!(s.balancer.count("/leases/renew") >= 1);
    assert_eq!(s.worker.running_turns(), 1, "before its deadline");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(s.worker.running_turns(), 0, "abandoned at its deadline, its handler still waiting");
    assert_eq!((s.balancer.count("/leases/complete"), s.balancer.count("/leases/release")), (0, 0));
    s.worker.stop().await;
}

// ---- framing smoke tests on SQL stores: real time, no timing assertions ----

/// Lease on the first server, complete through the second after the first stopped, each server with a store handle of
/// its own.
async fn completed_through_the_second_after_the_first_stopped(first_store: Store, second_store: Store) {
    let s = setup(first_store, second_store, Duration::from_secs(30)).await;
    ledger(&s.first, "a").await;
    let held = send(&s.first, "a", LedgerMsg::Hold { n: 5 }).await;
    s.probe.started.notified().await;
    s.first.stop().await;
    s.balancer.first_up.store(false, Ordering::SeqCst);
    s.probe.gate.send_replace(true);
    assert_eq!(ended(&s.second, &held).await.0, OutcomeStatus::Done);
    assert_eq!(total(&s.second, "a").await, 5);
    assert_eq!((s.balancer.count("/leases/complete"), s.balancer.count("/leases/release")), (1, 0));
    assert_eq!(s.probe.runs.load(Ordering::SeqCst), 1);
    s.worker.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_sqlite_a_turn_leased_on_one_server_completes_through_another() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.db");
    let options = cicada::store::SqliteOptions::default;
    let first = Store::sqlite(&path, options()).await.unwrap();
    let second = Store::sqlite(&path, options()).await.unwrap();
    completed_through_the_second_after_the_first_stopped(first, second).await;
}

/// The same on PostgreSQL (skipped without `CICADA_PG_URL`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_postgres_a_turn_leased_on_one_server_completes_through_another() {
    let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
        return;
    };
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let schema = format!("peers_{:012x}", nanos & 0xffff_ffff_ffff);
    let options = || cicada::store::PostgresOptions::for_schema(&schema);
    let first = Store::postgres(&url, options()).await.unwrap();
    let second = Store::postgres(&url, options()).await.unwrap();
    completed_through_the_second_after_the_first_stopped(first, second).await;
}
