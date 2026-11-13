//! A `Worker` over `ActorClient` runs unchanged Rust actors through two HTTP servers that share a store (spec
//! `remote-turn-http`, Scenario: Two-server and fault verification): it leases on one, renews and completes on the
//! other, keeps going when the first stops, takes an operator's cancel through a renewal, repeats a completion whose
//! answer was lost without running the handler again, keeps a rejected turn's failure report for good, abandons an
//! oversized completion and an unknown renewal answer without a release or a hidden report. A balancer in front of the
//! two routers routes each path and injects the faults. Real sockets and real time: the assertions are about what
//! happened, not when (the worker's deadlines are tested on virtual time in the core).

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use cicada::actor::contract_of;
use cicada::prelude::*;
use cicada::runtime::{NodeConfig, Worker, WorkerConfig};
use cicada::store::{MessageStatus, OutcomeStatus};
use cicada::{FailureCause, Store};
use cicada_axum::{ApiConfig, actor_api, actor_api_with};
use cicada_client::ActorClient;
use common::{eventually, serve};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
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

/// Two servers on `stores` (the first with a 3 s TTL, the second with `peer_ttl`), neither hosting anything, behind a
/// balancer, and a worker of the ledger type over an explicitly configured client of the balancer.
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
    let worker = Worker::builder(source)
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

async fn lease_held_through_the_second_after_the_first_stopped(first_store: Store, second_store: Store) {
    let s = setup(first_store, second_store, Duration::from_secs(2)).await;
    ledger(&s.first, "a").await;
    let held = send(&s.first, "a", LedgerMsg::Hold { n: 5 }).await;
    s.probe.started.notified().await;
    s.first.stop().await;
    s.balancer.first_up.store(false, Ordering::SeqCst);
    // Held for longer than both TTLs: only the renewals through the second server keep the lease.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(running(&s.second, &held).await, "still held");
    s.probe.gate.send_replace(true);
    assert_eq!(ended(&s.second, &held).await.0, OutcomeStatus::Done);
    assert_eq!(total(&s.second, "a").await, 5);
    assert!(s.balancer.count("/leases/renew") >= 2, "renewed through the second server");
    assert_eq!((s.balancer.count("/leases/complete"), s.balancer.count("/leases/release")), (1, 0));
    // The worker goes on leasing through the second server.
    let next = send(&s.second, "a", LedgerMsg::Add { n: 1, note: String::new() }).await;
    assert_eq!(ended(&s.second, &next).await.0, OutcomeStatus::Done);
    assert_eq!(s.probe.runs.load(Ordering::SeqCst), 2, "no handler ran twice");
    s.worker.stop().await;
    assert!(s.worker.stopped().await.is_none());
}

/// Lease on the first server, renew and complete on the second, with the first stopped mid-turn: no sticky session,
/// no second run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_leased_on_one_server_is_renewed_and_completed_through_another_after_it_stopped() {
    let store = Store::memory();
    lease_held_through_the_second_after_the_first_stopped(store.clone(), store).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_on_sqlite_with_a_store_handle_per_server() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.db");
    let options = cicada::store::SqliteOptions::default;
    let first = Store::sqlite(&path, options()).await.unwrap();
    let second = Store::sqlite(&path, options()).await.unwrap();
    lease_held_through_the_second_after_the_first_stopped(first, second).await;
}

/// The same on PostgreSQL (skipped without `CICADA_PG_URL`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_on_postgres_with_a_store_handle_per_server() {
    let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
        return;
    };
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let schema = format!("peers_{:012x}", nanos & 0xffff_ffff_ffff);
    let options = || cicada::store::PostgresOptions::for_schema(&schema);
    let first = Store::postgres(&url, options()).await.unwrap();
    let second = Store::postgres(&url, options()).await.unwrap();
    lease_held_through_the_second_after_the_first_stopped(first, second).await;
}

/// An operator's cancel reaches the worker through a renewal on the other server; the turn ends `cancelled`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_operator_cancel_reaches_the_worker_through_a_renewal() {
    let store = Store::memory();
    let s = setup(store.clone(), store, Duration::from_secs(2)).await;
    ledger(&s.first, "c").await;
    let held = send(&s.first, "c", LedgerMsg::Hold { n: 1 }).await;
    s.probe.started.notified().await;
    assert!(s.second.cancel_turn(&"ledger/c".parse().unwrap()).await.unwrap());
    assert_eq!(ended(&s.first, &held).await.0, OutcomeStatus::Cancelled);
    assert_eq!(s.balancer.bodies("/leases/complete")[0]["report"], json!({"kind": "cancelled"}));
    assert_eq!(total(&s.first, "c").await, 0);
    assert_eq!(s.balancer.count("/leases/release"), 0);
    s.worker.stop().await;
}

/// A completion that applied but whose answer was lost is repeated with the same report: one set of effects, no
/// second run of the handler, no release.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_completion_answer_is_repeated_and_applies_once() {
    let store = Store::memory();
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
    tokio::time::sleep(Duration::from_millis(300)).await;
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
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_completion_is_followed_by_the_failure_report_for_good() {
    let store = Store::memory();
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
    tokio::time::sleep(Duration::from_millis(300)).await;
    let reports = s.balancer.bodies("/leases/complete");
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
/// without a failure report of its own; the lease is left to expire.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversized_completion_is_abandoned_without_release_or_a_hidden_failure_report() {
    let store = Store::memory();
    let s = setup(store.clone(), store, Duration::from_secs(10)).await;
    *s.balancer.complete.lock().unwrap() =
        Some(actor_api_with(s.second.clone(), ApiConfig { max_request_body_bytes: 2_048 }));
    ledger(&s.first, "from").await;
    ledger(&s.first, "to").await;
    let big = send(&s.first, "from", LedgerMsg::Big { to: "to".into(), bytes: 4_000 }).await;
    eventually(async || s.balancer.count("/leases/complete") >= 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(s.balancer.count("/leases/complete"), 1, "no hidden replacement report");
    assert_eq!(s.balancer.count("/leases/release"), 0);
    assert!(running(&s.first, &big).await, "left to expire");
    assert_eq!(total(&s.first, "to").await, 0);
    s.worker.stop().await;
}

/// A renewal answered with a result this version does not know is no renewal: the turn is abandoned at its local
/// deadline (the first server's TTL of 3 s, less 0.3 s), without a completion or a release.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_renewal_answer_is_no_renewal_and_the_turn_is_abandoned_at_its_deadline() {
    let store = Store::memory();
    let s = setup(store.clone(), store, Duration::from_secs(3)).await;
    ledger(&s.first, "u").await;
    s.balancer.unknown_renewal.store(true, Ordering::SeqCst);
    send(&s.first, "u", LedgerMsg::Hold { n: 1 }).await;
    s.probe.started.notified().await;
    assert_eq!(s.worker.running_turns(), 1);
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    assert!(s.balancer.count("/leases/renew") >= 1);
    assert_eq!(s.worker.running_turns(), 0, "abandoned, its handler still waiting");
    assert_eq!((s.balancer.count("/leases/complete"), s.balancer.count("/leases/release")), (0, 0));
    s.worker.stop().await;
}
