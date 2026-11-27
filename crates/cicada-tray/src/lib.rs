//! Controls a Cicada worker over HTTP in the host process, with a native menu bar app on macOS.

#[cfg(target_os = "macos")]
mod appkit;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod menu;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod settings;
#[cfg(test)]
mod tests;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod tray_file;
mod turns;

use std::io;
use std::process::ExitCode;
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
        /// Stop time plus the worker's grace period and cancel timeout, on Tokio's clock; after
        /// [`Tray::stop_now`], at most its time plus the cancel timeout. When startup was interrupted before the
        /// worker was ready, this is the stop time.
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
    factory: Arc<dyn Fn() -> App + Send + Sync>,
    env: Option<Env>,
    control: Arc<Control>,
    turns: TurnHistory,
}

struct Control {
    // Serializes accepting a connect, publishing state and signalling a stop.
    connection: Mutex<Option<Connection>>,
    state: watch::Sender<TrayState>,
    #[cfg(target_os = "macos")]
    registered_types: Mutex<Vec<&'static str>>,
    /// The arguments of the last worker command, for tests.
    #[cfg(test)]
    args: Mutex<Vec<String>>,
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
            factory: Arc::new(factory),
            env: None,
            control: Arc::new(Control {
                connection: Mutex::new(None),
                state: watch::channel(TrayState::Offline).0,
                #[cfg(target_os = "macos")]
                registered_types: Mutex::default(),
                #[cfg(test)]
                args: Mutex::default(),
            }),
            turns: TurnHistory::default(),
        }
    }

    /// Uses this environment for each worker command in place of the process environment.
    pub fn with_env(mut self, env: Env) -> Self {
        self.env = Some(env);
        self
    }

    /// Runs the menu bar app, initially offline. Call this on the main thread on macOS 14 or later.
    ///
    /// The factory is also called at startup to list the registered types before the first connection.
    /// Quit, Ctrl-C and system logout wait for the worker to stop, which can take its full grace period and
    /// cancel timeout. A system termination ends the process through AppKit. On unsupported platforms or if
    /// startup fails, prints an error and returns exit code 1.
    pub fn run(self) -> ExitCode {
        #[cfg(target_os = "macos")]
        {
            appkit::run(self)
        }
        #[cfg(not(target_os = "macos"))]
        {
            eprintln!("the Cicada menu bar app requires macOS 14 or later");
            ExitCode::FAILURE
        }
    }

    /// Watches the current connection state. Read the receiver's current value on subscription and release the
    /// borrow before calling [`Self::connect`], [`Self::disconnect`] or [`Self::stop_now`].
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
    /// When `tray.toml` next to the config file has settings for the context, the worker gets `--types` with the
    /// saved types the app still registers (`all` when that is every one) and `--concurrency`, which win over
    /// `CICADA_TYPES` and `CICADA_CONCURRENCY`. A saved selection of which no registered type is left, or that
    /// `--types` cannot express, ends [`TrayState::Failed`] with an error that says to open Settings.
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
        let factory = self.factory.clone();
        let ready = self.control.clone();
        let turns = self.turns.clone();
        let mut args = vec![
            std::env::args_os().next().unwrap_or_else(|| "cicada-tray".into()),
            "worker".into(),
            format!("--context={context}").into(),
        ];
        let env = self.environment();
        let saved = (env.clone(), context.clone());
        // Capture the guard before spawning so even a task dropped before its first poll clears the connection.
        let task = WorkerTaskGuard { control: self.control.clone(), context: Some(context) };
        let app = runtime.spawn_blocking(move || {
            let app = factory();
            let (env, context) = saved;
            let saved = tray_file::saved_args(&env, &context, &app.type_names());
            (app, saved)
        });
        runtime.spawn(async move {
            let (app, saved) = match app.await {
                Ok(built) => built,
                Err(error) => {
                    let message =
                        if error.is_panic() { "the App factory panicked" } else { "the App factory was cancelled" };
                    task.fail(message.to_owned());
                    return;
                }
            };
            #[cfg(target_os = "macos")]
            ready.set_registered_types(&app);
            match saved {
                Ok(saved) => args.extend(saved.into_iter().map(Into::into)),
                Err(error) => {
                    task.fail(error);
                    return;
                }
            }
            #[cfg(test)]
            {
                *ready.args.lock().expect("tray args") =
                    args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
            }
            let app = app.observe(turns).on_worker_ready(move |settings| ready.worker_ready(settings));
            let result = app
                .run_with(args, env, io::sink(), async {
                    let _ = stopped.await;
                })
                .await;
            task.complete(result);
        });
    }

    fn environment(&self) -> Env {
        self.env.clone().unwrap_or_else(|| {
            std::env::vars_os()
                .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
                .collect()
        })
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

    /// Stop Now: while [`TrayState::Stopping`] a worker that was ready, ends its grace period, so its running turns are
    /// cancelled at once, and moves `until` to now plus the cancel timeout when that is earlier. A turn that ends
    /// without an acknowledged commit, also after a failed lease release, is listed as abandoned; one whose handler had
    /// ended may still commit. The state stays `Stopping` until the command returns, within the cancel timeout, and
    /// then becomes [`TrayState::Offline`]: a turn the worker left behind, such as one whose release is still in
    /// flight, stays listed as running until it ends. In any other state, or while stopping a worker that was never
    /// ready, it does nothing. Release a borrow of [`Self::state`] first.
    pub fn stop_now(&self) {
        let connection = self.control.connection.lock().expect("tray connection");
        let Some(connection) = connection.as_ref() else { return };
        let (Some(_), Some(ready)) = (connection.stopping_at, connection.ready.as_ref()) else { return };
        let TrayState::Stopping { context, until } = self.control.state.borrow().clone() else { return };
        ready.stop_now.now();
        let until = Instant::now().checked_add(ready.cancel_timeout).map_or(until, |cut| until.min(cut));
        self.control.state.send_replace(TrayState::Stopping { context, until });
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.disconnect();
    }
}

impl Control {
    #[cfg(target_os = "macos")]
    fn set_registered_types(&self, app: &App) {
        *self.registered_types.lock().expect("tray registered types") = app.type_names();
    }

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
    fn fail(mut self, error: String) {
        let context = self.context.take().expect("the worker task has a context");
        self.control.finish(TrayState::Failed { context, error });
    }

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
