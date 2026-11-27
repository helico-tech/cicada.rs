//! The tray and its HTTP worker against the actor API on a loopback socket.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use cicada::prelude::*;
use cicada::store::{MessageStatus, OutcomeStatus, TurnOutcome};
use cicada_app::{App, Env};
use cicada_axum::actor_api;
use cicada_test::TestActors;
use cicada_tray::{RunningTurn, Tray, TrayState, TurnResult};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Work {
    id: String,
}

impl ActorRef for Work {
    const TYPE: &'static str = "work";
    type Message = WorkMsg;
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(tag = "type")]
enum WorkMsg {
    Wait,
    Finish,
}

#[derive(Clone)]
struct WorkActor {
    began: Arc<Notify>,
    finish: watch::Sender<bool>,
}

impl WorkActor {
    fn new() -> Self {
        Self { began: Arc::default(), finish: watch::channel(false).0 }
    }
}

impl Actor for WorkActor {
    type Ref = Work;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, msg: WorkMsg) -> HandlerResult {
        if matches!(msg, WorkMsg::Wait) {
            self.began.notify_one();
            self.finish.subscribe().wait_for(|open| *open).await.unwrap();
        }
        Ok(())
    }
}

struct Config {
    file: tempfile::NamedTempFile,
}

impl Config {
    fn new(address: std::net::SocketAddr) -> Self {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), format!("[contexts.prod]\nserver = \"http://{address}\"\n")).unwrap();
        Self { file }
    }

    fn env(&self) -> Env {
        Env::from([("CICADA_CONFIG".into(), self.file.path().to_str().unwrap().into())])
    }
}

struct Server {
    actors: TestActors,
    config: Config,
    /// Whether `POST /leases/release` is answered; while false it waits.
    releases: watch::Sender<bool>,
    serving: JoinHandle<()>,
    _hold_clock: std::sync::mpsc::Sender<()>,
}

impl Server {
    async fn start(actor: WorkActor) -> Self {
        // Real sockets must get a chance to answer before Tokio advances through client timeouts. A blocking task
        // holds the paused clock while requests are in flight. Tests advance it explicitly where timing matters.
        let (hold, release) = std::sync::mpsc::channel::<()>();
        tokio::task::spawn_blocking(move || {
            let _ = release.recv_timeout(TEST_TIMEOUT);
        });
        let actors = TestActors::builder().with(|node| node.host(actor)).build().await.unwrap();
        let releases = watch::channel(true).0;
        let open = releases.clone();
        let routes: Router = actor_api(actors.system().clone()).layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let mut open = open.subscribe();
                async move {
                    if request.uri().path() == "/leases/release" {
                        let _ = open.wait_for(|open| *open).await;
                    }
                    next.run(request).await
                }
            },
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Config::new(listener.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        Self { actors, config, releases, serving, _hold_clock: hold }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.serving.abort();
    }
}

async fn wait_for(state: &mut watch::Receiver<TrayState>, want: TrayState) {
    let result = timeout(TEST_TIMEOUT, state.wait_for(|current| *current == want)).await;
    assert!(result.is_ok(), "expected {want:?}");
    result.unwrap().unwrap();
}

async fn failed(state: &mut watch::Receiver<TrayState>, context: &str, limit: Duration) -> String {
    let failed = timeout(limit, state.wait_for(|s| matches!(s, TrayState::Failed { .. }))).await.unwrap().unwrap();
    let TrayState::Failed { context: actual, error } = &*failed else { unreachable!() };
    assert_eq!(actual, context);
    error.clone()
}

fn connecting(context: &str) -> TrayState {
    TrayState::Connecting { context: context.into() }
}

fn online() -> TrayState {
    TrayState::Online { context: "prod".into() }
}

