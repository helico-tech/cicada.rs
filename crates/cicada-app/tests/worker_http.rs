//! `worker` over HTTP through `App::run_with` (spec `app`, ADR 0042): unchanged actors run through a server that hosts
//! nothing, on real sockets in real time. The server's router records every request and can require credentials,
//! redirect, hang or refuse; the assertions wait for what they observe and make no timing claim. Timing (the wait for
//! the server, renewals, how long a stop waits) and the races of the start and the stop are tested on virtual time in
//! `src/node.rs`, also over a real socket with the worker's own client held at a barrier.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use cicada::actor::contract_of;
use cicada::api::MessageState;
use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
use cicada::prelude::*;
use cicada::runtime::NodeConfig;
use cicada::{HandlerResult, Store, Turn};
use cicada_app::{App, AppError, Env};
use cicada_client::ActorClient;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};

use crate::common::{
    Counter, CounterActor, CounterMsg, CounterState, OtherCounterActor, RegistryActor, Running, TempDb, eventually,
    no_config, run,
};

// ---- the server ----

/// A request the server saw.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    tenant: Option<String>,
}

/// What the server does besides the API, and what it saw.
#[derive(Default)]
struct Behaviour {
    seen: Mutex<Vec<Seen>>,
    /// Every request needs `Authorization: Bearer <token>` and `X-Tenant: ops`, else `401`.
    require: Mutex<Option<String>>,
    /// `POST /leases` is answered `307` to `/moved`.
    redirect_leases: AtomicBool,
    /// `PUT /types/{name}` never answers.
    hang_registration: AtomicBool,
    /// The lease routes answer `403`.
    refuse_leases: AtomicBool,
}

impl Behaviour {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn count(&self, method: &str, path: &str) -> usize {
        self.seen().iter().filter(|s| s.method == method && s.path == path).count()
    }
}

fn seen(request: &Request) -> Seen {
    let header = |name: &str| request.headers().get(name).map(|v| v.to_str().unwrap().to_owned());
    Seen {
        method: request.method().to_string(),
        path: request.uri().path().to_owned(),
        authorization: header("authorization"),
        tenant: header("x-tenant"),
    }
}

async fn behave(State(behaviour): State<Arc<Behaviour>>, request: Request, next: Next) -> Response {
    let seen = seen(&request);
    behaviour.seen.lock().unwrap().push(seen.clone());
    let required = behaviour.require.lock().unwrap().clone();
    if let Some(token) = required
        && (seen.authorization != Some(format!("Bearer {token}")) || seen.tenant.as_deref() != Some("ops"))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if behaviour.hang_registration.load(Ordering::SeqCst) && seen.method == "PUT" && seen.path.starts_with("/types/") {
        return std::future::pending::<Response>().await;
    }
    if behaviour.redirect_leases.load(Ordering::SeqCst) && seen.path == "/leases" {
        return (StatusCode::TEMPORARY_REDIRECT, [("location", "/moved")]).into_response();
    }
    if behaviour.refuse_leases.load(Ordering::SeqCst) && seen.path.starts_with("/leases") {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

/// A started node that hosts nothing, on an in-memory store, serving the API behind [`behave`].
struct Server {
    system: ActorSystem,
    base: String,
    behaviour: Arc<Behaviour>,
}

impl Server {
    async fn start() -> Self {
        let system = ActorSystem::builder(Store::memory()).build();
        system.start().await.unwrap();
        let behaviour = Arc::new(Behaviour::default());
        let routes: Router =
            cicada_axum::actor_api(system.clone()).layer(middleware::from_fn_with_state(behaviour.clone(), behave));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, routes).await });
        Self { system, base, behaviour }
    }

    fn client(&self) -> ActorClient {
        ActorClient::new(&self.base)
    }

    async fn registered(&self) -> Vec<String> {
        self.system.types().await.unwrap().into_iter().map(|t| t.definition.name.to_string()).collect()
    }

    /// Waits until the worker registered `type_name`.
    async fn registers(&self, type_name: &str) {
        eventually("the registration", || async { self.registered().await.iter().any(|t| t == type_name) }).await;
    }
}

