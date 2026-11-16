//! The sample node over HTTP, run as `App::run_with` runs it, on SQLite (and on PostgreSQL when `CICADA_PG_URL` is
//! set); and the binary itself, stopped with SIGTERM.

use std::ffi::OsString;
use std::future::Future;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cicada::ActorRef;
use cicada::api::MessageState;
use cicada::prelude::*;
use cicada_actors::{DeadLetters, DeadLettersState};
use cicada_app::{AppError, Env};
use cicada_client::ActorClient;
use cicada_examples::registry::DeviceRegistry;
use cicada_examples::saga::{Inventory, InventoryMsg, InventoryState, Order, OrderMsg, OrderState, OrderStatus};
use cicada_sample_service::e2e::{
    Counter, CounterMsg, CounterState, EffectLog, Flaky, FlakyMsg, FlakyState, Recorder, RecorderMsg, RecorderState,
    Slow, SlowMsg, SlowState, flaky_retry,
};
use cicada_sample_service::{Catalog, OLD_VARIABLES, Sample, SampleConfig, refuse_old_variables};
use reqwest::StatusCode;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

fn vars(pairs: &[(&str, &str)]) -> Env {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// The environment of a node `test-node` on the SQLite file `db`.
fn env(db: &std::path::Path) -> Env {
    vars(&[
        ("CICADA_STORE", &db.display().to_string()),
        ("CICADA_NODE_ID", "test-node"),
        ("CICADA_POLL_INTERVAL", "50ms"),
    ])
}

/// An output that the test reads while the node writes to it.
#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Out {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

/// A sample node served on a random local port.
struct Node {
    base: String,
    http: reqwest::Client,
    api: ActorClient,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), AppError>>>,
}

impl Node {
    /// `serve --listen 127.0.0.1:0 <flags>` with `env`.
    async fn start(env: &Env, flags: &[&str]) -> Self {
        let mut line = vec!["cicada-sample-service", "serve", "--listen", "127.0.0.1:0"];
        line.extend(flags);
        let line = args(&line);
        let Sample { app } = Sample::new(&line, env).expect("the sample's variables");
        let out = Out::default();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(app.run_with(line, env.clone(), out.clone(), async move {
            let _ = stopped.await;
        }));
        let started = Instant::now();
        let base = loop {
            if let Some(l) = out.text().lines().find(|l| l.starts_with("Listening on ")) {
                break l.trim_start_matches("Listening on ").to_owned();
            }
            if task.is_finished() {
                panic!("the node ended: {:?}", task.await.unwrap());
            }
            assert!(started.elapsed() < Duration::from_secs(20), "the node did not listen");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let token = env.get("API_TOKEN").map(String::as_str);
        let api = ActorClient::with_client(&base, authorized(token));
        Self { base, http: reqwest::Client::new(), api, stop: Some(stop), task: Some(task) }
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.http.get(format!("{}{path}", self.base)).send().await.unwrap()
    }

    /// The stored watches as `(target, watcher)`, over HTTP.
    async fn watches(&self) -> Vec<(String, String)> {
        let page: serde_json::Value = self.get("/watches").await.json().await.unwrap();
        let text = |w: &serde_json::Value, key: &str| w[key].as_str().unwrap().to_owned();
        page["items"].as_array().unwrap().iter().map(|w| (text(w, "target"), text(w, "watcher"))).collect()
    }

    /// Stops the node, as SIGTERM does.
    async fn stop(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.task.take().unwrap().await.unwrap().expect("served until the stop");
    }
}

fn authorized(token: Option<&str>) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = token {
        headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    }
    reqwest::Client::builder().default_headers(headers).build().unwrap()
}

