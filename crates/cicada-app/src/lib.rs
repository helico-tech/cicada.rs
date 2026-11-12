//! Turns a set of Cicada actors into a binary (layer 2 of ADR 0018).
//!
//! The operator commands of the binary talk to a node only over HTTP, through [`cicada_client::ActorClient`]. The
//! server comes from `--server`, `CICADA_SERVER` or a context of the config file. The help, the usage errors and the
//! hints name the binary as it was called.
//!
//! ```no_run
//! fn main() -> std::process::ExitCode {
//!     cicada_app::App::new().version(concat!("myapp ", env!("CARGO_PKG_VERSION"))).run()
//! }
//! ```

mod cli;
mod command;
mod config;
mod operator;
mod output;
#[cfg(feature = "tui")]
mod tui;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::io::Write;
use std::process::ExitCode;

use crate::command::Parsed;

/// Environment variables, by name: the process environment in [`App::run`], a literal map in tests.
pub type Env = BTreeMap<String, String>;

/// A binary: its commands and what it prints for `--version`.
pub struct App {
    version: Option<String>,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App").field("version", &self.version).finish_non_exhaustive()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// A binary with the operator commands only.
    pub fn new() -> Self {
        Self { version: None }
    }

    /// What `--version` prints; `cicada-app` and the version of this crate when it is not called.
    pub fn version(mut self, text: impl Into<String>) -> Self {
        self.version = Some(text.into());
        self
    }

    /// Builds a runtime, reads the arguments and the environment of the process, and runs the command. Returns the
    /// exit code: 0 for success, the help and the version, 2 for a command line that cannot be used, 1 for a command
    /// that failed; an error is printed to standard error as `error: …`. It must not be called inside a runtime.
    pub fn run(self) -> ExitCode {
        match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(runtime) => runtime.block_on(self.run_async()),
            Err(e) => {
                eprintln!("error: cannot start the runtime of tokio: {e}");
                ExitCode::FAILURE
            }
        }
    }

    /// [`Self::run`] on the caller's runtime: for a host whose `main` is async.
    pub async fn run_async(self) -> ExitCode {
        let args: Vec<OsString> = std::env::args_os().collect();
        let result = self.run_with(args, process_env(), std::io::stdout(), std::future::pending()).await;
        exit_code(result)
    }

    /// The command on the caller's runtime, with everything from outside passed in. `args` starts with the name of the
    /// program, as [`std::env::args_os`] does. `shutdown` stops a node; the operator commands do not look at it. It
    /// never ends the process and sets up no logging: the help and the version go to `out` and return `Ok`.
    pub fn run_with(
        self,
        args: Vec<OsString>,
        env: Env,
        out: impl Write + Send + 'static,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> impl Future<Output = Result<(), AppError>> + Send + 'static {
        drop(shutdown);
        async move {
            let mut out = out;
            let name = command::program_name(&args);
            match command::parse(&args).map_err(AppError::Usage)? {
                Parsed::Help(help) => write!(out, "{help}").map_err(|e| AppError::Failed(e.into())),
                Parsed::Version => {
                    let version =
                        self.version.unwrap_or_else(|| concat!("cicada-app ", env!("CARGO_PKG_VERSION")).into());
                    writeln!(out, "{version}").map_err(|e| AppError::Failed(e.into()))
                }
                Parsed::Operator(cli) => operator::run(*cli, &name, &env, &mut out).await.map_err(AppError::Failed),
            }
        }
    }
}

/// Why a command did not succeed.
#[derive(Debug)]
pub enum AppError {
    /// The command line cannot be used: exit code 2.
    Usage(String),
    /// The command failed: exit code 1.
    Failed(anyhow::Error),
}

impl AppError {
    /// 2 for [`Self::Usage`], 1 for [`Self::Failed`].
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl fmt::Display for AppError {
    /// The message, with the causes of a failure that it does not already hold.
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
