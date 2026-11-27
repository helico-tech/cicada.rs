//! `serve` and `worker` through `App::run_with`, on a SQLite file in a temporary folder and a port of the loopback
//! address, in real time (and on PostgreSQL when `CICADA_PG_URL` is set).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use cicada::api::MessageState;
use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
use cicada::runtime::{NodeConfig, NodeState};
use cicada::store::{MessageStatus, NodeId, OutcomeStatus, SqliteOptions};
use cicada::{ActorSystem, Store};
use cicada_app::{App, AppError, Env, node_command};
use cicada_client::ActorClient;

use crate::common::{
    Counter, CounterActor, CounterMsg, CounterState, OtherCounterActor, Registry, RegistryActor, Running, TempDb, args,
    env_of, eventually, free_port, no_config, run,
};

fn counter_app() -> App {
    App::new().host(CounterActor)
}

#[test]
fn type_names_follow_host_call_order() {
    assert!(App::new().type_names().is_empty());
    assert_eq!(App::new().host(RegistryActor).host(CounterActor).type_names(), ["registry", "counter"]);
}

const LOOPBACK_ANY: [&str; 4] = ["myapp", "serve", "--listen", "127.0.0.1:0"];

#[tokio::test]
async fn node_config_for_sees_the_selected_types_in_host_order_and_flags_win() {
    for (flags, selected, override_id) in [
        (&[][..], &["registry", "counter"][..], None),
        (&["--types", "registry"][..], &["registry"][..], None),
        (&["--types", "none"][..], &[][..], None),
        (&["--types", "counter,registry", "--node-id", "other"][..], &["registry", "counter"][..], Some("other")),
    ] {
        let db = TempDb::new();
        let (chosen, mut choice) = tokio::sync::oneshot::channel();
        let app = App::new().host(RegistryActor).host(CounterActor).node_config_for(move |types| {
            assert_eq!(types, selected);
            let mut config = NodeConfig::default();
            if types.contains(&"counter") {
                config.node_id = NodeId::new("runner").unwrap();
            }
            chosen.send(config.node_id.clone()).unwrap();
            Ok(config)
        });
        let args: Vec<_> = LOOPBACK_ANY.into_iter().chain(flags.iter().copied()).collect();
        let (node, base) = Running::serve(app, &db.env(), &args).await;
        let chosen = choice.try_recv().unwrap();
        let nodes = ActorClient::new(&base).nodes().await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id.as_str(), override_id.unwrap_or(chosen.as_str()));
        node.stop().await.unwrap();
    }
}

#[tokio::test]
async fn the_last_node_config_builder_call_wins() {
    let fixed = || NodeConfig { node_id: NodeId::new("fixed").unwrap(), ..NodeConfig::default() };
    let hook = || NodeConfig { node_id: NodeId::new("hook").unwrap(), ..NodeConfig::default() };
    let apps = [
        (counter_app().node_config_for(|_| panic!("replaced by node_config")).node_config(fixed()), "fixed"),
        (counter_app().node_config(fixed()).node_config_for(move |_| Ok(hook())), "hook"),
    ];
    for (app, expected) in apps {
        let db = TempDb::new();
        let (node, base) = Running::serve(app, &db.env(), &LOOPBACK_ANY).await;
        assert_eq!(ActorClient::new(&base).nodes().await.unwrap()[0].id.as_str(), expected);
        node.stop().await.unwrap();
    }
}

#[tokio::test]
async fn a_node_config_hook_error_fails_before_opening_the_store() {
    let db = TempDb::new();
    let app = counter_app().node_config_for(|_| Err(anyhow::anyhow!("identity is locked").context("load identity")));
    let error = run(app, &db.env(), &["myapp", "worker"]).await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert_eq!(error.to_string(), "load identity: identity is locked");
    assert!(!db.path.exists(), "the store was not opened");
}

