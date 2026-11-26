//! The tray and its HTTP worker against the actor API on a loopback socket.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use cicada::prelude::*;
use cicada::store::{MessageStatus, OutcomeStatus};
use cicada_app::{App, Env};
use cicada_axum::actor_api;
use cicada_test::TestActors;
use cicada_tray::{Tray, TrayState};
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

    async fn handle(&self, _: &mut Turn<Self>, _: WorkMsg) -> HandlerResult {
        self.began.notify_one();
        self.finish.subscribe().wait_for(|open| *open).await.unwrap();
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
        let routes: Router = actor_api(actors.system().clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Config::new(listener.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        Self { actors, config, serving, _hold_clock: hold }
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
    tray.connect("prod");
    wait_for(&mut state, online()).await;
    timeout(TEST_TIMEOUT, actor.began.notified()).await.unwrap();

    let until = Instant::now() + Duration::from_secs(10);
    tray.disconnect();
    assert_eq!(*state.borrow(), TrayState::Stopping { context: "prod".into(), until });
    actor.finish.send_replace(true);
    wait_for(&mut state, TrayState::Offline).await;
    let Some(MessageStatus::Finished(outcome)) = server.actors.system().status(&message.id).await.unwrap() else {
        panic!("the turn did not finish");
    };
    assert_eq!(outcome.status, OutcomeStatus::Done);
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