#[tokio::test(start_paused = true)]
async fn an_idle_worker_goes_online_and_a_reconnect_builds_a_fresh_app() {
    let actor = WorkActor::new();
    let server = Server::start(actor.clone()).await;
    let builds = Arc::new(AtomicUsize::new(0));
    let tray = Tray::new({
        let builds = builds.clone();
        move || {
            builds.fetch_add(1, Ordering::Relaxed);
            App::new().host(actor.clone())
        }
    })
    .with_env(server.config.env());
    let mut state = tray.state();
    assert_eq!(*state.borrow(), TrayState::Offline);
    tray.connect("prod");
    assert_eq!(*state.borrow(), connecting("prod"));
    tray.connect("elsewhere");
    wait_for(&mut state, online()).await;
    tray.connect("elsewhere");
    assert_eq!(*state.borrow(), online());

    tray.disconnect();
    let stopping = state.borrow().clone();
    assert!(matches!(stopping, TrayState::Stopping { ref context, .. } if context == "prod"));
    tray.connect("elsewhere");
    tray.disconnect();
    assert_eq!(*state.borrow(), stopping);
    wait_for(&mut state, TrayState::Offline).await;
    assert_eq!(builds.load(Ordering::Relaxed), 1);

    tray.connect("prod");
    wait_for(&mut state, online()).await;
    assert_eq!(builds.load(Ordering::Relaxed), 2);
    tray.disconnect();
    wait_for(&mut state, TrayState::Offline).await;
}

#[tokio::test(start_paused = true)]
async fn disconnect_uses_the_settled_settings_and_drains_a_running_turn() {
    let actor = WorkActor::new();
    let server = Server::start(actor.clone()).await;
    let work = Work { id: "gated".into() };
    server.actors.create(&work, &NoState {}).await;
    let message = server.actors.send(&work, WorkMsg::Wait).await;
    let host = cicada::runtime::NodeConfig {
        shutdown_grace_period: Duration::from_secs(30),
        shutdown_cancel_timeout: Duration::from_secs(3),
        ..Default::default()
    };
    let mut env = server.config.env();
    env.insert("CICADA_SHUTDOWN_GRACE_PERIOD".into(), "PT7S".into());
    let host_actor = actor.clone();
    let tray = Tray::new(move || App::new().host(host_actor.clone()).node_config(host.clone())).with_env(env);
    let mut state = tray.state();
    let mut changes = tray.turn_changes();
    assert_eq!(*changes.borrow(), 0);
    tray.connect("prod");
    wait_for(&mut state, online()).await;
    timeout(TEST_TIMEOUT, actor.began.notified()).await.unwrap();
    assert_eq!(*changes.borrow_and_update(), 1);
    let turns = tray.turns();
    let running = RunningTurn {
        actor: work.address().unwrap(),
        message_type: "Wait".into(),
        attempt: 1,
        claimed_at: server.actors.now(),
    };
    assert_eq!(turns.running, std::slice::from_ref(&running));
    assert!(turns.finished.is_empty());

    let until = Instant::now() + Duration::from_secs(10);
    tray.disconnect();
    assert_eq!(*state.borrow(), TrayState::Stopping { context: "prod".into(), until });
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(*state.borrow(), TrayState::Stopping { context: "prod".into(), until });
    assert_eq!(tray.turns().running, std::slice::from_ref(&running));
    actor.finish.send_replace(true);
    wait_for(&mut state, TrayState::Offline).await;
    assert!(changes.has_changed().unwrap());
    assert_eq!(*changes.borrow(), 2);
    let turns = tray.turns();
    assert!(turns.running.is_empty());
    assert_eq!(turns.finished.len(), 1);
    assert_eq!(turns.finished[0].turn, running);
    assert_eq!(turns.finished[0].outcome, TurnResult::Committed(TurnOutcome::Done));
    let Some(MessageStatus::Finished(outcome)) = server.actors.system().status(&message.id).await.unwrap() else {
        panic!("the turn did not finish");
    };
    assert_eq!(outcome.status, OutcomeStatus::Done);
}

#[tokio::test(start_paused = true)]
async fn a_turn_cancelled_at_shutdown_is_abandoned() {
    let actor = WorkActor::new();
    let server = Server::start(actor.clone()).await;
    let work = Work { id: "gated".into() };
    server.actors.create(&work, &NoState {}).await;
    let message = server.actors.send(&work, WorkMsg::Wait).await;
    let host_actor = actor.clone();
    let tray = Tray::new(move || {
        App::new()
            .host(host_actor.clone())
            .node_config(cicada::runtime::NodeConfig { shutdown_grace_period: Duration::ZERO, ..Default::default() })
    })
    .with_env(server.config.env());
    let mut state = tray.state();
    let mut changes = tray.turn_changes();
    tray.connect("prod");
    timeout(TEST_TIMEOUT, actor.began.notified()).await.unwrap();
    let running = tray.turns().running.pop().unwrap();
    tray.disconnect();
    for _ in 0..10 {
        tokio::time::advance(Duration::from_millis(1)).await;
    }
    wait_for(&mut state, TrayState::Offline).await;
    changes.changed().await.unwrap();
    assert_eq!(*changes.borrow(), 2);
    let turns = tray.turns();
    assert!(turns.running.is_empty());
    assert_eq!(turns.finished.len(), 1);
    assert_eq!(turns.finished[0].turn, running);
    assert_eq!(turns.finished[0].outcome, TurnResult::Abandoned);
    assert!(matches!(server.actors.system().status(&message.id).await.unwrap(), Some(MessageStatus::Pending(_))));
}

