//! Turns a set of Cicada actors into a binary (layer 2 of ADR 0018, ADR 0027).
//!
//! ```no_run
//! # use cicada::prelude::*;
//! # use serde::{Deserialize, Serialize};
//! # #[derive(Debug, Clone, Serialize, Deserialize)]
//! # struct Counter { name: String }
//! # impl ActorRef for Counter { const TYPE: &'static str = "counter"; type Message = CounterMsg; }
//! # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
//! # #[serde(tag = "type")]
//! # enum CounterMsg { Add { amount: i64 } }
//! # struct CounterActor;
//! # impl Actor for CounterActor {
//! #     type Ref = Counter;
//! #     type State = NoState;
//! #     async fn handle(&self, _: &mut Turn<Self>, _: CounterMsg) -> HandlerResult { Ok(()) }
//! # }
//! fn main() -> std::process::ExitCode {
//!     cicada_app::App::new().host(CounterActor).run()
//! }
//! ```
//!
//! The binary has three kinds of commands:
//!
//! - `serve` hosts the types, serves the HTTP API at the root and a public `/ready`, and stops in good order on SIGTERM
//!   or SIGINT. It listens on `127.0.0.1:8080` unless `--listen` or `CICADA_LISTEN` says otherwise, and has no
//!   authentication of its own: a host adds its layer with [`App::http`].
//! - `worker` hosts the types and serves nothing.
//! - The operator commands of `cicada` (`get`, `send`, `create`, …), which talk to a node only over HTTP, through
//!   [`cicada_client::ActorClient`], also in a binary that serves. The server comes from `--server`, `CICADA_SERVER` or
//!   a context of the config file.
//!
//! The store comes from `--store` or `CICADA_STORE` (a `postgres://` address, `sqlite:FILE` or the path of a SQLite
//! file), the types from `--types` or `CICADA_TYPES`, and each setting of the node from its flag or its variable
//! `CICADA_…`, else the host's [`App::node_config`], else the default. On PostgreSQL, `serve` and `worker` wait for a
//! store that answers "unavailable", as containers start in any order: they try again every second and start no try
//! after a minute, but one try can take seconds (about 10 s for an address that does not answer), so the wait is about
//! a minute and can be longer, and a wrong password counts as unavailable and is tried again like an unreachable
//! database. A binary without types has neither `serve` nor
//! `worker`: the plain `cicada` binary is one. The stores are the features `sqlite` and `postgres`, and the terminal UI
//! of the operator commands the feature `tui`; none is on by default. The help, the usage errors and the hints name the
//! binary as it was called.

mod cli;
#[cfg(all(test, feature = "tui"))]
mod cli_reference;
mod command;
mod config;
mod node;
mod operator;
mod output;
mod settings;
#[cfg(feature = "tui")]
mod tui;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;
use cicada::observer::TurnObserver;
use cicada::runtime::NodeConfig;
use cicada::{Actor, ActorSystem, ActorType};

use crate::command::Parsed;
use crate::node::{HostedType, HttpFn, OnStartFn, Parts, Stop};
use crate::settings::{Kind, WorkerMode};

/// Environment variables, by name: the process environment in [`App::run`], a literal map in tests.
pub type Env = BTreeMap<String, String>;