/// The environment of a worker over HTTP: no config file and a short poll interval.
fn http_env() -> Env {
    let mut env = no_config();
    env.insert("CICADA_POLL_INTERVAL".into(), "50ms".into());
    env
}

async fn state(client: &ActorClient, id: &MessageId) -> Option<MessageState> {
    client.status(id).await.unwrap().map(|s| s.status)
}

async fn done(client: &ActorClient, id: &MessageId) {
    eventually("the turn is done", || async { state(client, id).await == Some(MessageState::Done) }).await;
}

/// Creates the counter `name` and sends it `message`; the id of the message.
async fn send(client: &ActorClient, name: &str, message: CounterMsg) -> MessageId {
    let counter = Counter { name: name.into() };
    client.create(&counter, &CounterState::default()).await.unwrap();
    client.send(&counter, message).await.unwrap().id
}

#[tokio::test]
async fn an_http_worker_uses_the_id_from_the_node_config_hook() {
    let server = Server::start().await;
    let id = cicada::store::NodeId::new("runner-http").unwrap();
    let worker_id = id.clone();
    let app = App::new().host(RegistryActor).host(CounterActor).node_config_for(move |types| {
        assert_eq!(types, ["counter"]);
        Ok(NodeConfig { node_id: worker_id, ..NodeConfig::default() })
    });
    let worker = Running::spawn(app, &http_env(), &["myapp", "worker", "--server", &server.base, "--types", "counter"]);
    server.registers("counter").await;
    let counter = Counter { name: "pinned".into() };
    server.system.create_with(&counter, &CounterState::default(), CreateOptions::new().pinned_to(id)).await.unwrap();
    let sent = server.client().send(&counter, CounterMsg::Add { amount: 1 }).await.unwrap();
    done(&server.client(), &sent.id).await;
    worker.stop().await.unwrap();
}

#[tokio::test]
async fn an_invalid_http_target_skips_the_node_config_hook() {
    let cases = [
        (&["myapp", "worker", "--context", "missing"][..], "no context 'missing'"),
        (&["myapp", "worker", "--server", "no-url"][..], "invalid server URL"),
        (&["myapp", "worker", "--server", "http://localhost", "--token", "bad\ntoken"][..], "invalid token"),
    ];
    for (args, diagnostic) in cases {
        let app = App::new().host(CounterActor).node_config_for(|_| panic!("the HTTP target is invalid"));
        let error = run(app, &http_env(), args).await.unwrap_err();
        assert_eq!(error.exit_code(), 1, "{error}");
        assert!(error.to_string().contains(diagnostic), "{error}");
    }
}

// ---- an actor that waits for the test ----

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Gated {
    name: String,
}

impl ActorRef for Gated {
    const TYPE: &'static str = "gated";
    type Message = GatedMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum GatedMsg {
    /// Tells the test it began, then waits until the gate opens.
    Wait,
}

#[derive(Clone)]
struct Gate {
    began: Arc<Notify>,
    open: watch::Sender<bool>,
}

struct GatedActor(Gate);

impl Actor for GatedActor {
    type Ref = Gated;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: GatedMsg) -> HandlerResult {
        self.0.began.notify_one();
        let _ = self.0.open.subscribe().wait_for(|open| *open).await;
        Ok(())
    }
}

/// Counts the turns it is told of.
#[derive(Default)]
struct Observed {
    started: AtomicUsize,
    ended: Arc<AtomicUsize>,
}

struct ObservedEnd(Arc<AtomicUsize>);

impl TurnObserver for Observed {
    fn turn_started(&self, _: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        self.started.fetch_add(1, Ordering::SeqCst);
        Box::new(ObservedEnd(self.ended.clone()))
    }
}

