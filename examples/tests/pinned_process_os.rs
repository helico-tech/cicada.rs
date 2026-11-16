//! Example 21's OS adapter on real processes, on Linux and macOS only, without an actor runtime or virtual time.
//!
//! The check runs this test executable again as a helper (selected by an environment variable), under a watchdog
//! that kills it after a minute. The helper drives [`ProcessHost`] with [`OsBackend`] over real children, which are
//! this executable once more in the inert child mode of [`child_main`], and prints one handshake line per case:
//! a normal exit with its code, a repeated start that spawns nothing, a stop that kills and reaps, and a shutdown
//! that confirms every exit. These cases do not show cleanup after an abrupt death of the host.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cicada::store::NodeId;
use cicada_examples::pinned_process::{IO_TIMEOUT, Observed, OsBackend, ProcessHost, Run, child_main};

/// Selects the role of this executable: `helper` or `child`.
const ROLE: &str = "CICADA_PINNED_PROCESS_OS_ROLE";

/// How long the helper may take before the check kills it.
const WATCHDOG: Duration = Duration::from_secs(60);

/// The inert child, when the role says so; otherwise nothing.
#[test]
fn child_fixture() {
    if std::env::var(ROLE).as_deref() == Ok("child") {
        child_main();
    }
}

/// The helper, when the role says so; otherwise nothing.
#[test]
fn helper() {
    if std::env::var(ROLE).as_deref() == Ok("helper") {
        tokio::runtime::Runtime::new().unwrap().block_on(cases());
    }
}

fn backend() -> OsBackend {
    let exe = std::env::current_exe().unwrap();
    OsBackend::new(exe, ["--exact", "child_fixture", "--quiet", "--test-threads=1"]).env(ROLE, "child")
}

fn host(backend: OsBackend) -> std::sync::Arc<ProcessHost> {
    ProcessHost::new(NodeId::new("os-host").unwrap(), std::sync::Arc::new(backend))
}

fn run(key: &str) -> Run {
    Run { key: key.into() }
}

/// The slot of `run` once it is terminal, waiting at most [`IO_TIMEOUT`] more.
async fn terminal(host: &ProcessHost, run: &Run) -> Observed {
    let deadline = tokio::time::Instant::now() + IO_TIMEOUT;
    loop {
        let status = host.snapshot(run).expect("a slot").status;
        if status.is_terminal() || tokio::time::Instant::now() >= deadline {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn cases() {
    let exits = host(backend().exit_with(7));
    exits.start(&run("a")).await.unwrap();
    assert_eq!(terminal(&exits, &run("a")).await, Observed::Exited { code: Some(7) });
    println!("HANDSHAKE exit 7");
    let before = exits.snapshot(&run("a")).unwrap();
    let again = exits.start(&run("a")).await.unwrap();
    assert_eq!(again, before, "a repeat changes nothing");
    println!("HANDSHAKE repeat");

    let waits = host(backend());
    let started = waits.start(&run("b")).await.unwrap();
    assert!(matches!(started.status, Observed::Running { pid: Some(_) }), "{started:?}");
    let stopped = waits.stop(&run("b")).await.unwrap();
    assert_eq!(stopped.status, Observed::Exited { code: None }, "killed by a signal and reaped");
    println!("HANDSHAKE stop");

    let running = waits.start(&run("c")).await.unwrap();
    assert!(matches!(running.status, Observed::Running { .. }), "{running:?}");
    // No pump runs here, so the shutdown waits out its budget for deliveries and reports them.
    let report = waits.shutdown(Duration::from_secs(2)).await;
    assert!(report.unconfirmed.is_empty(), "{report:?}");
    assert_eq!(report.unflushed, ["b", "c"]);
    assert_eq!(waits.snapshot(&run("c")).unwrap().status, Observed::Exited { code: None });
    println!("HANDSHAKE shutdown");
    println!("HANDSHAKE done");
}

#[test]
fn the_os_adapter_exits_repeats_stops_and_reaps_real_children() {
    let exe = std::env::current_exe().unwrap();
    let mut helper = Command::new(exe)
        .args(["--exact", "helper", "--nocapture", "--test-threads=1"])
        .env(ROLE, "helper")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = helper.stdout.take().unwrap();
    let (lines, received) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                return;
            }
        }
    });
    let deadline = Instant::now() + WATCHDOG;
    let mut handshakes = Vec::new();
    loop {
        match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => match line.split_once("HANDSHAKE ").map(|(_, handshake)| handshake) {
                Some("done") => break,
                Some(handshake) => handshakes.push(handshake.to_owned()),
                None => {}
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = helper.kill();
                let _ = helper.wait();
                panic!("the helper did not finish within {WATCHDOG:?}; handshakes so far: {handshakes:?}");
            }
        }
    }
    let status = helper.wait().unwrap();
    assert_eq!(handshakes, ["exit 7", "repeat", "stop", "shutdown"]);
    assert!(status.success(), "the helper failed: {status}");
}