/// A binary: the types it hosts, what the host adds to a node, and what `--version` prints. [`Self::run`] runs it.
pub struct App {
    types: Vec<HostedType>,
    observers: Vec<Arc<dyn TurnObserver>>,
    node_config: Option<NodeConfig>,
    http: Vec<HttpFn>,
    on_start: Vec<OnStartFn>,
    version: Option<String>,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("types", &self.types.iter().map(|t| t.name).collect::<Vec<_>>())
            .field("observers", &self.observers.len())
            .field("node_config", &self.node_config)
            .field("http", &self.http.len())
            .field("on_start", &self.on_start.len())
            .field("version", &self.version)
            .finish()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// A binary that hosts no type yet: it has the operator commands only.
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            observers: Vec::new(),
            node_config: None,
            http: Vec::new(),
            on_start: Vec::new(),
            version: None,
        }
    }

    /// Adds a type, as [`cicada::runtime::ActorSystemBuilder::host`]. A binary with a type has `serve` and `worker`,
    /// which host every type unless `--types` names some.
    pub fn host<A: Actor>(mut self, actor: impl Into<ActorType<A>>) -> Self {
        let actor = actor.into();
        let name = actor.type_name();
        self.types.push(HostedType { name, host: Box::new(move |hosting| hosting.host(actor)) });
        self
    }

    /// Adds an observer of the turns of the node, as [`cicada::runtime::ActorSystemBuilder::observe`]; for telemetry,
    /// `cicada_otel::ActorTelemetry`.
    pub fn observe(mut self, observer: impl TurnObserver) -> Self {
        self.observers.push(Arc::new(observer));
        self
    }

    /// The host's `NodeConfig`. A flag of `serve` or `worker`, and its variable `CICADA_…`, override a field of it.
    pub fn node_config(mut self, config: NodeConfig) -> Self {
        self.node_config = Some(config);
        self
    }

    /// Lets the host change the HTTP side of `serve`: `f` gets the router of the API and the node, and returns the
    /// router to serve. A layer put on the router covers the API and what was merged before it; routes merged after it
    /// are not covered, as in axum. Without a layer of the host the API has no authentication: it can change and delete
    /// everything. `/ready` is answered by `App` outside this router and before it: no layer of the host covers it, and
    /// a route `/ready` of the host is never reached. A route that the API has already makes axum panic while the
    /// router is built: the panic hook of the process prints axum's message to standard error, and then the command
    /// fails with that message as its error (exit code 1), before the node starts. `App` leaves the hook alone, as it
    /// is shared by every thread of the process. Functions of several calls apply in the order of the calls.
    pub fn http(mut self, f: impl FnOnce(Router, &ActorSystem) -> Router + Send + 'static) -> Self {
        self.http.push(Box::new(f));
        self
    }

    /// Runs `f` once after the node started, in `serve` before it listens and in `worker`: to create an actor that must
    /// exist, for example. An error stops the node and fails the command; a stop while it runs drops it. Functions of
    /// several calls run in the order of the calls.
    pub fn on_start<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce(ActorSystem) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        self.on_start.push(Box::new(move |system| Box::pin(f(system))));
        self
    }

    /// What `--version` prints; `cicada-app` and the version of this crate when it is not called, as `App` does not
    /// know the version of the host.
    pub fn version(mut self, text: impl Into<String>) -> Self {
        self.version = Some(text.into());
        self
    }

    /// Builds a runtime, reads the arguments and the environment of the process, and runs the command. For `serve` and
    /// `worker` it sets up logging (`tracing` to standard output, filtered by `RUST_LOG`, `info,sqlx=warn` when unset,
    /// unless the process has logging already) and stops the node on SIGTERM or SIGINT; for an operator command it
    /// installs nothing. Returns the exit code: 0 for success, the help, the version and a node that was stopped, 2 for
    /// a command line that cannot be used, 1 for a command that failed; an error is printed to standard error as
    /// `error: …`. It must not be called inside a runtime.
    pub fn run(self) -> ExitCode {
        match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(runtime) => runtime.block_on(self.run_async()),
            Err(e) => {
                eprintln!("error: cannot start the runtime of tokio: {e}");
                ExitCode::FAILURE
            }
        }
    }

    /// [`Self::run`] on the caller's runtime: for a host whose `main` is async, or whose actors are built with `await`.
    pub async fn run_async(self) -> ExitCode {
        let args: Vec<OsString> = std::env::args_os().collect();
        let shutdown: Pin<Box<dyn Future<Output = ()> + Send>> = if !self.types.is_empty() && node_command(&args) {
            init_logging();
            Box::pin(shutdown_signal())
        } else {
            Box::pin(std::future::pending())
        };
        exit_code(self.run_with(args, process_env(), std::io::stdout(), shutdown).await)
    }

    /// The command on the caller's runtime, with everything from outside passed in. `args` starts with the name of the
    /// program, as [`std::env::args_os`] does. `shutdown` stops `serve` and `worker`, and is watched from the moment
    /// the command begins; the other commands do not look at it. It never ends the process and sets up no logging: the
    /// help and the version go to `out` and return `Ok`, and `serve` writes `Listening on http://<address>` there when
    /// it takes requests.
    // The signature promises a future that is `Send` and `'static`, and the compiler checks it.
    #[allow(clippy::manual_async_fn)]
    pub fn run_with(
        self,
        args: Vec<OsString>,
        env: Env,
        out: impl Write + Send + 'static,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> impl Future<Output = Result<(), AppError>> + Send + 'static {
        async move {
            let mut out = out;
            let name = command::program_name(&args);
            let names: Vec<&'static str> = self.types.iter().map(|t| t.name).collect();
            match command::parse(&args, !names.is_empty()).map_err(AppError::usage)? {
                Parsed::Help(help) => write!(out, "{help}").map_err(|e| AppError::Failed(e.into())),
                Parsed::Version => {
                    let version =
                        self.version.unwrap_or_else(|| concat!("cicada-app ", env!("CARGO_PKG_VERSION")).into());
                    writeln!(out, "{version}").map_err(|e| AppError::Failed(e.into()))
                }
                Parsed::Operator(cli) => operator::run(*cli, &name, &env, &mut out).await.map_err(AppError::Failed),
                Parsed::Serve(args) => {
                    let stop = Stop::watch(shutdown);
                    let settings = settings::resolve(
                        Kind::Serve,
                        &args.node,
                        args.listen,
                        &env,
                        self.node_config.as_ref(),
                        &names,
                    )
                    .map_err(AppError::usage)?;
                    node::serve(self.into_parts(), settings, &mut out, stop).await
                }
                Parsed::Worker(args) => {
                    let stop = Stop::watch(shutdown);
                    match settings::worker_mode(&args, &env).map_err(AppError::usage)? {
                        WorkerMode::Store => {
                            let host = self.node_config.as_ref();
                            let settings = settings::resolve(Kind::Worker, &args.node, None, &env, host, &names)
                                .map_err(AppError::usage)?;
                            node::worker(self.into_parts(), settings, stop).await
                        }
                        WorkerMode::Http => {
                            if !self.on_start.is_empty() {
                                return Err(AppError::usage(
                                    "a worker over HTTP cannot run App::on_start: its functions need an ActorSystem, \
                                     which only a node on the store has; bootstrap with serve or a worker on the store, \
                                     or move that work out of this binary's App"
                                        .to_owned(),
                                ));
                            }
                            let settings = settings::resolve_http(&args.node, &env, self.node_config.as_ref(), &names)
                                .map_err(AppError::usage)?;
                            let target = settings::http_target(&args, &env, &name).map_err(AppError::Failed)?;
                            node::http_worker(self.into_parts(), settings, target, stop).await
                        }
                    }
                }
            }
        }
    }

    fn into_parts(self) -> Parts {
        Parts { types: self.types, observers: self.observers, http: self.http, on_start: self.on_start }
    }
}