impl ObservedTurn for ObservedEnd {
    fn turn_ended(self: Box<Self>, _: &TurnEnd<'_>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// ---- the tests ----

/// Scenario: No database dependency. Scenario: Applicable settings (the selected types and the observers).
/// Scenario: Ignored settings cannot break HTTP execution.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_http_worker_runs_the_selected_actors_without_opening_the_store_of_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    let unused = dir.path().join("never.db");
    for store in ["postgres://cicada:secret@127.0.0.1:1/cicada".to_owned(), unused.display().to_string()] {
        let server = Server::start().await;
        let observed = Arc::new(Observed::default());
        let blank_label = NodeConfig { label: Some(" ".into()), ..NodeConfig::default() };
        let app = App::new().host(CounterActor).host(RegistryActor).observe(observed.clone()).node_config(blank_label);
        let mut env = http_env();
        env.insert("CICADA_NODE_LABEL".into(), "a\0b".into());
        env.insert("CICADA_STORE".into(), store.clone());
        env.insert("CICADA_LEASE_TTL".into(), "soon".into());
        env.insert("CICADA_RETENTION".into(), "forever".into());
        env.insert("CICADA_LISTEN".into(), "localhost".into());
        let node = Running::spawn(app, &env, &["myapp", "worker", "--server", &server.base, "--types", "counter"]);
        server.registers("counter").await;
        let client = server.client();
        let id = send(&client, "c", CounterMsg::Add { amount: 5 }).await;
        done(&client, &id).await;
        assert_eq!(client.state::<_, CounterState>(&Counter { name: "c".into() }).await.unwrap().unwrap().count, 5);
        assert_eq!(server.registered().await, ["counter"], "only the selected type: {store}");
        eventually("the observer saw the end", || async { observed.ended.load(Ordering::SeqCst) == 1 }).await;
        assert_eq!(observed.started.load(Ordering::SeqCst), 1);
        node.stop().await.expect("exit code 0");
        assert!(server.system.nodes().await.unwrap().len() == 1, "the worker added no node row");
    }
    assert!(!unused.exists(), "the SQLite file was not created");
}

/// Scenario: Credentials and diagnostics: the token and the headers of a context reach the type and lease routes
/// (renewals are covered on virtual time in `src/node.rs`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_token_and_headers_of_a_context_reach_every_route_of_the_worker() {
    let server = Server::start().await;
    *server.behaviour.require.lock().unwrap() = Some("s3cret".into());
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let file = format!(
        "[contexts.prod]\nserver = \"{}\"\ntoken = \"s3cret\"\nheaders = {{ \"X-Tenant\" = \"ops\" }}\n",
        server.base
    );
    std::fs::write(&config, file).unwrap();
    let mut env = http_env();
    env.insert("CICADA_CONFIG".into(), config.display().to_string());
    let node = Running::spawn(App::new().host(CounterActor), &env, &["myapp", "worker", "--context", "prod"]);
    let headers = [("authorization", "Bearer s3cret"), ("x-tenant", "ops")]
        .into_iter()
        .map(|(name, value)| (name.parse().unwrap(), value.parse().unwrap()))
        .collect();
    let operator =
        ActorClient::with_client(&server.base, reqwest::Client::builder().default_headers(headers).build().unwrap());
    server.registers("counter").await;
    let id = send(&operator, "c", CounterMsg::Add { amount: 1 }).await;
    done(&operator, &id).await;
    node.stop().await.expect("exit code 0");
    let seen = server.behaviour.seen();
    for (method, path) in
        [("GET", "/types"), ("PUT", "/types/counter"), ("POST", "/leases"), ("POST", "/leases/complete")]
    {
        assert!(server.behaviour.count(method, path) >= 1, "{method} {path}: {seen:?}");
    }
    assert!(
        seen.iter().all(|s| s.authorization.as_deref() == Some("Bearer s3cret") && s.tenant.as_deref() == Some("ops")),
        "every request carried the token and the header: {seen:?}"
    );
}

