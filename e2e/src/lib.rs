//! End-to-end and fault tests: the sample service in Docker (a PostgreSQL cluster from `e2e/docker-compose.yml` and
//! single SQLite nodes), driven over HTTP only. This library is the harness; the scenarios are in `tests/suite/`, one
//! module per test class, and `e2e/README.md` says how to run them.
//!
//! The suite runs only with `CICADA_E2E=1`; otherwise its one test returns at once, so `cargo test --workspace` needs
//! no Docker. [`run_suite`] owns the life cycle: it builds the image, starts the stack once, runs the classes one after
//! another in the given order, and always saves the container logs to `target/e2e-logs/` and removes every container
//! and volume afterwards, also when a scenario fails, the suite panics or the run is interrupted with Ctrl-C.

pub mod cluster;
pub mod docker;
pub mod ordering;

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::FutureExt;

use crate::cluster::cluster;
use crate::docker::{compose, docker_blocking};

/// The containers started with `docker run` by the SQLite class; the teardown removes them as well.
pub const SINGLE_CONTAINERS: [&str; 2] = ["cicada-rs-e2e-sqlite", "cicada-rs-e2e-auth"];
/// The named volume of the SQLite node.
pub const SQLITE_VOLUME: &str = "cicada-rs-e2e-sqlite-data";

/// A bound for every test (and every hook), so a hung fault scenario cannot stall the run.
pub const TEST_TIMEOUT: Duration = Duration::from_secs(240);

/// One step of the suite: a test or a hook. It fails by panicking (assertions).
pub type Step = fn() -> Pin<Box<dyn Future<Output = ()>>>;

/// Wraps an `async fn()` into a [`Step`] named after the function: `step!(health_on_every_node)`.
#[macro_export]
macro_rules! step {
    ($f:path) => {
        (
            stringify!($f),
            (|| Box::pin($f()) as ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ()>>>) as $crate::Step,
        )
    };
}

/// A test class: tests run in order, with optional hooks.
pub struct Class {
    /// The name shown in the report.
    pub name: &'static str,
    /// Runs before the first test; when it fails, every test of the class fails.
    pub before_all: Option<Step>,
    /// Runs after every test; when it fails, that test fails.
    pub after_each: Option<Step>,
    /// Runs after the last test, also when tests failed.
    pub after_all: Option<Step>,
    /// The tests in run order, by name.
    pub tests: Vec<(&'static str, Step)>,
}

impl Class {
    /// A class without hooks.
    pub fn new(name: &'static str, tests: Vec<(&'static str, Step)>) -> Self {
        Self { name, before_all: None, after_each: None, after_all: None, tests }
    }
}

/// True when `CICADA_E2E=1`.
pub fn enabled() -> bool {
    std::env::var("CICADA_E2E").is_ok_and(|v| v == "1")
}

/// Runs `classes` in order against a fresh stack, then tears it down; see the crate documentation. Only the tests
/// whose `Class::test` name contains `CICADA_E2E_ONLY` run, when that is set (the stack still comes up first).
///
/// # Panics
///
/// When a test failed (after the teardown), or when the image or the stack cannot be brought up.
pub fn run_suite(classes: Vec<Class>) {
    if !enabled() {
        println!("Skipping the E2E suite: set CICADA_E2E=1 to run it (needs Docker; see e2e/README.md).");
        return;
    }
    let only = std::env::var("CICADA_E2E_ONLY").ok().filter(|f| !f.is_empty());
    let started = Instant::now();
    let stack = Stack;
    teardown(false); // leftovers of an aborted run
    let build = Instant::now();
    docker::build_image();
    println!("E2E: image {} built in {:.1?}", docker::IMAGE, build.elapsed());

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a tokio runtime");
    let results = runtime.block_on(async {
        tokio::spawn(async {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("E2E: interrupted; tearing the stack down");
                teardown(true);
                std::process::exit(130);
            }
        });
        let up = Instant::now();
        compose::up().await.expect("compose up");
        cluster().await_healthy(Duration::from_secs(90)).await;
        println!("E2E: cluster up in {:.1?}", up.elapsed());
        let mut results = Vec::new();
        for class in &classes {
            run_class(class, only.as_deref(), &mut results).await;
        }
        results
    });
    drop(runtime);
    drop(stack);

    let failed: Vec<&(String, Result<(), String>)> = results.iter().filter(|(_, r)| r.is_err()).collect();
    println!("E2E: {} passed, {} failed in {:.1?}", results.len() - failed.len(), failed.len(), started.elapsed());
    for (name, result) in &failed {
        println!("  FAILED {name}: {}", result.as_ref().unwrap_err());
    }
    assert!(failed.is_empty(), "{} E2E test(s) failed; container logs are in {}", failed.len(), logs_dir().display());
}