/// Runs `test` against a node on `env`, then stops the node.
async fn with_node(env: Env, flags: &[&str], test: impl AsyncFnOnce(&Node)) {
    let node = Node::start(&env, flags).await;
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
fn the_sample_reads_its_own_variables() {
    let config =
        SampleConfig::from_vars(&vars(&[("API_TOKEN", "secret"), ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://otel:4318")]))
            .unwrap();
    assert_eq!(config.api_token.as_deref(), Some("secret"));
    assert!(config.telemetry);
    assert!(!format!("{config:?}").contains("secret"));
    assert_eq!(SampleConfig::from_vars(&vars(&[])).unwrap(), SampleConfig::default());
}

/// An old variable stops the start of a node, and the error names the new one.
#[test]
fn an_old_variable_stops_the_start_and_names_the_new_one() {
    for (old, new) in OLD_VARIABLES {
        let error = Sample::new(&args(&["sample", "serve"]), &vars(&[(old, "x")])).unwrap_err();
        assert!(error.0.contains(&format!("{old} (use {new})")), "{old}: {error}");
    }
    let error = refuse_old_variables(&vars(&[("DATABASE_URL", "postgres://u:secret@db/x"), ("HOSTED_TYPES", "none")]));
    let error = error.unwrap_err().0;
    assert!(error.contains("DATABASE_URL (use CICADA_STORE)") && error.contains("HOSTED_TYPES (use CICADA_TYPES)"));
    assert!(!error.contains("secret"), "{error}");
    assert!(Sample::new(&args(&["sample", "worker"]), &vars(&[("PORT", "8080")])).is_err());
    assert!(Sample::new(&args(&["sample", "get", "actors"]), &vars(&[("PORT", "8080")])).is_ok(), "not a node");
    assert!(refuse_old_variables(&vars(&[("PORT", " ")])).is_ok(), "a blank variable is unset");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_type_runs_on_one_node_and_the_singletons_are_created_at_start() {
    let db = temp_db();
    with_node(env(&db.path), &[], async |node| {
        let api = &node.api;
        assert!(api.health().await.unwrap());
        assert!(api.get(&DeviceRegistry).await.unwrap().is_some());
        assert!(api.get(&DeadLetters::default()).await.unwrap().is_some());
        let watches = node.watches().await;
        assert!(watches.contains(&("*".to_owned(), "cicada.dead-letters/_".to_owned())), "{watches:?}");
        let expected = Catalog::new("x", EffectLog::new()).types().len();
        assert_eq!(api.types().await.unwrap().len(), expected);
        let info: serde_json::Value = node.get("/").await.json().await.unwrap();
        assert_eq!(info["node"], "test-node");
        assert_eq!(info["types"].as_array().unwrap().len(), expected);
        assert!(info.get("store").is_none() && info.get("groups").is_none(), "{info}");
        assert_eq!(node.get("/ready").await.status(), StatusCode::OK);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_saga_runs_over_http() {
    let db = temp_db();
    with_node(env(&db.path), &[], async |node| {
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
    with_node(env(&db.path), &[], async |node| {
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
    with_node(env(&db.path), &[], async |node| {
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
            api.state::<_, DeadLettersState>(&DeadLetters::default())
                .await
                .unwrap()
                .is_some_and(|s| s.letters.contains_key(&failing.id))
        })
        .await;
        let letters = api.state::<_, DeadLettersState>(&DeadLetters::default()).await.unwrap().unwrap().letters;
        assert_eq!(letters.len(), 2);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_api_token_the_api_requires_the_bearer_token() {
    let db = temp_db();
    let mut env = env(&db.path);
    env.insert("API_TOKEN".into(), "s3cret".into());
    with_node(env, &[], async |node| {
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
    // A node hosting the E2E types registers them first (same SQLite file), then a node hosting nothing serves the API.
    let db = temp_db();
    let counter = Counter { id: "c".into() };
    let e2e_only = ["--types", "e2e.slow,e2e.counter,e2e.recorder,e2e.flaky"];
    with_node(env(&db.path), &e2e_only, async |node| {
        let info: serde_json::Value = node.get("/").await.json().await.unwrap();
        assert_eq!(info["types"].as_array().unwrap().len(), 4, "{info}");
        assert!(
            node.api.get(&DeadLetters::default()).await.unwrap().is_none(),
            "no singleton of a type it does not host"
        );
        node.api.create(&counter, &CounterState::default()).await.unwrap();
    })
    .await;
    with_node(env(&db.path), &["--types", "none"], async |node| {
        let api = &node.api;
        let sent = api.send(&counter, CounterMsg::Step { seq: 1, hold_millis: 0 }).await.unwrap();
        // Nobody here runs it.
        assert_eq!(api.status(&sent.id).await.unwrap().map(|s| s.status), Some(MessageState::Pending));
        let address = counter.address().unwrap();
        let refused = api.send_message(&address, "Nope", serde_json::json!({}), SendOptions::new()).await;
        assert_eq!(refused.unwrap_err().status(), Some(422));
        assert!(api.status(&MessageId::new("unknown").unwrap()).await.unwrap().is_none());
        let info = node.get("/").await.text().await.unwrap();
        assert!(info.contains(r#""types":[]"#), "{info}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_shutdown_the_node_stops_and_the_port_is_closed() {
    let db = temp_db();
    let node = Node::start(&env(&db.path), &[]).await;
    let base = node.base.clone();
    assert_eq!(reqwest::get(format!("{base}/ready")).await.unwrap().status(), StatusCode::OK);
    node.stop().await;
    assert!(reqwest::get(format!("{base}/ready")).await.is_err(), "the port is closed");
}

/// sample-execution, Scenario: A flag overrides the environment locally. `--node-id` wins over `CICADA_NODE_ID`
/// (`test-node`), and the records and effects of the E2E actors name the id the node runs as.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_node_id_flag_overrides_the_variable_in_records_and_effects() {
    let db = temp_db();
    with_node(env(&db.path), &["--node-id", "flagged"], async |node| {
        let api = &node.api;
        let info: serde_json::Value = node.get("/").await.json().await.unwrap();
        assert_eq!(info["node"], "flagged");
        let counter = Counter { id: "c".into() };
        let recorder = Recorder { id: "r".into() };
        api.create(&counter, &CounterState::default()).await.unwrap();
        api.send(&counter, CounterMsg::Step { seq: 1, hold_millis: 0 }).await.unwrap();
        api.create(&recorder, &RecorderState::default()).await.unwrap();
        api.send(&recorder, RecorderMsg::Record { label: "e".into(), sleep_millis: 0 }).await.unwrap();
        eventually("both ran", || async {
            let counted = api.state::<_, CounterState>(&counter).await.unwrap().is_some_and(|s| s.count == 1);
            counted && api.state::<_, RecorderState>(&recorder).await.unwrap().is_some_and(|s| s.records.len() == 1)
        })
        .await;
        assert_eq!(api.state::<_, CounterState>(&counter).await.unwrap().unwrap().entries[0].node, "flagged");
        assert_eq!(api.state::<_, RecorderState>(&recorder).await.unwrap().unwrap().records[0].node, "flagged");
        let effects = node.get("/e2e/effects?actor=e2e.recorder/r").await.text().await.unwrap();
        assert!(effects.contains(r#""node":"flagged""#), "{effects}");
    })
    .await;
}

/// app, Scenario: A host with an async main. The binary's `main` is async and calls `run_async`: it serves, and stops
/// on SIGTERM with exit code 0.
#[cfg(unix)]
#[test]
fn the_binary_serves_and_stops_on_sigterm_with_exit_code_0() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let db = temp_db();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cicada-sample-service"))
        .args(["serve", "--listen", "127.0.0.1:0"])
        .env_clear()
        .env("CICADA_STORE", &db.path)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary starts");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let base = loop {
        let line = lines.next().expect("a line before the end").unwrap();
        if let Some(base) = line.strip_prefix("Listening on ") {
            break base.to_owned();
        }
    };
    let mut socket = std::net::TcpStream::connect(base.trim_start_matches("http://")).unwrap();
    socket.write_all(b"GET /ready HTTP/1.1\r\nHost: node\r\nConnection: close\r\n\r\n").unwrap();
    let mut answer = String::new();
    std::io::Read::read_to_string(&mut socket, &mut answer).unwrap();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    let killed = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
    assert!(killed.success());
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert_eq!(status.code(), Some(0), "{stderr}");
}

/// The node on PostgreSQL (`CICADA_STORE`), in a schema of its own. Skipped without `CICADA_PG_URL`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_node_runs_on_postgres() {
    let Ok(url) = std::env::var("CICADA_PG_URL") else {
        eprintln!("CICADA_PG_URL is not set; skipping");
        return;
    };
    let schema = format!("sample_{}", uuid_like());
    let mut env = env(std::path::Path::new("unused.db"));
    env.insert("CICADA_STORE".into(), url);
    env.insert("CICADA_STORE_SCHEMA".into(), schema);
    env.insert("CICADA_CONCURRENCY".into(), "4".into());
    with_node(env, &[], async |node| {
        let api = &node.api;
        assert!(api.health().await.unwrap());
        assert!(api.get(&DeviceRegistry).await.unwrap().is_some());
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
