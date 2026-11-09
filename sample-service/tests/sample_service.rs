//! The sample node over HTTP, on SQLite (and on PostgreSQL when `CICADA_PG_URL` is set).

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cicada::ActorRef;
use cicada::api::MessageState;
use cicada::prelude::*;
use cicada_client::ActorClient;
use cicada_examples::registry::DeviceRegistry;
use cicada_examples::saga::{Inventory, InventoryMsg, InventoryState, Order, OrderMsg, OrderState, OrderStatus};
use cicada_extras::{DeadLetters, DeadLettersState};
use cicada_sample_service::e2e::{
    Counter, CounterMsg, CounterState, EffectLog, Flaky, FlakyMsg, FlakyState, Recorder, RecorderMsg, RecorderState,
    Slow, SlowMsg, SlowState, flaky_retry,
};
use cicada_sample_service::{Catalog, GROUPS, SampleConfig, SampleNode, serve};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

fn vars(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn all_groups() -> Vec<String> {
    GROUPS.iter().map(|g| (*g).to_owned()).collect()
}

fn config(db: &std::path::Path) -> SampleConfig {
    SampleConfig {
        node_id: Some("test-node".into()),
        sqlite_path: db.to_path_buf(),
        poll_interval: Some(Duration::from_millis(50)),
        failure_watcher_interval: Duration::from_millis(100),
        ..SampleConfig::default()
    }
}

/// A sample node served on a random local port.
struct Node {
    base: String,
    http: reqwest::Client,
    api: ActorClient,
    stop: Option<oneshot::Sender<()>>,
    server: Option<JoinHandle<anyhow::Result<()>>>,
}

impl Node {
    async fn start(config: &SampleConfig) -> Self {
        let node = SampleNode::start(config).await.expect("the node starts");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (stop, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(serve(node, listener, async move {
            let _ = stopped.await;
        }));
        let http = reqwest::Client::new();
        let api = ActorClient::with_client(&base, authorized(config.api_token.as_deref()));
        Self { base, http, api, stop: Some(stop), server: Some(server) }
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.http.get(format!("{}{path}", self.base)).send().await.unwrap()
    }

    /// Stops the server and the node, as SIGTERM does.
    async fn stop(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.server.take().unwrap().await.unwrap().expect("served until shutdown");
    }
}

fn authorized(token: Option<&str>) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = token {
        headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    }
    reqwest::Client::builder().default_headers(headers).build().unwrap()
}

/// Runs `test` against a node on `config`, then stops the node.
async fn with_node(config: SampleConfig, test: impl AsyncFnOnce(&Node)) {
    let node = Node::start(&config).await;
    test(&node).await;
    node.stop().await;
}

async fn eventually<Fut: Future<Output = bool>>(what: &str, mut condition: impl FnMut() -> Fut) {
    let started = Instant::now();
    while !condition().await {
        assert!(started.elapsed() < Duration::from_secs(10), "Timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

struct TempDb {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn temp_db() -> TempDb {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("actors.db");
    TempDb { _dir: dir, path }
}

#[test]
fn configuration_comes_from_the_environment() {
    let config = SampleConfig::from_vars(&vars(&[
        ("PORT", "9000"),
        ("NODE_ID", "n1"),
        ("HOSTED_TYPES", "saga, e2e"),
        ("DATABASE_URL", "postgres://a:b@db/x"),
        ("LEASE_TTL", "3s"),
        ("POLL_INTERVAL", "PT0.1S"),
        ("CONCURRENCY", "4"),
        ("API_TOKEN", "secret"),
    ]))
    .unwrap();
    assert_eq!(config.port, 9000);
    assert_eq!(config.groups, ["saga", "e2e"]);
    assert_eq!(config.lease_ttl, Some(Duration::from_secs(3)));
    assert_eq!(config.poll_interval, Some(Duration::from_millis(100)));
    assert_eq!(config.concurrency, Some(4));
    let shown = format!("{config:?}");
    assert!(!shown.contains("secret") && !shown.contains("b@db"), "{shown}");
    assert_eq!(SampleConfig::from_vars(&vars(&[])).unwrap().groups, all_groups());
    assert!(SampleConfig::from_vars(&vars(&[("HOSTED_TYPES", "none")])).unwrap().groups.is_empty());
    let unknown = SampleConfig::from_vars(&vars(&[("HOSTED_TYPES", "saga,nope")])).unwrap_err();
    assert!(unknown.0.contains("unknown group(s) [nope]"), "{unknown}");
    let invalid = SampleConfig::from_vars(&vars(&[("LEASE_TTL", "soon")])).unwrap_err();
    assert_eq!(invalid.0, "LEASE_TTL: not a duration: soon");
    assert_eq!(SampleConfig::from_vars(&vars(&[("PORT", "x")])).unwrap_err().0, "PORT: not a number: x");
}

#[test]
fn the_postgres_pool_is_never_smaller_than_the_store_default_nor_than_the_concurrency_needs() {
    // The sample once always set 10, below the default concurrency of 16, starving heartbeats.
    let pool = |pairs: &[(&str, &str)]| SampleConfig::from_vars(&vars(pairs)).unwrap().effective_pool_size();
    assert_eq!(pool(&[]), None, "the store's default (20)");
    assert_eq!(pool(&[("CONCURRENCY", "4")]), Some(20));
    assert_eq!(pool(&[("CONCURRENCY", "40")]), Some(44));
    assert_eq!(pool(&[("CONCURRENCY", "40"), ("DATABASE_POOL_SIZE", "7")]), Some(7));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_groups_run_together_on_one_node_singletons_are_created_at_start() {
    let db = temp_db();
    with_node(config(&db.path), async |node| {
        let api = &node.api;
        assert!(api.health().await.unwrap());
        assert!(api.get(&DeviceRegistry).await.unwrap().is_some());
        assert!(api.get(&DeadLetters).await.unwrap().is_some());
        let expected = Catalog::new("x", EffectLog::new()).types(&all_groups()).len();
        assert_eq!(api.types().await.unwrap().len(), expected);
        let info = node.get("/").await.text().await.unwrap();
        assert!(info.contains(r#""node":"test-node""#), "{info}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_saga_runs_over_http() {
    let db = temp_db();
    with_node(config(&db.path), async |node| {
        let api = &node.api;
        let apple = Inventory { sku: "apple".into() };
        let order = Order { id: "o1".into() };
        api.create(&apple, &InventoryState::default()).await.unwrap();
        api.send(&apple, InventoryMsg::Restock { quantity: 5 }).await.unwrap();
        api.create(&order, &OrderState::default()).await.unwrap();
        api.send(&order, OrderMsg::PlaceOrder { sku: "apple".into(), quantity: 2, amount_cents: 1_000 }).await.unwrap();
        eventually("order completed", || async {
            api.state::<_, OrderState>(&order).await.unwrap().is_some_and(|s| s.status == OrderStatus::Completed)
        })
        .await;
        assert_eq!(api.state::<_, InventoryState>(&apple).await.unwrap().unwrap().available, 3);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_e2e_actors_record_node_attempt_and_effects() {
    let db = temp_db();
    with_node(config(&db.path), async |node| {
        let api = &node.api;
        let slow = Slow { id: "s".into() };
        let counter = Counter { id: "c".into() };
        let recorder = Recorder { id: "r".into() };
        api.create(&slow, &SlowState::default()).await.unwrap();
        let nap = api.send(&slow, SlowMsg::Sleep { millis: 100, label: Some("nap".into()) }).await.unwrap();
        api.create(&counter, &CounterState::default()).await.unwrap();
        for seq in 1..=20 {
            api.send(&counter, CounterMsg::Step { seq, hold_millis: 0 }).await.unwrap();
        }
        api.create(&recorder, &RecorderState::default()).await.unwrap();
        api.send(&recorder, RecorderMsg::Record { label: "effect-1".into(), sleep_millis: 0 }).await.unwrap();

        eventually("slow done", || async {
            api.status(&nap.id).await.unwrap().is_some_and(|s| s.status == MessageState::Done)
        })
        .await;
        let runs = api.state::<_, SlowState>(&slow).await.unwrap().unwrap().runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].node, "test-node");
        assert_eq!(runs[0].attempt, 1);
        assert_eq!(runs[0].label.as_deref(), Some("nap"));

        eventually("counter done", || async {
            api.state::<_, CounterState>(&counter).await.unwrap().is_some_and(|s| s.count == 20)
        })
        .await;
        let state = api.state::<_, CounterState>(&counter).await.unwrap().unwrap();
        assert_eq!(state.out_of_order, 0);
        assert_eq!(state.overlaps, 0);
        assert_eq!(state.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), (1..=20).collect::<Vec<_>>());

        eventually("recorded", || async {
            api.state::<_, RecorderState>(&recorder).await.unwrap().is_some_and(|s| s.records.len() == 1)
        })
        .await;
        let effects = node.get("/e2e/effects?actor=e2e.recorder/r").await.text().await.unwrap();
        assert!(effects.contains(r#""label":"effect-1""#) && effects.contains(r#""node":"test-node""#), "{effects}");
        let all = node.get("/e2e/effects").await.text().await.unwrap();
        assert!(all.contains(r#""label":"nap""#) && all.contains(r#""messageId""#), "{all}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flaky_messages_are_retried_and_final_failures_reach_the_dead_letters() {
    let db = temp_db();
    with_node(config(&db.path), async |node| {
        let api = &node.api;
        let flaky = Flaky { id: "f".into() };
        let retried = SendOptions::new().retry(flaky_retry());
        api.create(&flaky, &FlakyState::default()).await.unwrap();
        // Succeeds on the third attempt.
        api.send_with(&flaky, FlakyMsg::Attempt { times: 2 }, retried.clone()).await.unwrap();
        // Never succeeds.
        let failing = api.send_with(&flaky, FlakyMsg::Attempt { times: 5 }, retried).await.unwrap();
        // No policy: fails at its first error.
        let unretried = api.send(&flaky, FlakyMsg::Attempt { times: 1 }).await.unwrap();
        eventually("flaky succeeded", || async {
            api.state::<_, FlakyState>(&flaky).await.unwrap().is_some_and(|s| s.succeeded == 1)
        })
        .await;
        assert_eq!(api.state::<_, FlakyState>(&flaky).await.unwrap().unwrap().last_attempt, 3);

        // The failing message is retried in place: one id, pending with its failures, then failed once.
        eventually("failing message failed", || async {
            api.status(&failing.id).await.unwrap().is_some_and(|s| s.status == MessageState::Failed)
        })
        .await;
        let status = api.status(&failing.id).await.unwrap().unwrap();
        assert_eq!(status.failures, 2);
        assert_eq!(status.message.retry, Some(flaky_retry()));
        let status = api.status(&unretried.id).await.unwrap().unwrap();
        assert_eq!(status.status, MessageState::Failed);
        assert_eq!(status.failures, 0);
        assert_eq!(status.message.retry, None);

        eventually("dead letters recorded", || async {
            api.state::<_, DeadLettersState>(&DeadLetters)
                .await
                .unwrap()
                .is_some_and(|s| s.letters.contains_key(failing.id.as_str()))
        })
        .await;
        assert_eq!(api.state::<_, DeadLettersState>(&DeadLetters).await.unwrap().unwrap().letters.len(), 2);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_api_token_the_api_requires_the_bearer_token() {
    let db = temp_db();
    let config = SampleConfig { api_token: Some("s3cret".into()), ..config(&db.path) };
    with_node(config, async |node| {
        assert_eq!(node.get("/health").await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(node.get("/e2e/effects").await.status(), StatusCode::UNAUTHORIZED);
        let wrong = node.http.get(format!("{}/health", node.base)).bearer_auth("s3creT").send().await.unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        let right = node.http.get(format!("{}/health", node.base)).bearer_auth("s3cret").send().await.unwrap();
        assert_eq!(right.status(), StatusCode::OK);
        assert!(node.api.health().await.unwrap());
        assert_eq!(node.get("/").await.status(), StatusCode::OK); // node info stays public
        assert_eq!(node.get("/ready").await.status(), StatusCode::OK); // and readiness, for probes
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_only_node_accepts_sends_for_types_other_nodes_registered() {
    // A hosting node registers the types first (same SQLite file), then a node hosting nothing serves the API.
    let db = temp_db();
    let counter = Counter { id: "c".into() };
    let e2e_only = SampleConfig { groups: vec!["e2e".into()], ..config(&db.path) };
    with_node(e2e_only, async |node| {
        node.api.create(&counter, &CounterState::default()).await.unwrap();
    })
    .await;
    let send_only = SampleConfig { groups: Vec::new(), ..config(&db.path) };
    with_node(send_only, async |node| {
        let api = &node.api;
        let sent = api.send(&counter, CounterMsg::Step { seq: 1, hold_millis: 0 }).await.unwrap();
        // Nobody here runs it.
        assert_eq!(api.status(&sent.id).await.unwrap().map(|s| s.status), Some(MessageState::Pending));
        let address = counter.address().unwrap();
        let refused = api.send_message(&address, "Nope", serde_json::json!({}), SendOptions::new()).await;
        assert_eq!(refused.unwrap_err().status(), Some(422));
        assert!(api.status(&MessageId::new("unknown").unwrap()).await.unwrap().is_none());
        let info = node.get("/").await.text().await.unwrap();
        assert!(info.contains(r#""groups":[]"#) && info.contains(r#""types":[]"#), "{info}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_shutdown_the_server_stops_and_the_store_is_closed() {
    let db = temp_db();
    let node = SampleNode::start(&config(&db.path)).await.unwrap();
    let system = node.system().clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(node, listener, async move {
        let _ = stopped.await;
    }));
    assert_eq!(reqwest::get(format!("{base}/ready")).await.unwrap().status(), StatusCode::OK);
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
    assert!(!system.health().await, "the node is stopped and its store closed");
    assert!(reqwest::get(format!("{base}/ready")).await.is_err(), "the port is closed");
}

/// The node on PostgreSQL (`DATABASE_URL`), in a schema of its own. Skipped without `CICADA_PG_URL`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_node_runs_on_postgres() {
    let Ok(url) = std::env::var("CICADA_PG_URL") else {
        eprintln!("CICADA_PG_URL is not set; skipping");
        return;
    };
    let schema = format!("sample_{}", uuid_like());
    let config = SampleConfig {
        database_url: Some(url),
        database_schema: schema.clone(),
        concurrency: Some(4),
        ..config(std::path::Path::new("unused.db"))
    };
    with_node(config, async |node| {
        let api = &node.api;
        assert!(api.health().await.unwrap());
        assert!(api.get(&DeviceRegistry).await.unwrap().is_some());
        let info = node.get("/").await.text().await.unwrap();
        assert!(info.contains(&format!("postgres (schema {schema})")), "{info}");
        let counter = Counter { id: "pg".into() };
        api.create(&counter, &CounterState::default()).await.unwrap();
        for seq in 1..=5 {
            api.send(&counter, CounterMsg::Step { seq, hold_millis: 0 }).await.unwrap();
        }
        eventually("counter done", || async {
            api.state::<_, CounterState>(&counter).await.unwrap().is_some_and(|s| s.count == 5)
        })
        .await;
    })
    .await;
}

/// 12 hex digits from the clock, for a fresh schema name.
fn uuid_like() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    format!("{:012x}", nanos & 0xffff_ffff_ffff)
}
