//! Controls a Cicada worker over HTTP in the host process. The model can drive a menu bar or other UI.

#[allow(dead_code)] // AppKit uses the menu model in the next task.
mod menu;
mod turns;

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use cicada_app::{App, AppError, Env, WorkerReady};
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use turns::TurnHistory;
pub use turns::{FinishedTurn, RunningTurn, TurnResult, TurnSnapshot};

/// State of the tray's worker connection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TrayState {
    /// No worker is running.
    #[default]
    Offline,
    /// The worker is checking the server and starting.
    Connecting {
        /// Name of the selected app context.
        context: String,
    },
    /// The worker has started and can claim turns.
    Online {
        /// Name of the selected app context.
        context: String,
    },
    /// A stop was requested and the worker has yet to return.
    Stopping {
        /// Name of the selected app context.
        context: String,
        /// Stop time plus the worker's grace period and cancel timeout, on Tokio's clock. When startup was
        /// interrupted before the worker was ready, this is the stop time.
        until: Instant,
    },
    /// The command failed or its task ended unexpectedly. A new connection can be started.
    Failed {
        /// Name of the selected app context.
        context: String,
        /// Why the connection failed.
        error: String,
    },
}

/// Connects and disconnects an in-process worker made from a fresh [`App`] for each connection.
pub struct Tray {
    factory: Box<dyn Fn() -> App + Send + Sync>,
    env: Option<Env>,
    control: Arc<Control>,
    turns: TurnHistory,
}

struct Control {
    // Serializes accepting a connect, publishing state and signalling a stop.
    connection: Mutex<Option<Connection>>,
    state: watch::Sender<TrayState>,
}

struct Connection {
    context: String,
    stop: Option<oneshot::Sender<()>>,
    ready: Option<WorkerReady>,
    stopping_at: Option<Instant>,
}

struct WorkerTaskGuard {
    control: Arc<Control>,
    context: Option<String>,
}

impl Tray {
    /// Makes an offline tray. The factory is called for each accepted connection.
    pub fn new(factory: impl Fn() -> App + Send + Sync + 'static) -> Self {
        Self {
            factory: Box::new(factory),
            env: None,
            control: Arc::new(Control { connection: Mutex::new(None), state: watch::channel(TrayState::Offline).0 }),
            turns: TurnHistory::default(),
        }
    }

    /// Uses this environment for each worker command in place of the process environment.
    pub fn with_env(mut self, env: Env) -> Self {
        self.env = Some(env);
        self
    }

    /// Watches the current connection state. Read the receiver's current value on subscription and release the
    /// borrow before calling [`Self::connect`] or [`Self::disconnect`].
    pub fn state(&self) -> watch::Receiver<TrayState> {
        self.control.state.subscribe()
    }

    /// Returns a snapshot of running and recently finished turns across this tray's connections.
    pub fn turns(&self) -> TurnSnapshot {
        self.turns.snapshot()
    }

    /// Watches a counter that advances at each turn start and finish. Read [`Self::turns`] to redraw.
    pub fn turn_changes(&self) -> watch::Receiver<u64> {
        self.turns.changes()
    }

    /// Starts a worker using the named context on the caller's Tokio runtime. Calls while starting, online or
    /// stopping have no effect.
    ///
    /// # Panics
    ///
    /// Panics when starting a connection outside a Tokio runtime.
    pub fn connect(&self, context: impl Into<String>) {
        let context = context.into();
        let (runtime, stopped) = {
            let mut connection = self.control.connection.lock().expect("tray connection");
            if connection.is_some() {
                return;
            }
            // Use try_current so a missing runtime cannot poison the connection lock.
            let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                drop(connection);
                panic!("Tray::connect requires a Tokio runtime");
            };
            let (stop, stopped) = oneshot::channel();
            *connection =
                Some(Connection { context: context.clone(), stop: Some(stop), ready: None, stopping_at: None });
            self.control.state.send_replace(TrayState::Connecting { context: context.clone() });
            (runtime, stopped)
        };
        let Ok(app) = catch_unwind(AssertUnwindSafe(|| (self.factory)())) else {
            self.control.finish(TrayState::Failed { context, error: "the App factory panicked".to_owned() });
            return;
        };
        let ready = self.control.clone();
        let app = app.observe(self.turns.clone()).on_worker_ready(move |settings| ready.worker_ready(settings));
        let args = vec![
            std::env::args_os().next().unwrap_or_else(|| "cicada-tray".into()),
            "worker".into(),
            format!("--context={context}").into(),
        ];
        let env = self.env.clone().unwrap_or_else(|| {
            std::env::vars_os()
                .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
                .collect()
        });
        // Capture the guard before spawning so even a task dropped before its first poll clears the connection.
        let task = WorkerTaskGuard { control: self.control.clone(), context: Some(context) };
        runtime.spawn(async move {
            let result = app
                .run_with(args, env, io::sink(), async {
                    let _ = stopped.await;
                })
                .await;
            task.complete(result);
        });
    }

    /// Requests a graceful stop. The state stays [`TrayState::Stopping`] until the command returns.
    pub fn disconnect(&self) {
        let mut connection = self.control.connection.lock().expect("tray connection");
        let Some(connection) = connection.as_mut() else { return };
        let Some(stop) = connection.stop.take() else { return };
        let now = Instant::now();
        connection.stopping_at = Some(now);
        self.control.state.send_replace(connection.stopping(now));
        let _ = stop.send(());
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.disconnect();
    }
}

impl Control {
    fn worker_ready(&self, ready: WorkerReady) {
        let mut connection = self.connection.lock().expect("tray connection");
        let connection = connection.as_mut().expect("the worker is still running");
        connection.ready = Some(ready);
        self.state.send_replace(match connection.stopping_at {
            Some(at) => connection.stopping(at),
            None => TrayState::Online { context: connection.context.clone() },
        });
    }

    fn finish(&self, state: TrayState) {
        let mut connection = self.connection.lock().expect("tray connection");
        *connection = None;
        self.state.send_replace(state);
    }
}

impl WorkerTaskGuard {
    fn complete(mut self, result: Result<(), AppError>) {
        let context = self.context.take().expect("the worker task has a context");
        self.control.finish(match result {
            Ok(()) => TrayState::Offline,
            Err(error) => TrayState::Failed { context, error: error.to_string() },
        });
    }
}

impl Drop for WorkerTaskGuard {
    fn drop(&mut self) {
        let Some(context) = self.context.take() else { return };
        let error = if std::thread::panicking() {
            "the HTTP worker task panicked"
        } else {
            "the HTTP worker task was cancelled before it returned"
        };
        self.control.finish(TrayState::Failed { context, error: error.to_owned() });
    }
}

impl Connection {
    fn stopping(&self, at: Instant) -> TrayState {
        let until = self.ready.as_ref().map_or(at, |ready| {
            at.checked_add(ready.grace_period.saturating_add(ready.cancel_timeout))
                .or_else(|| at.checked_add(ready.grace_period.max(ready.cancel_timeout)))
                .unwrap_or(at)
        });
        TrayState::Stopping { context: self.context.clone(), until }
    }
}