#[tokio::test]
async fn a_bad_flag_or_variable_skips_the_node_config_hook() {
    let db = TempDb::new();
    let mut bad_env = db.env();
    bad_env.insert("CICADA_POLL_INTERVAL".into(), "soon".into());
    let cases = [
        (db.env(), &["myapp", "worker", "--types", "absent"][..], "--types"),
        (bad_env, &["myapp", "worker"][..], "CICADA_POLL_INTERVAL"),
    ];
    for (env, args, setting) in cases {
        let app = counter_app().node_config_for(|_| panic!("settings did not pass validation"));
        let error = run(app, &env, args).await.unwrap_err();
        assert_eq!(error.exit_code(), 2, "{error}");
        assert!(error.to_string().contains(setting), "{error}");
    }
}

#[tokio::test]
async fn help_and_version_skip_the_node_config_hook() {
    for args in [&["myapp", "serve", "--help"][..], &["myapp", "--version"][..]] {
        let app = counter_app().node_config_for(|_| panic!("the command does not run a node"));
        assert!(!run(app, &no_config(), args).await.unwrap().is_empty());
    }
}

async fn status(client: &ActorClient, id: &cicada::store::MessageId) -> Option<MessageState> {
    client.status(id).await.unwrap().map(|s| s.status)
}

/// The types registered in the SQLite file `db`.
async fn registered(db: &TempDb) -> usize {
    let store = Store::sqlite(&db.path, SqliteOptions::default()).await.unwrap();
    let types = ActorSystem::builder(store.clone()).build().types().await.unwrap().len();
    store.close().await;
    types
}

/// Whether something answers HTTP on `port` within half a second.
async fn answers(port: u16) -> bool {
    let http = reqwest::Client::builder().timeout(Duration::from_millis(500)).build().unwrap();
    http.get(format!("http://127.0.0.1:{port}/ready")).send().await.is_ok()
}

/// Scenario: Serve, create and send. Scenario: The address it listens on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_creates_and_sends_over_http_and_answers_ready() {
    let db = TempDb::new();
    let (node, base) = Running::serve(counter_app(), &db.env(), &LOOPBACK_ANY).await;
    let port: u16 = base.strip_prefix("http://127.0.0.1:").expect("the loopback address").parse().unwrap();
    assert_ne!(port, 0);
    assert_eq!(node.out.text(), format!("Listening on {base}\n"), "one line");

    let client = ActorClient::new(&base);
    let counter = Counter { name: "c".into() };
    client.create(&counter, &CounterState::default()).await.unwrap();
    let sent = client.send(&counter, CounterMsg::Add { amount: 5 }).await.unwrap();
    eventually("done", || async { status(&client, &sent.id).await == Some(MessageState::Done) }).await;
    assert_eq!(client.state::<_, CounterState>(&counter).await.unwrap().unwrap().count, 5);
    assert_eq!(reqwest::get(format!("{base}/ready")).await.unwrap().status(), StatusCode::OK);
    node.stop().await.expect("stopped in good order");
    assert!(!answers(port).await, "the port is closed");
}

/// Scenario: Stopping.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_lets_the_running_turn_finish_while_the_api_answers_and_ready_says_503() {
    let db = TempDb::new();
    let mut env = db.env();
    env.insert("CICADA_SHUTDOWN_GRACE_PERIOD".into(), "10s".into());
    let (mut node, base) = Running::serve(counter_app(), &env, &LOOPBACK_ANY).await;
    let client = ActorClient::new(&base);
    let counter = Counter { name: "c".into() };
    client.create(&counter, &CounterState::default()).await.unwrap();
    let held = client.send(&counter, CounterMsg::Hold { millis: 1500 }).await.unwrap();
    eventually("running", || async { status(&client, &held.id).await == Some(MessageState::Running) }).await;

    node.signal();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!node.is_finished(), "the turn is still running");
    assert_eq!(reqwest::get(format!("{base}/ready")).await.unwrap().status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reqwest::get(format!("{base}/types")).await.unwrap().status(), StatusCode::OK, "the API answers");
    node.ended().await.expect("exit code 0");

    let store = Store::sqlite(&db.path, SqliteOptions::default()).await.unwrap();
    let system = ActorSystem::builder(store.clone()).build();
    let finished = system.status(&held.id).await.unwrap();
    assert!(
        matches!(finished, Some(MessageStatus::Finished(ref o)) if o.status == OutcomeStatus::Done),
        "{finished:?}"
    );
    store.close().await;
}