/// Scenario: Permanent refusal or stop: a refusal of the credentials fails on the first try, without a retry (that it
/// takes no time is proven on virtual time in `src/node.rs`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_token_fails_the_worker_on_the_first_try() {
    let server = Server::start().await;
    *server.behaviour.require.lock().unwrap() = Some("s3cret".into());
    let args = ["myapp", "worker", "--server", &server.base, "--token", "wrong"];
    let error = run(App::new().host(CounterActor), &http_env(), &args).await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().starts_with("the server is not ready") && error.to_string().contains("401"), "{error}");
    assert!(!error.to_string().contains("wrong"), "the token is not printed: {error}");
    assert_eq!(server.behaviour.seen().len(), 1, "one try");
}

/// Scenario: Credentials and diagnostics: the registration that stays after a conflict is read with the token.
/// Scenario: Registration fails after successful preflight.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conflicting_contract_fails_the_start_and_is_read_with_the_token() {
    let server = Server::start().await;
    server.system.register_contract(&contract_of::<OtherCounterActor>().unwrap()).await.unwrap();
    *server.behaviour.require.lock().unwrap() = Some("s3cret".into());
    let mut env = http_env();
    env.insert("CICADA_SERVER".into(), server.base.clone());
    env.insert("CICADA_TOKEN".into(), "s3cret".into());
    let config = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(config.path(), "[contexts.c]\nserver = \"http://unused\"\nheaders = { \"X-Tenant\" = \"ops\" }\n")
        .unwrap();
    env.insert("CICADA_CONFIG".into(), config.path().display().to_string());
    let error = run(App::new().host(CounterActor), &env, &["myapp", "worker", "--context", "c"]).await;
    let error = error.expect_err("a conflict");
    // `--context c` gives the header, CICADA_SERVER wins over the context's server, CICADA_TOKEN gives the token.
    assert_eq!(error.exit_code(), 1);
    assert!(
        error.to_string().starts_with("the worker cannot start") && error.to_string().contains("counter"),
        "{error}"
    );
    assert_eq!(server.behaviour.count("GET", "/types/counter"), 1, "{:?}", server.behaviour.seen());
    assert_eq!(server.behaviour.count("POST", "/leases"), 0);
    assert!(server.behaviour.seen().iter().all(|s| s.authorization.as_deref() == Some("Bearer s3cret")));
}

/// Scenario: Redirect refusal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redirect_of_a_lease_call_is_not_followed_and_stops_the_worker() {
    let server = Server::start().await;
    server.behaviour.redirect_leases.store(true, Ordering::SeqCst);
    let args = ["myapp", "worker", "--server", &server.base];
    let error = run(App::new().host(CounterActor), &http_env(), &args).await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().starts_with("the worker stopped") && error.to_string().contains("307"), "{error}");
    assert_eq!(server.behaviour.count("POST", "/leases"), 1);
    assert_eq!(server.behaviour.count("POST", "/moved") + server.behaviour.count("GET", "/moved"), 0, "not followed");
}

/// Scenario: A stopped worker and a failed worker: a worker whose leases the server starts to refuse fails by itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_whose_leases_are_refused_later_fails_by_itself() {
    let server = Server::start().await;
    let node =
        Running::spawn(App::new().host(CounterActor), &http_env(), &["myapp", "worker", "--server", &server.base]);
    server.registers("counter").await;
    let client = server.client();
    done(&client, &send(&client, "c", CounterMsg::Add { amount: 1 }).await).await;
    server.behaviour.refuse_leases.store(true, Ordering::SeqCst);
    let error = node.ended().await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().starts_with("the worker stopped") && error.to_string().contains("403"), "{error}");
}

/// Scenario: Stop during registration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_during_registration_ends_the_worker_without_a_lease() {
    let server = Server::start().await;
    server.behaviour.hang_registration.store(true, Ordering::SeqCst);
    let mut node =
        Running::spawn(App::new().host(CounterActor), &http_env(), &["myapp", "worker", "--server", &server.base]);
    eventually("the registration", || async { server.behaviour.count("PUT", "/types/counter") == 1 }).await;
    assert!(!node.is_finished());
    node.signal();
    node.ended().await.expect("exit code 0");
    assert_eq!(server.behaviour.count("POST", "/leases"), 0);
}