#[tokio::test(start_paused = true)]
async fn the_history_evicts_the_oldest_finished_turns() {
    let actor = WorkActor::new();
    actor.finish.send_replace(true);
    let server = Server::start(actor.clone()).await;
    let work = Work { id: "ring".into() };
    server.actors.create(&work, &NoState {}).await;
    for msg in [WorkMsg::Wait, WorkMsg::Wait].into_iter().chain((0..50).map(|_| WorkMsg::Finish)) {
        server.actors.send(&work, msg).await;
    }
    let tray = Tray::new(move || App::new().host(actor.clone())).with_env(server.config.env());
    let mut state = tray.state();
    let mut changes = tray.turn_changes();
    tray.connect("prod");
    timeout(TEST_TIMEOUT, changes.wait_for(|count| *count == 104)).await.unwrap().unwrap();
    let turns = tray.turns();
    assert!(turns.running.is_empty());
    assert_eq!(turns.finished.len(), 50);
    assert!(
        turns
            .finished
            .iter()
            .all(|turn| turn.turn.message_type == "Finish" && turn.outcome == TurnResult::Committed(TurnOutcome::Done))
    );
    tray.disconnect();
    wait_for(&mut state, TrayState::Offline).await;
}

#[tokio::test(start_paused = true)]
async fn disconnect_before_readiness_interrupts_startup() {
    let actor = WorkActor::new();
    let server = Server::start(actor.clone()).await;
    let tray = Tray::new(move || App::new().host(actor.clone())).with_env(server.config.env());
    let mut state = tray.state();
    tray.connect("prod");
    assert_eq!(*state.borrow(), connecting("prod"));
    let until = Instant::now();
    tray.disconnect();
    assert_eq!(*state.borrow(), TrayState::Stopping { context: "prod".into(), until });
    wait_for(&mut state, TrayState::Offline).await;
}

#[tokio::test(start_paused = true)]
async fn shutting_down_the_worker_runtime_before_its_first_poll_allows_reconnect() {
    let actor = WorkActor::new();
    let server = Server::start(actor.clone()).await;
    let tray = Arc::new(Tray::new(move || App::new().host(actor.clone())).with_env(server.config.env()));
    let mut state = tray.state();
    let worker_tray = tray.clone();
    tokio::task::spawn_blocking(move || {
        // This runtime never drives its queue. Shutting it down drops the unpolled worker task.
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let _entered = runtime.enter();
        worker_tray.connect("prod");
        assert_eq!(*worker_tray.state().borrow(), connecting("prod"));
    })
    .await
    .unwrap();
    assert_eq!(
        *state.borrow(),
        TrayState::Failed {
            context: "prod".into(),
            error: "the HTTP worker task was cancelled before it returned".into()
        }
    );

    tray.connect("prod");
    wait_for(&mut state, online()).await;
    tray.disconnect();
    wait_for(&mut state, TrayState::Offline).await;
}

#[tokio::test(start_paused = true)]
async fn a_panicking_app_factory_reports_failure_and_allows_reconnect() {
    let actor = WorkActor::new();
    let server = Server::start(actor.clone()).await;
    let first = AtomicBool::new(true);
    let tray = Tray::new(move || {
        assert!(!first.swap(false, Ordering::Relaxed), "cannot build the app");
        App::new().host(actor.clone())
    })
    .with_env(server.config.env());
    let mut state = tray.state();
    tray.connect("prod");
    assert_eq!(failed(&mut state, "prod", TEST_TIMEOUT).await, "the App factory panicked");

    tray.connect("prod");
    wait_for(&mut state, online()).await;
    tray.disconnect();
    wait_for(&mut state, TrayState::Offline).await;
}