/// Scenario: The address is taken.
#[tokio::test]
async fn an_address_that_is_taken_fails_before_the_node_starts() {
    let db = TempDb::new();
    let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = taken.local_addr().unwrap().to_string();
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let app = counter_app().on_start(move |_| async move {
        flag.store(true, Ordering::SeqCst);
        Ok(())
    });
    let error = run(app, &db.env(), &["myapp", "serve", "--listen", &address]).await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().contains(&format!("cannot listen on {address}")), "{error}");
    assert!(!ran.load(Ordering::SeqCst), "on_start did not run");
    assert_eq!(registered(&db).await, 0, "no node started: nothing was registered");
}

/// Scenario: The node cannot start.
#[tokio::test]
async fn a_node_that_cannot_start_fails_with_the_problem_and_nothing_listens() {
    let db = TempDb::new();
    let store = Store::sqlite(&db.path, SqliteOptions::default()).await.unwrap();
    ActorSystem::builder(store.clone()).host(OtherCounterActor).build().initialize().await.unwrap();
    store.close().await;
    let port = free_port();
    let listen = format!("127.0.0.1:{port}");
    let error = run(counter_app(), &db.env(), &["myapp", "serve", "--listen", &listen]).await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().contains("the node cannot start") && error.to_string().contains("contract"), "{error}");
    assert!(!answers(port).await, "nothing listens");
}

/// Scenario: Label precedence: an invalid label of the host fails the start (exit code 1), an invalid flag is a usage
/// error (exit code 2).
#[tokio::test]
async fn an_invalid_label_of_the_host_fails_the_start_and_one_of_the_flag_the_command_line() {
    let db = TempDb::new();
    let blank = NodeConfig { label: Some(" ".into()), ..NodeConfig::default() };
    let error = run(counter_app().node_config(blank), &db.env(), &["myapp", "worker"]).await.unwrap_err();
    assert_eq!(error.exit_code(), 1, "{error}");
    assert!(error.to_string().contains("label must not be blank"), "{error}");
    let error = run(counter_app(), &db.env(), &["myapp", "worker", "--node-label", " "]).await.unwrap_err();
    assert_eq!(error.exit_code(), 2, "{error}");
    assert!(error.to_string().contains("--node-label"), "{error}");
}

/// Scenario: A stop while the store is waited for.
#[tokio::test]
async fn a_stop_while_the_store_is_waited_for_ends_at_once() {
    let mut env = no_config();
    env.insert("CICADA_STORE".into(), "postgres://cicada:cicada@127.0.0.1:1/cicada".into());
    let mut node = Running::spawn(counter_app(), &env, &["myapp", "worker"]);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!node.is_finished(), "it waits for the store");
    let stopped = Instant::now();
    node.signal();
    node.ended().await.expect("exit code 0");
    assert!(stopped.elapsed() < Duration::from_millis(500), "{:?}", stopped.elapsed());
}

/// Scenario: A stop during an operator command.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_operator_command_does_not_look_at_the_stop() {
    let db = TempDb::new();
    let slow = |api: Router, _: &ActorSystem| {
        api.layer(middleware::from_fn(|request: Request, next: Next| async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            next.run(request).await
        }))
    };
    let (node, base) = Running::serve(counter_app().http(slow), &db.env(), &LOOPBACK_ANY).await;
    ActorClient::new(&base).create(&Counter { name: "c".into() }, &CounterState::default()).await.unwrap();
    let out = common::Out::default();
    let command = counter_app().node_config_for(|_| panic!("the operator command does not run a node")).run_with(
        args(&["myapp", "get", "actors", "--server", &base]),
        no_config(),
        out.clone(),
        std::future::ready(()),
    );
    command.await.expect("the command goes on");
    assert!(out.text().contains("counter/c"), "{}", out.text());
    node.stop().await.unwrap();
}