/// True when `args` (the name of the program first) name `serve` or `worker`, read as the command line reads them:
/// false for `--context serve get actors`, for `serve --help` and for a command line that cannot be used. A host asks
/// it before it builds what only a node needs, such as an exporter of telemetry.
pub fn node_command(args: &[OsString]) -> bool {
    matches!(command::parse(args, true), Ok(Parsed::Serve(_) | Parsed::Worker(_)))
}

/// Why a command did not succeed.
#[derive(Debug)]
pub enum AppError {
    /// The command line cannot be used: exit code 2. `run_with` hides the password of an address the message quotes
    /// (`scheme://user:***@host`); it leaves alone a raw password with `/`, `#`, `?`, a quote or a space, an address
    /// without `//` (`postgres:u:pw@h`) and the libpq form (`host=h password=…`).
    Usage(String),
    /// The command failed: exit code 1.
    Failed(anyhow::Error),
}

impl AppError {
    /// A command line that cannot be used, quoting what the user gave: a flag, a variable or an argument that may be an
    /// address with a password, which is hidden.
    fn usage(message: String) -> Self {
        Self::Usage(command::hide_passwords(&message))
    }

    /// 2 for [`Self::Usage`], 1 for [`Self::Failed`].
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl fmt::Display for AppError {
    /// The message, with each cause of a failure that it does not already hold.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::Failed(error) => f.write_str(&operator::error_text(error)),
        }
    }
}

impl std::error::Error for AppError {}

/// The variables of the process, without those whose name or value is not UTF-8.
fn process_env() -> Env {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// The exit code of `result`; an error is printed to standard error first.
fn exit_code(result: Result<(), AppError>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

/// `tracing` to standard output, filtered by `RUST_LOG`, with colours only on a terminal; nothing when the process has
/// logging already.
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sqlx=warn"));
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(ansi).try_init();
}

/// Completes on SIGTERM (`docker stop`, Kubernetes) or SIGINT (Ctrl-C). The handlers are installed at once, so that
/// neither signal ends the process from here on; a second signal changes nothing.
fn shutdown_signal() -> impl Future<Output = ()> + Send + 'static {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let watch = |kind: SignalKind, name: &'static str| {
            let installed = signal(kind);
            async move {
                match installed {
                    Ok(mut signal) => {
                        signal.recv().await;
                        tracing::info!("{name} received; stopping");
                    }
                    Err(e) => {
                        tracing::warn!("Cannot watch {name}: {e}");
                        std::future::pending::<()>().await;
                    }
                }
            }
        };
        let terminate = watch(SignalKind::terminate(), "SIGTERM");
        let interrupt = watch(SignalKind::interrupt(), "SIGINT");
        async move {
            tokio::select! {
                () = terminate => {}
                () = interrupt => {}
            }
        }
    }
    #[cfg(not(unix))]
    async {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("Ctrl-C received; stopping");
        } else {
            std::future::pending::<()>().await;
        }
    }
}