#[tokio::test(start_paused = true)]
async fn a_panicking_worker_task_reports_failure() {
    #[derive(Deserialize, Serialize)]
    struct BrokenState;

    impl JsonSchema for BrokenState {
        fn schema_name() -> std::borrow::Cow<'static, str> {
            "BrokenState".into()
        }

        fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
            panic!("cannot build the actor's schema");
        }
    }

    struct BrokenActor;

    impl Actor for BrokenActor {
        type Ref = Work;
        type State = BrokenState;

        async fn handle(&self, _: &mut Turn<Self>, _: WorkMsg) -> HandlerResult {
            unreachable!("the actor never starts");
        }
    }

    let server = Server::start(WorkActor::new()).await;
    let tray = Tray::new(|| App::new().host(BrokenActor)).with_env(server.config.env());
    let mut state = tray.state();
    tray.connect("prod");
    assert_eq!(failed(&mut state, "prod", TEST_TIMEOUT).await, "the HTTP worker task panicked");
}

#[tokio::test(start_paused = true)]
async fn a_bad_context_fails_and_can_be_followed_by_a_new_connection() {
    let actor = WorkActor::new();
    let server = Server::start(actor.clone()).await;
    let tray = Tray::new(move || App::new().host(actor.clone())).with_env(server.config.env());
    let mut state = tray.state();
    tray.connect("--help");
    let error = failed(&mut state, "--help", TEST_TIMEOUT).await;
    assert!(error.contains("context") && error.contains("--help"), "{error}");
    tray.connect("prod");
    wait_for(&mut state, online()).await;
    tray.disconnect();
    wait_for(&mut state, TrayState::Offline).await;
}

