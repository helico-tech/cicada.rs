//! What the tests of the command line and of `serve` and `worker` share.

#![allow(dead_code)] // each test crate uses a part

use std::ffi::OsString;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cicada::prelude::*;
use cicada_app::{App, AppError, Env};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// An output that a test reads while the command still writes to it.
#[derive(Debug, Clone, Default)]
pub struct Out(Arc<Mutex<Vec<u8>>>);

impl Out {
    /// What was written so far.
    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Out {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `args` as the arguments of a process.
pub fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// An environment whose config file does not exist, so the user's own config never leaks into a test.
pub fn no_config() -> Env {
    Env::from([("CICADA_CONFIG".to_owned(), "/nonexistent/cicada/config.toml".to_owned())])
}

/// Runs `app` with `args` (the name of the program first) and `env`, and no stop; its output, or the error.
pub async fn run(app: App, env: &Env, list: &[&str]) -> Result<String, AppError> {
    let out = Out::default();
    app.run_with(args(list), env.clone(), out.clone(), std::future::pending()).await?;
    Ok(out.text())
}

/// A command that runs in the background until [`Running::stop`].
pub struct Running {
    /// What it wrote.
    pub out: Out,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), AppError>>,
}

impl Running {
    /// Starts `app` with `args` and `env`.
    pub fn spawn(app: App, env: &Env, list: &[&str]) -> Self {
        Self::spawn_to(app, env, list, Out::default())
    }

    /// Starts `app` with `args` and `env`, writing to `out`.
    pub fn spawn_to(app: App, env: &Env, list: &[&str], out: Out) -> Self {
        let (stop, stopped) = oneshot::channel::<()>();
        let shutdown = async move {
            let _ = stopped.await;
        };
        let task = tokio::spawn(app.run_with(args(list), env.clone(), out.clone(), shutdown));
        Self { out, stop: Some(stop), task }
    }

    /// Starts `serve` and waits until it takes requests; its base URL.
    pub async fn serve(app: App, env: &Env, list: &[&str]) -> (Self, String) {
        Self::serve_to(app, env, list, Out::default()).await
    }

    /// [`Self::serve`], writing to `out`.
    pub async fn serve_to(app: App, env: &Env, list: &[&str], out: Out) -> (Self, String) {
        let running = Self::spawn_to(app, env, list, out);
        let started = Instant::now();
        loop {
            // Only complete lines: the last one may still be arriving in pieces.
            let text = running.out.text();
            let complete = text.rsplit_once('\n').map_or("", |(done, _)| done);
            if let Some(line) = complete.lines().find(|l| l.starts_with("Listening on ")) {
                let base = line.trim_start_matches("Listening on ").to_owned();
                return (running, base);
            }
            if running.task.is_finished() {
                panic!("serve ended before it listened: {:?}", running.task.await.unwrap());
            }
            assert!(started.elapsed() < Duration::from_secs(20), "serve did not listen: {}", running.out.text());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Whether the command has ended.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Completes the stop that `run_with` was given and waits for the command to end.
    pub async fn stop(mut self) -> Result<(), AppError> {
        let _ = self.stop.take().unwrap().send(());
        self.task.await.unwrap()
    }

    /// Completes the stop without waiting.
    pub fn signal(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }

    /// Waits for the command to end.
    pub async fn ended(self) -> Result<(), AppError> {
        self.task.await.unwrap()
    }
}

/// Waits until `condition` holds, for up to 10 s.
pub async fn eventually<Fut: Future<Output = bool>>(what: &str, mut condition: impl FnMut() -> Fut) {
    let started = Instant::now();
    while !condition().await {
        assert!(started.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A SQLite file in a folder of its own, removed with it.
pub struct TempDb {
    _dir: tempfile::TempDir,
    pub path: PathBuf,
}

impl TempDb {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("actors.db");
        Self { _dir: dir, path }
    }

    /// The environment of a node on this file, with a short poll interval.
    pub fn env(&self) -> Env {
        env_of(&self.path)
    }
}

/// The environment of a node on the SQLite file `path`, with a short poll interval.
pub fn env_of(path: &Path) -> Env {
    let mut env = no_config();
    env.insert("CICADA_STORE".into(), path.display().to_string());
    env.insert("CICADA_POLL_INTERVAL".into(), "50ms".into());
    env
}

/// A port on the loopback address that nothing listens on (as far as this process knows).
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// type `counter`: a count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counter {
    pub name: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CounterMsg {
    Add {
        amount: i64,
    },
    /// Takes `millis` before it adds one.
    Hold {
        millis: u64,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CounterState {
    #[serde(default)]
    pub count: i64,
}

pub struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        match message {
            CounterMsg::Add { amount } => turn.state_mut().count += amount,
            CounterMsg::Hold { millis } => {
                tokio::time::sleep(Duration::from_millis(millis)).await;
                turn.state_mut().count += 1;
            }
        }
        Ok(())
    }
}

/// type `counter` with another contract: a store that registered it refuses `CounterActor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtherCounter {
    pub name: String,
}

impl ActorRef for OtherCounter {
    const TYPE: &'static str = "counter";
    type Message = OtherCounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum OtherCounterMsg {
    Reset,
}

pub struct OtherCounterActor;

impl Actor for OtherCounterActor {
    type Ref = OtherCounter;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: OtherCounterMsg) -> HandlerResult {
        Ok(())
    }
}

/// type `registry`, key `_`: a singleton that keeps no data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry;

impl ActorRef for Registry {
    const TYPE: &'static str = "registry";
    type Message = RegistryMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum RegistryMsg {
    Ping,
}

pub struct RegistryActor;

impl Actor for RegistryActor {
    type Ref = Registry;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: RegistryMsg) -> HandlerResult {
        Ok(())
    }
}