/// Scenario: Stop during running work: with a long grace period the turn that was running when the stop came is
/// completed, and the command succeeds (that the stop waits for it is proven on virtual time in `src/node.rs`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_during_running_work_completes_the_turn() {
    let server = Server::start().await;
    let gate = Gate { began: Arc::new(Notify::new()), open: watch::channel(false).0 };
    let mut env = http_env();
    env.insert("CICADA_SHUTDOWN_GRACE_PERIOD".into(), "30s".into());
    let app = App::new().host(GatedActor(gate.clone()));
    let mut node = Running::spawn(app, &env, &["myapp", "worker", "--server", &server.base]);
    server.registers("gated").await;
    let client = server.client();
    let gated = Gated { name: "g".into() };
    client.create(&gated, &NoState {}).await.unwrap();
    let id = client.send(&gated, GatedMsg::Wait).await.unwrap().id;
    gate.began.notified().await;
    node.signal();
    gate.open.send_replace(true);
    node.ended().await.expect("exit code 0");
    assert_eq!(state(&client, &id).await, Some(MessageState::Done));
    assert_eq!(server.behaviour.count("POST", "/leases/release"), 0, "a finished handler is completed, not released");
}

/// Scenario: Uncooperative or finished work: without a grace period the stop cancels a turn whose handler has not
/// ended, and its lease is released with the token: the message is not done and waits for another worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_without_a_grace_period_releases_the_lease_of_an_unfinished_handler() {
    let server = Server::start().await;
    *server.behaviour.require.lock().unwrap() = Some("s3cret".into());
    let gate = Gate { began: Arc::new(Notify::new()), open: watch::channel(false).0 };
    let mut env = http_env();
    env.insert("CICADA_SHUTDOWN_GRACE_PERIOD".into(), "0s".into());
    let config = tempfile::NamedTempFile::new().unwrap();
    let file = format!(
        "[contexts.c]\nserver = \"{}\"\ntoken = \"s3cret\"\nheaders = {{ \"X-Tenant\" = \"ops\" }}\n",
        server.base
    );
    std::fs::write(config.path(), file).unwrap();
    env.insert("CICADA_CONFIG".into(), config.path().display().to_string());
    let app = App::new().host(GatedActor(gate.clone()));
    let mut node = Running::spawn(app, &env, &["myapp", "worker", "--context", "c"]);
    server.registers("gated").await;
    let gated = Gated { name: "g".into() };
    server.system.create(&gated, &NoState {}).await.unwrap();
    let id = match server.system.send(&gated, GatedMsg::Wait).await.unwrap() {
        cicada::SendResult::Accepted(m) => m.id,
        other => panic!("not accepted: {other:?}"),
    };
    gate.began.notified().await;
    node.signal();
    node.ended().await.expect("exit code 0");
    assert_eq!(server.behaviour.count("POST", "/leases/release"), 1, "{:?}", server.behaviour.seen());
    assert_eq!(server.behaviour.count("POST", "/leases/complete"), 0);
    let status = server.system.status(&id).await.unwrap();
    assert!(matches!(status, Some(cicada::store::MessageStatus::Pending(_))), "{status:?}");
    assert!(server.behaviour.seen().iter().all(|s| s.authorization.as_deref() == Some("Bearer s3cret")));
}