/// Scenario: A stop while `on_start` runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_while_on_start_runs_drops_it_and_no_request_is_taken() {
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let db = TempDb::new();
    let (began, dropped) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let (b, d) = (began.clone(), dropped.clone());
    let app = counter_app().on_start(move |_| async move {
        let _guard = Dropped(d);
        b.store(true, Ordering::SeqCst);
        std::future::pending::<anyhow::Result<()>>().await
    });
    let port = free_port();
    let listen = format!("127.0.0.1:{port}");
    let mut node = Running::spawn(app, &db.env(), &["myapp", "serve", "--listen", &listen]);
    eventually("on_start began", || async { began.load(Ordering::SeqCst) }).await;
    assert!(!answers(port).await, "no request is taken while on_start runs");
    node.signal();
    let out = node.out.clone();
    node.ended().await.expect("exit code 0");
    assert!(dropped.load(Ordering::SeqCst), "the function was dropped");
    assert!(!out.text().contains("Listening"), "{}", out.text());
}

/// Scenario: A worker next to a serving node.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_handles_what_a_serving_node_that_hosts_nothing_accepts() {
    let db = TempDb::new();
    let started = Arc::new(AtomicBool::new(false));
    let flag = started.clone();
    let worker_app = counter_app().on_start(move |_| async move {
        flag.store(true, Ordering::SeqCst);
        Ok(())
    });
    let worker = Running::spawn(worker_app, &db.env(), &["myapp", "worker"]);
    eventually("the worker started", || async { started.load(Ordering::SeqCst) }).await;
    let (server, base) =
        Running::serve(counter_app(), &db.env(), &["myapp", "serve", "--listen", "127.0.0.1:0", "--types", "none"])
            .await;
    let client = ActorClient::new(&base);
    let counter = Counter { name: "c".into() };
    client.create(&counter, &CounterState::default()).await.unwrap();
    let sent = client.send(&counter, CounterMsg::Add { amount: 2 }).await.unwrap();
    eventually("done", || async { status(&client, &sent.id).await == Some(MessageState::Done) }).await;
    let nodes = client.nodes().await.unwrap();
    let hosting: Vec<usize> = nodes.iter().map(|n| n.hosted.len()).collect();
    assert!(hosting.contains(&0) && hosting.contains(&1), "{nodes:?}");
    server.stop().await.unwrap();
    worker.stop().await.unwrap();
}

