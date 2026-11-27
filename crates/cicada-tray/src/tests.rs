//! Apply and Reconnect end to end: the Settings form, the menu's control and a worker restarted over HTTP.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::prelude::*;
use cicada_app::{App, Env};
use cicada_axum::actor_api;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::timeout;

use crate::menu::{Action, FailureKind, MenuControl};
use crate::settings::SettingsForm;
use crate::{Tray, TrayState};

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Work {
    id: String,
}

impl ActorRef for Work {
    const TYPE: &'static str = "work";
    type Message = Ping;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Other {
    id: String,
}

impl ActorRef for Other {
    const TYPE: &'static str = "other";
    type Message = Ping;
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(tag = "type")]
enum Ping {
    Ping,
}

struct WorkActor;

impl Actor for WorkActor {
    type Ref = Work;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: Ping) -> HandlerResult {
        Ok(())
    }
}

struct OtherActor;

impl Actor for OtherActor {
    type Ref = Other;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: Ping) -> HandlerResult {
        Ok(())
    }
}

async fn wait_for(state: &mut watch::Receiver<TrayState>, want: TrayState) {
    let result = timeout(Duration::from_secs(10), state.wait_for(|current| *current == want)).await;
    assert!(result.is_ok(), "expected {want:?}");
}

/// Scenario: Apply while connected.
#[tokio::test(start_paused = true)]
async fn apply_while_online_restarts_the_worker_with_the_new_arguments() {
    // Real sockets must answer before Tokio advances through client timeouts: a blocking task holds the paused clock.
    let (hold, release) = std::sync::mpsc::channel::<()>();
    tokio::task::spawn_blocking(move || {
        let _ = release.recv_timeout(Duration::from_secs(10));
    });
    let actors = TestActors::builder().with(|node| node.host(WorkActor)).build().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serving = tokio::spawn(async move { axum::serve(listener, actor_api(actors.system().clone())).await });
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, format!("current-context = \"prod\"\n[contexts.prod]\nserver = \"http://{address}\"\n"))
        .unwrap();
    let env = Env::from([("CICADA_CONFIG".to_owned(), config.to_str().unwrap().to_owned())]);
    let types = Arc::new(Mutex::new(Vec::new()));
    let seen = types.clone();
    let tray = Tray::new(move || {
        let seen = seen.clone();
        App::new().host(WorkActor).host(OtherActor).on_worker_ready(move |ready| {
            *seen.lock().unwrap() = ready.types.clone();
        })
    })
    .with_env(env.clone());
    let mut state = tray.state();
    let online = TrayState::Online { context: "prod".into() };

    tray.connect("prod");
    wait_for(&mut state, online.clone()).await;
    assert_eq!(*types.lock().unwrap(), ["work", "other"]);
    assert_eq!(*tray.control.args.lock().unwrap(), args(&[]), "no saved settings yet");

    let mut form = SettingsForm::load(&env, &["work", "other"], Some("prod")).unwrap();
    form.edited.types = vec!["other".into()];
    form.edited.concurrency = "3".into();
    let name = form.apply().unwrap();
    let mut control = MenuControl::default();
    let current = state.borrow().clone();
    assert_eq!(control.apply(&current, name), Some(Action::Disconnect));
    tray.disconnect();
    wait_for(&mut state, TrayState::Offline).await;
    let Some(Action::Connect(name)) = control.tick(&TrayState::Offline) else { panic!("a reconnect") };
    tray.connect(name);
    wait_for(&mut state, online).await;
    assert_eq!(*types.lock().unwrap(), ["other"]);
    assert_eq!(*tray.control.args.lock().unwrap(), args(&["--types=other", "--concurrency=3"]));

    let details = tray.control.details(&tray.control.connection.lock().unwrap());
    assert_eq!(details.ready.map(|ready| ready.concurrency), Some(3), "the menu's \"N of M\"");
    assert!(details.stopping.is_none() && details.failed_at.is_none());

    tray.disconnect();
    tray.stop_now();
    let details = tray.control.details(&tray.control.connection.lock().unwrap());
    assert!(matches!(details.stopping, Some((_, true))), "the stop began and Stop Now was used");
    wait_for(&mut state, TrayState::Offline).await;
    serving.abort();
    drop(hold);
}

/// The menu's failure row: the tray records when a connection failed and its kind, until the next connection ends.
#[tokio::test(start_paused = true)]
async fn a_failed_connection_records_when_it_failed() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "[contexts.bad]\nserver = \"ftp://actors.internal\"\n").unwrap();
    let env = Env::from([("CICADA_CONFIG".to_owned(), config.to_str().unwrap().to_owned())]);
    let tray = Tray::new(|| App::new().host(WorkActor)).with_env(env);
    let mut state = tray.state();
    let before = std::time::SystemTime::now();
    tray.connect("bad");
    // Release the watch's borrow at once: `finish` below writes the state.
    let failed = timeout(Duration::from_secs(10), state.wait_for(|s| matches!(s, TrayState::Failed { .. }))).await;
    assert!(failed.is_ok_and(|failed| failed.is_ok()), "a server URL that is not http fails");
    let details = tray.control.details(&tray.control.connection.lock().unwrap());
    let failed_at = details.failed_at.expect("the time of the failure");
    assert_eq!(details.failure_kind, Some(FailureKind::Other), "a refused URL scheme has no typed kind");
    assert!(failed_at >= cicada::time::Timestamp::from(before));
    tray.control.finish(TrayState::Offline, None);
    assert!(tray.control.details(&tray.control.connection.lock().unwrap()).failed_at.is_none());
}

/// The worker command's arguments for context `prod` with `extra`.
fn args(extra: &[&str]) -> Vec<String> {
    let bin = std::env::args_os().next().unwrap().to_string_lossy().into_owned();
    [bin.as_str(), "worker", "--context=prod"].iter().chain(extra).map(|arg| arg.to_string()).collect()
}

/// A usage error and a failed command whose chain holds a typed refusal: the kinds the menu shows.
#[test]
fn a_failed_command_gets_its_kind_from_its_chain() {
    let refused = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "x");
    let error = anyhow::Error::new(refused).context("the server is not ready");
    assert_eq!(crate::failure_kind(&cicada_app::AppError::Failed(error)), FailureKind::Refused);
    let usage = cicada_app::AppError::Usage("--server postgres://user:secret@host".into());
    assert_eq!(crate::failure_kind(&usage), FailureKind::Other);
}