/// Scenario: Startup hooks require an ActorSystem: refused before the config file or the network. Scenario: A worker
/// over HTTP (with `App::task`).
#[tokio::test]
async fn on_start_and_a_task_are_refused_over_http_before_any_request() {
    let server = Server::start().await;
    let broken = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(broken.path(), "this is = not [toml").unwrap();
    let ran = Arc::new(AtomicBool::new(false));
    let selectors = [&["--server", server.base.as_str()][..], &[]];
    for (selector, task) in selectors.into_iter().flat_map(|s| [(s, false), (s, true)]) {
        let flag = ran.clone();
        let app = App::new().host(CounterActor).node_config_for(|_| panic!("HTTP mode refused the startup hook"));
        let app = if task {
            app.task("runner", move |_, _| async move {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            })
        } else {
            app.on_start(move |_| async move {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            })
        };
        let mut env = http_env();
        env.insert("CICADA_CONFIG".into(), broken.path().display().to_string());
        env.insert("CICADA_CONTEXT".into(), "prod".into());
        let args: Vec<&str> = ["myapp", "worker"].into_iter().chain(selector.iter().copied()).collect();
        let error = run(app, &env, &args).await.unwrap_err();
        assert_eq!(error.exit_code(), 2, "{error}");
        assert!(error.to_string().contains("App::on_start or App::task"), "{error}");
    }
    assert!(!ran.load(Ordering::SeqCst));
    assert!(server.behaviour.seen().is_empty(), "no request: {:?}", server.behaviour.seen());
}

/// Scenario: Explicit selection overrides ambient settings (the flags before `worker`). Scenario: Explicit server
/// settings. Scenario: Ignored settings cannot break HTTP execution (applicable values still checked).
#[tokio::test]
async fn usage_errors_send_no_request_and_a_bad_applicable_setting_fails_the_start() {
    let server = Server::start().await;
    let db = TempDb::new();
    let base = server.base.as_str();
    let store = db.path.to_str().unwrap();
    let mut bad_concurrency = http_env();
    bad_concurrency.insert("CICADA_CONCURRENCY".into(), "many".into());
    for (env, args, says) in [
        (http_env(), vec!["myapp", "--server", base, "worker", "--store", store], "not both"),
        (http_env(), vec!["myapp", "worker", "--server", base, "--lease-ttl", "5s"], "does not take --lease-ttl"),
        (http_env(), vec!["myapp", "worker", "--server", base, "--node-label", "x"], "has no row in the node table"),
        (bad_concurrency, vec!["myapp", "worker", "--server", base], "CICADA_CONCURRENCY"),
        (http_env(), vec!["myapp", "worker", "--token", "t"], "needs a store or a server"),
    ] {
        let error = run(App::new().host(CounterActor), &env, &args).await.unwrap_err();
        assert_eq!(error.exit_code(), 2, "{args:?}: {error}");
        assert!(error.to_string().contains(says), "{args:?}: {error}");
    }
    assert!(server.behaviour.seen().is_empty(), "no request: {:?}", server.behaviour.seen());
    assert!(!db.path.exists(), "no store was opened");
    let error =
        run(App::new().host(CounterActor), &http_env(), &["myapp", "worker", "--server", base, "--concurrency", "0"])
            .await
            .unwrap_err();
    assert_eq!(error.exit_code(), 1, "the worker validates its own limits at the start");
    assert!(error.to_string().contains("concurrency"), "{error}");
}

/// Scenario: HTTP target resolution: a worker on the store does not read the config file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_on_the_store_does_not_read_the_config_file() {
    let db = TempDb::new();
    let broken = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(broken.path(), "this is = not [toml").unwrap();
    let mut env = db.env();
    env.insert("CICADA_CONFIG".into(), broken.path().display().to_string());
    env.insert("CICADA_SERVER".into(), "http://ambient".into());
    env.insert("CICADA_TOKEN".into(), "ambient".into());
    let node = Running::spawn(App::new().host(CounterActor), &env, &["myapp", "worker"]);
    eventually("the store worker registered its type", || async {
        let Ok(store) = Store::sqlite(&db.path, cicada::store::SqliteOptions::default()).await else { return false };
        let registered = ActorSystem::builder(store.clone()).build().types().await.map(|t| t.len()).unwrap_or(0);
        store.close().await;
        registered == 1
    })
    .await;
    assert!(!node.is_finished());
    let result: Result<(), AppError> = node.stop().await;
    result.expect("exit code 0");
}