async fn require_token(request: Request, next: Next) -> Response {
    match request.headers().get(header::AUTHORIZATION) {
        Some(value) if value == "Bearer t1" => next.run(request).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Scenario: A layer of the host. Scenario: A route of the host.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_layer_of_the_host_covers_the_api_and_not_ready_nor_a_route_merged_after_it() {
    let db = TempDb::new();
    let app = counter_app().http(|api, system| {
        let node = system.node_id().to_string();
        api.layer(middleware::from_fn(require_token)).merge(Router::new().route("/info", get(move || async { node })))
    });
    let (node, base) = Running::serve(app, &db.env(), &LOOPBACK_ANY).await;
    let http = reqwest::Client::new();
    assert_eq!(http.get(format!("{base}/types")).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    let with_token = http.get(format!("{base}/types")).bearer_auth("t1").send().await.unwrap();
    assert_eq!(with_token.status(), StatusCode::OK);
    assert_eq!(http.get(format!("{base}/ready")).send().await.unwrap().status(), StatusCode::OK);
    let info = http.get(format!("{base}/info")).send().await.unwrap();
    assert_eq!(info.status(), StatusCode::OK, "merged after the layer");
    assert!(!info.text().await.unwrap().is_empty());
    node.stop().await.unwrap();
}

/// Scenario: A route `/ready` of the host.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_route_ready_of_the_host_is_never_reached() {
    let db = TempDb::new();
    let app = counter_app().http(|api, _| api.merge(Router::new().route("/ready", get(|| async { "the host's" }))));
    let (node, base) = Running::serve(app, &db.env(), &LOOPBACK_ANY).await;
    let ready = reqwest::get(format!("{base}/ready")).await.unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    let body = ready.text().await.unwrap();
    assert!(body.contains("\"node\"") && !body.contains("the host's"), "App answers: {body}");
    node.stop().await.unwrap();
}

/// Scenario: A route of the host that clashes with the API.
#[tokio::test]
async fn a_route_that_clashes_with_the_api_fails_before_the_node_starts() {
    let db = TempDb::new();
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let app = counter_app()
        .http(|api, _| api.merge(Router::new().route("/health", get(|| async { "mine" }))))
        .on_start(move |_| async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
    let error = run(app, &db.env(), &LOOPBACK_ANY).await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().contains("the HTTP routes cannot be built"), "{error}");
    assert!(!ran.load(Ordering::SeqCst));
    assert_eq!(registered(&db).await, 0, "the node did not start");
}

/// Scenario: An actor that must exist.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_start_creates_an_actor_before_the_first_request() {
    let db = TempDb::new();
    let app = App::new().host(RegistryActor).on_start(|system| async move {
        system.create(&Registry, &cicada::NoState {}).await?;
        Ok(())
    });
    let (node, base) = Running::serve(app, &db.env(), &LOOPBACK_ANY).await;
    assert!(ActorClient::new(&base).get(&Registry).await.unwrap().is_some());
    node.stop().await.unwrap();
}

/// Scenario: In a worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_start_runs_once_in_a_worker_after_the_node_started() {
    let db = TempDb::new();
    let (runs, started) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
    let (r, s) = (runs.clone(), started.clone());
    let app = counter_app().on_start(move |system| async move {
        s.store(system.state() == NodeState::Started, Ordering::SeqCst);
        r.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let worker = Running::spawn(app, &db.env(), &["myapp", "worker"]);
    eventually("on_start ran", || async { runs.load(Ordering::SeqCst) == 1 }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    worker.stop().await.unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(started.load(Ordering::SeqCst), "after the node started");
}

/// Scenario: The function fails.
#[tokio::test]
async fn a_failing_on_start_stops_the_node_and_fails_the_command() {
    let db = TempDb::new();
    let app = counter_app().on_start(|_| async { anyhow::bail!("the dead letters cannot be created") });
    let port = free_port();
    let listen = format!("127.0.0.1:{port}");
    let error = run(app, &db.env(), &["myapp", "serve", "--listen", &listen]).await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert_eq!(error.to_string(), "the dead letters cannot be created");
    assert!(!answers(port).await, "nothing listens");
    let store = Store::sqlite(&db.path, SqliteOptions::default()).await.unwrap();
    let nodes = ActorSystem::builder(store.clone()).build().nodes().await.unwrap();
    assert!(nodes.is_empty(), "the node stopped and removed its row: {nodes:?}");
    store.close().await;
}

/// Counts the turns it is told of.
#[derive(Default)]
struct Seen {
    started: AtomicUsize,
    ended: Arc<AtomicUsize>,
}

struct SeenTurn(Arc<AtomicUsize>);

impl TurnObserver for Seen {
    fn turn_started(&self, _: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        self.started.fetch_add(1, Ordering::SeqCst);
        Box::new(SeenTurn(self.ended.clone()))
    }
}

impl ObservedTurn for SeenTurn {
    fn turn_ended(self: Box<Self>, _: &TurnEnd<'_>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Scenario: An observer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_observer_is_told_the_turns_of_the_node() {
    let db = TempDb::new();
    let seen = Arc::new(Seen::default());
    let (node, base) = Running::serve(counter_app().observe(seen.clone()), &db.env(), &LOOPBACK_ANY).await;
    let client = ActorClient::new(&base);
    let counter = Counter { name: "c".into() };
    client.create(&counter, &CounterState::default()).await.unwrap();
    client.send(&counter, CounterMsg::Add { amount: 1 }).await.unwrap();
    eventually("the end of the turn", || async { seen.ended.load(Ordering::SeqCst) == 1 }).await;
    assert_eq!(seen.started.load(Ordering::SeqCst), 1);
    node.stop().await.unwrap();
}

/// Scenario: A command of the host's binary. Scenario: No server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hosts_binary_has_the_operator_commands_over_http() {
    let db = TempDb::new();
    let (node, base) = Running::serve(counter_app(), &db.env(), &LOOPBACK_ANY).await;
    ActorClient::new(&base).create(&Counter { name: "c".into() }, &CounterState::default()).await.unwrap();
    let listed = run(counter_app(), &no_config(), &["myapp", "get", "actors", "--server", &base]).await.unwrap();
    assert!(listed.starts_with("ADDRESS") && listed.contains("counter/c"), "{listed}");
    node.stop().await.unwrap();

    let error = run(counter_app(), &no_config(), &["myapp", "get", "actors"]).await.unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().contains("no server") && error.to_string().contains("`myapp config set-context`"));
}

/// Scenario: An operator flag on `serve`.
#[tokio::test]
async fn serve_refuses_an_operator_flag_before_anything_starts() {
    let db = TempDb::new();
    let Err(AppError::Usage(error)) = run(counter_app(), &db.env(), &["myapp", "serve", "--token", "secret"]).await
    else {
        panic!("a usage error")
    };
    assert!(error.contains("no authentication of its own") && error.contains("App::http"), "{error}");
    assert!(!db.path.exists(), "no store was opened");
}

/// Scenario: No serve without types. (Scenario: An operator command of the plain binary is the whole of `tests/cli.rs`.)
#[tokio::test]
async fn a_binary_without_types_has_neither_serve_nor_worker() {
    let db = TempDb::new();
    for command in ["serve", "worker"] {
        let path = db.path.display().to_string();
        let Err(AppError::Usage(error)) = run(App::new(), &no_config(), &["cicada", command, "--store", &path]).await
        else {
            panic!("{command}: a usage error")
        };
        assert!(error.starts_with(&format!("unrecognized subcommand '{command}'")), "{error}");
    }
    assert!(!db.path.exists());
}

/// Scenario: Help.
#[tokio::test]
async fn the_help_of_a_hosts_binary_names_it_and_its_node_commands() {
    let help = run(counter_app(), &no_config(), &["myapp", "--help"]).await.unwrap();
    assert!(help.contains("Usage: myapp [OPTIONS] <COMMAND>"), "{help}");
    assert!(help.contains("\n  serve ") && help.contains("\n  worker ") && help.contains("\n  get "), "{help}");
    let serve = run(counter_app(), &no_config(), &["myapp", "serve", "--help"]).await.unwrap();
    for flag in ["--listen", "--store", "--types", "--retention", "--max-lost-claims", "CICADA_STORE"] {
        assert!(serve.contains(flag), "{flag}: {serve}");
    }
}

/// Scenario: A command line that cannot be read. Scenario: A value that cannot be read. Scenario: No store.
#[tokio::test]
async fn a_command_line_or_a_variable_that_cannot_be_read_is_a_usage_error() {
    let db = TempDb::new();
    let flag = run(counter_app(), &db.env(), &["myapp", "serve", "--lease-ttl", "soon"]).await.unwrap_err();
    assert!(matches!(flag, AppError::Usage(_)) && flag.exit_code() == 2, "{flag}");
    let mut env = db.env();
    env.insert("CICADA_LEASE_TTL".into(), "soon".into());
    let variable = run(counter_app(), &env, &["myapp", "worker"]).await.unwrap_err();
    assert_eq!(variable.exit_code(), 2);
    assert!(variable.to_string().contains("CICADA_LEASE_TTL"), "{variable}");
    let no_store = run(counter_app(), &no_config(), &["myapp", "serve"]).await.unwrap_err();
    assert_eq!(no_store.exit_code(), 2);
    assert!(no_store.to_string().contains("--store") && no_store.to_string().contains("CICADA_STORE"));
    let none = run(counter_app(), &db.env(), &["myapp", "worker", "--types", "none"]).await.unwrap_err();
    assert_eq!(none.exit_code(), 2, "{none}");
    let unknown = run(counter_app(), &db.env(), &["myapp", "serve", "--types", "counter,nope"]).await.unwrap_err();
    assert_eq!(unknown.exit_code(), 2, "{unknown}");
    assert!(!db.path.exists(), "nothing started");
}

/// Scenario: A PostgreSQL address without its slashes (no file is created). Scenario: An address of another kind.
#[tokio::test]
async fn a_mistyped_address_creates_no_file_and_is_not_printed() {
    let error = run(counter_app(), &no_config(), &["myapp", "serve", "--store", "postgres:db"]).await.unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(!std::path::Path::new("postgres:db").exists());
    let secret = run(counter_app(), &no_config(), &["myapp", "worker", "--store", "postgress://u:secret@h/db"]).await;
    let error = secret.unwrap_err().to_string();
    assert!(error.contains("postgres://") && !error.contains("secret"), "{error}");
}

/// An address given without `--store` is an argument clap does not know; its password stays out of the error.
#[tokio::test]
async fn a_usage_error_hides_the_password_of_an_address() {
    let line = ["myapp", "serve", "postgres://u:pw@h/db"];
    let Err(AppError::Usage(error)) = run(counter_app(), &no_config(), &line).await else { panic!("a usage error") };
    assert!(error.contains("'postgres://u:***@h/db'"), "{error}");
    assert!(!error.contains("pw"), "{error}");
}

/// The usage errors of the settings quote what the user gave, in a variable or a flag that clap does not read:
/// a password in it is hidden.
#[tokio::test]
async fn a_usage_error_of_the_settings_hides_the_password_of_an_address() {
    let db = TempDb::new();
    let address = "postgres://u:S3CRET@h/db";
    let mut env = db.env();
    env.insert("CICADA_LISTEN".into(), address.into());
    let listen = run(counter_app(), &env, &["myapp", "serve"]).await.unwrap_err();
    assert!(matches!(listen, AppError::Usage(_)), "{listen}");
    assert!(listen.to_string().starts_with("CICADA_LISTEN: invalid value 'postgres://u:***@h/db'"), "{listen}");
    assert!(!listen.to_string().contains("S3CRET"), "{listen}");

    let types = run(counter_app(), &db.env(), &["myapp", "serve", "--types", address]).await.unwrap_err();
    assert!(matches!(types, AppError::Usage(_)), "{types}");
    assert!(types.to_string().contains("does not host postgres://u:***@h/db"), "{types}");
    assert!(!types.to_string().contains("S3CRET"), "{types}");
    assert!(!db.path.exists(), "nothing started");
}

/// Scenario: Apostrophe in a clap-rejected argument. Scenario: Apostrophes in a settings error. Scenario: Adjacent
/// addresses. A raw `'` is legal in a password and clap quotes with `'`: the whole password is hidden all the same.
#[tokio::test]
async fn a_usage_error_hides_a_password_with_an_apostrophe_and_each_of_adjacent_addresses() {
    let db = TempDb::new();
    let line = ["myapp", "serve", "postgres://u:S3'CRET@h/db"];
    let Err(error) = run(counter_app(), &db.env(), &line).await else { panic!("a usage error") };
    assert!(matches!(error, AppError::Usage(_)) && error.exit_code() == 2, "{error}");
    let error = error.to_string();
    assert!(error.starts_with("unexpected argument 'postgres://u:***@h/db'"), "{error}");
    assert!(!error.contains("S3") && !error.contains("CRET"), "{error}");

    let mut env = db.env();
    env.insert("CICADA_LISTEN".into(), "postgres://o'neil:S3'CRET@h/db".into());
    let listen = run(counter_app(), &env, &["myapp", "serve"]).await.unwrap_err();
    assert!(matches!(listen, AppError::Usage(_)) && listen.exit_code() == 2, "{listen}");
    let listen = listen.to_string();
    assert!(listen.starts_with("CICADA_LISTEN: invalid value 'postgres://o'neil:***@h/db'"), "{listen}");
    assert!(!listen.contains("S3") && !listen.contains("CRET"), "{listen}");

    let line = ["myapp", "serve", "--types", "a://h,,b://u:secret@k"];
    let types = run(counter_app(), &db.env(), &line).await.unwrap_err();
    assert!(matches!(types, AppError::Usage(_)) && types.exit_code() == 2, "{types}");
    let types = types.to_string();
    assert!(types.starts_with("--types: an empty name in 'a://h,,b://u:***@k'"), "{types}");
    assert!(!types.contains("secret"), "{types}");
    assert!(!db.path.exists(), "nothing started");
}

/// A node setting that can be read and is not valid fails the start: exit code 1.
#[tokio::test]
async fn a_setting_that_is_not_valid_fails_the_start() {
    let db = TempDb::new();
    let error = run(counter_app(), &db.env(), &["myapp", "worker", "--concurrency", "0"]).await.unwrap_err();
    assert_eq!(error.exit_code(), 1, "{error}");
    assert!(error.to_string().contains("concurrency"), "{error}");
}

/// Scenario: Whether a node runs.
#[test]
fn node_command_reads_the_arguments_as_the_command_line_does() {
    let cases: [(&[&str], bool); 7] = [
        (&["myapp", "serve", "--store", "a.db"], true),
        (&["myapp", "worker"], true),
        (&["myapp", "get", "actors"], false),
        (&["myapp", "--help"], false),
        (&["myapp", "--context", "serve", "get", "actors"], false),
        (&["myapp", "serve", "--help"], false),
        (&["myapp", "serve", "--token", "x"], false),
    ];
    for (line, expected) in cases {
        assert_eq!(node_command(&args(line)), expected, "{line:?}");
    }
}

/// With `CICADA_PG_URL`: a worker handles a message on PostgreSQL, in a schema of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_handles_a_message_on_postgres() {
    let Ok(url) = std::env::var("CICADA_PG_URL") else {
        eprintln!("CICADA_PG_URL is not set; skipping");
        return;
    };
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let schema = format!("app_{:012x}", nanos & 0xffff_ffff_ffff);
    let mut env: Env = env_of(std::path::Path::new("unused.db"));
    env.insert("CICADA_STORE".into(), url.clone());
    env.insert("CICADA_STORE_SCHEMA".into(), schema.clone());
    env.insert("CICADA_CONCURRENCY".into(), "2".into());
    let started = Arc::new(AtomicBool::new(false));
    let flag = started.clone();
    let app = counter_app().on_start(move |_| async move {
        flag.store(true, Ordering::SeqCst);
        Ok(())
    });
    let worker = Running::spawn(app, &env, &["myapp", "worker"]);
    eventually("the worker started", || async { started.load(Ordering::SeqCst) || worker.is_finished() }).await;
    assert!(!worker.is_finished(), "the worker runs");

    let store = Store::postgres(&url, cicada::store::PostgresOptions::for_schema(&schema)).await.unwrap();
    let sender = ActorSystem::builder(store.clone()).build();
    let counter = Counter { name: "pg".into() };
    sender.create(&counter, &CounterState::default()).await.unwrap();
    let cicada::SendResult::Accepted(receipt) = sender.send(&counter, CounterMsg::Add { amount: 3 }).await.unwrap()
    else {
        panic!("accepted")
    };
    eventually("done", || async {
        matches!(sender.status(&receipt.id).await.unwrap(), Some(MessageStatus::Finished(o)) if o.status == OutcomeStatus::Done)
    })
    .await;
    store.close().await;
    worker.stop().await.unwrap();
}