async fn run_class(class: &Class, only: Option<&str>, results: &mut Vec<(String, Result<(), String>)>) {
    let tests: Vec<&(&str, Step)> = class
        .tests
        .iter()
        .filter(|(name, _)| only.is_none_or(|f| format!("{}::{name}", class.name).contains(f)))
        .collect();
    if tests.is_empty() {
        return;
    }
    let before = match class.before_all {
        Some(hook) => guarded(hook).await,
        None => Ok(()),
    };
    for (name, test) in tests {
        let name = format!("{}::{name}", class.name);
        let started = Instant::now();
        let mut result = match &before {
            Ok(()) => guarded(*test).await,
            Err(e) => Err(format!("before all: {e}")),
        };
        if let Some(hook) = class.after_each
            && let Err(e) = guarded(hook).await
        {
            result = result.and(Err(format!("after each: {e}")));
        }
        match &result {
            Ok(()) => println!("E2E: PASS {name} ({:.1?})", started.elapsed()),
            Err(e) => println!("E2E: FAIL {name} ({:.1?}): {e}", started.elapsed()),
        }
        results.push((name, result));
    }
    if let Some(hook) = class.after_all
        && let Err(e) = guarded(hook).await
    {
        println!("E2E: {} after all failed: {e}", class.name);
        results.push((format!("{}::after_all", class.name), Err(e)));
    }
}

/// Runs `step` within [`TEST_TIMEOUT`], with a panic as its failure.
async fn guarded(step: Step) -> Result<(), String> {
    match tokio::time::timeout(TEST_TIMEOUT, AssertUnwindSafe(step()).catch_unwind()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(panic)) => Err(panic_message(panic.as_ref())),
        Err(_) => Err(format!("timed out after {TEST_TIMEOUT:?}")),
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".to_owned())
}

/// Tears the stack down when dropped: at the end of the run, and while unwinding from a panic.
struct Stack;

impl Drop for Stack {
    fn drop(&mut self) {
        teardown(true);
    }
}

static TORN_DOWN: AtomicBool = AtomicBool::new(false);

/// Removes the compose stack (containers, network, volumes) and the single containers and their volume. With
/// `final_run`, saves the compose logs first and does it only once.
fn teardown(final_run: bool) {
    if final_run {
        if TORN_DOWN.swap(true, Ordering::SeqCst) {
            return;
        }
        save_logs("compose", &compose::logs_blocking());
    }
    let down = compose::down_blocking();
    if !down.ok() {
        eprintln!("E2E: compose down failed (exit {}): {}", down.code, down.text);
    }
    for container in SINGLE_CONTAINERS {
        docker_blocking(&["rm", "-f", "-v", container]);
    }
    docker_blocking(&["volume", "rm", "-f", SQLITE_VOLUME]);
}

/// `target/e2e-logs` (under `CARGO_TARGET_DIR` when set).
pub fn logs_dir() -> PathBuf {
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| docker::root().join("target"));
    target.join("e2e-logs")
}

/// Writes `text` to `target/e2e-logs/<name>-<time>.log`.
pub fn save_logs(name: &str, text: &str) {
    let dir = logs_dir();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let file = dir.join(format!("{name}-{stamp}.log"));
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&file, text)) {
        eprintln!("E2E: cannot save {}: {e}", file.display());
    }
}