#[tokio::test(start_paused = true)]
async fn app_start_hooks_and_tasks_report_the_worker_commands_refusal() {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let config = Config::new(socket.local_addr().unwrap());
    for task in [false, true] {
        let tray = Tray::new(move || {
            let app = App::new().host(WorkActor::new());
            if task {
                app.task("bootstrap", |_, _| async { panic!("the task must be refused") })
            } else {
                app.on_start(|_| async { panic!("the startup hook must be refused") })
            }
        })
        .with_env(config.env());
        let mut state = tray.state();
        tray.connect("prod");
        let error = failed(&mut state, "prod", TEST_TIMEOUT).await;
        assert!(error.contains("cannot run App::on_start or App::task"), "{error}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_server_stays_connecting_during_retries() {
    let socket = TcpSocket::new_v4().unwrap();
    // Reserve a port without listening, so a connection to it is refused.
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let config = Config::new(socket.local_addr().unwrap());
    let tray = Tray::new(|| App::new().host(WorkActor::new())).with_env(config.env());
    let mut state = tray.state();
    let start = Instant::now();
    tray.connect("prod");
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(*state.borrow(), connecting("prod"));
    let error = failed(&mut state, "prod", Duration::from_secs(65)).await;
    assert!(start.elapsed() >= Duration::from_secs(60));
    assert!(error.contains("the server is not ready"), "{error}");
}

/// The cancel timeout of [`online_in_a_long_turn`].
const CANCEL_TIMEOUT: Duration = Duration::from_secs(3);

/// A tray online whose worker, with a grace period of 30 s and a cancel timeout of 3 s, runs the turn `Wait` until
/// the actor's `finish` opens; the message's id.
async fn online_in_a_long_turn() -> (Server, Tray, watch::Receiver<TrayState>, WorkActor, cicada::store::MessageId) {
    let actor = WorkActor::new();
    let server = Server::start(actor.clone()).await;
    let work = Work { id: "gated".into() };
    server.actors.create(&work, &NoState {}).await;
    let message = server.actors.send(&work, WorkMsg::Wait).await;
    let host = cicada::runtime::NodeConfig {
        shutdown_grace_period: Duration::from_secs(30),
        shutdown_cancel_timeout: CANCEL_TIMEOUT,
        ..Default::default()
    };
    let host_actor = actor.clone();
    let tray =
        Tray::new(move || App::new().host(host_actor.clone()).node_config(host.clone())).with_env(server.config.env());
    let mut state = tray.state();
    tray.connect("prod");
    wait_for(&mut state, online()).await;
    timeout(TEST_TIMEOUT, actor.began.notified()).await.unwrap();
    (server, tray, state, actor, message.id)
}

fn stopping(until: Instant) -> TrayState {
    TrayState::Stopping { context: "prod".into(), until }
}

/// The finished turns are one abandoned turn, and its message is pending again.
async fn abandoned(server: &Server, tray: &Tray, id: &cicada::store::MessageId) {
    let turns = tray.turns();
    assert!(turns.running.is_empty());
    assert_eq!(turns.finished.iter().map(|turn| turn.outcome.clone()).collect::<Vec<_>>(), [TurnResult::Abandoned]);
    assert!(matches!(server.actors.system().status(id).await.unwrap(), Some(MessageStatus::Pending(_))));
}

/// Scenario: Stop Now during the grace period. `until` moves to now plus the cancel timeout, a second Stop Now keeps
/// it, and the tray is offline at the cancel timeout, not the grace period, even while the cancelled turn's release
/// is held: the turn is still listed as running then, and as abandoned once its release ends.
#[tokio::test(start_paused = true)]
async fn stop_now_while_stopping_a_ready_worker_is_offline_at_the_cancel_timeout_and_abandons_its_turn() {
    let (server, tray, mut state, _actor, id) = online_in_a_long_turn().await;
    let mut changes = tray.turn_changes();
    let stopped = Instant::now();
    tray.disconnect();
    assert_eq!(*state.borrow(), stopping(stopped + Duration::from_secs(33)));
    tokio::time::advance(Duration::from_secs(1)).await;
    // Held, the release keeps the worker stopping while time moves on.
    server.releases.send_replace(false);
    let asked = Instant::now();
    tray.stop_now();
    assert_eq!(*state.borrow(), stopping(asked + CANCEL_TIMEOUT));
    // Let the worker take it before the clock moves.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    tray.stop_now();
    assert_eq!(*state.borrow(), stopping(asked + CANCEL_TIMEOUT), "a second Stop Now never moves it later");
    // The release stays held through the cancel timeout: the worker leaves the turn behind and returns.
    tokio::time::advance(CANCEL_TIMEOUT - Duration::from_secs(1)).await;
    wait_for(&mut state, TrayState::Offline).await;
    assert_eq!(asked.elapsed(), CANCEL_TIMEOUT, "offline at the cancel timeout");
    assert_eq!(tray.turns().running.len(), 1, "listed as running until its release ends");
    server.releases.send_replace(true);
    timeout(TEST_TIMEOUT, async {
        while !tray.turns().running.is_empty() {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    abandoned(&server, &tray, &id).await;
}

/// Stop Now at once after the disconnect, before the worker itself is stopping, is kept.
#[tokio::test(start_paused = true)]
async fn stop_now_right_after_disconnect_still_ends_within_the_cancel_timeout() {
    let (server, tray, mut state, _actor, id) = online_in_a_long_turn().await;
    let asked = Instant::now();
    tray.disconnect();
    tray.stop_now();
    assert_eq!(*state.borrow(), stopping(asked + CANCEL_TIMEOUT));
    for _ in 0..10 {
        tokio::time::advance(Duration::from_millis(1)).await;
    }
    wait_for(&mut state, TrayState::Offline).await;
    assert!(asked.elapsed() < CANCEL_TIMEOUT, "offline after {:?}", asked.elapsed());
    abandoned(&server, &tray, &id).await;
}

/// Scenario: Stop Now while online. Also offline, failed, and while stopping a worker that was never ready.
#[tokio::test(start_paused = true)]
async fn stop_now_does_nothing_unless_stopping_a_ready_worker() {
    let (server, tray, mut state, actor, _id) = online_in_a_long_turn().await;
    tray.stop_now();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(*state.borrow(), online());
    assert_eq!(tray.turns().running.len(), 1, "the turn runs on");
    actor.finish.send_replace(true);
    tray.disconnect();
    wait_for(&mut state, TrayState::Offline).await;
    tray.stop_now();
    assert_eq!(*state.borrow(), TrayState::Offline);

    tray.connect("--help");
    failed(&mut state, "--help", TEST_TIMEOUT).await;
    let failed = state.borrow().clone();
    tray.stop_now();
    assert_eq!(*state.borrow(), failed);

    let fresh = Tray::new(move || App::new().host(WorkActor::new())).with_env(server.config.env());
    let mut state = fresh.state();
    fresh.connect("prod");
    let until = Instant::now();
    fresh.disconnect();
    fresh.stop_now();
    assert_eq!(*state.borrow(), stopping(until), "never ready");
    wait_for(&mut state, TrayState::Offline).await;
}
