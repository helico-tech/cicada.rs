//! Example 21 as a running host:
//!
//! ```text
//! cargo run -p cicada-examples --example pinned_process -- --db runs.db --node host-a [--new-run KEY] [--exit-code N]
//! ```
//!
//! It opens the SQLite store `--db`, builds a [`ProcessHost`] for the stable node id `--node` with a fresh host
//! epoch, and runs the run type there. At startup it refreshes the existing runs pinned to that node: a run of an
//! earlier host process that was not terminal shows `NeedsReconciliation`, and nothing is adopted, signalled or
//! restarted. `--new-run KEY` creates one new run under a key never used before and starts it. Its child is this
//! executable in its inert child mode: it waits until stopped, or with `--exit-code N` exits at once with `N`. The
//! host prints each change of its runs until Ctrl-C.
//!
//! Ctrl-C closes the host to new runs, stops and reaps every child and flushes the last observations, within
//! [`IO_TIMEOUT`], while the node and the store still run; then it stops the node, the pump and the store, in that
//! order. When it cannot confirm every exit or delivery it says so and exits with a failure. Run one host per node
//! id; a host killed abruptly may leave a child behind, which its successor reports for reconciliation.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use cicada::runtime::NodeConfig;
use cicada::store::{NodeId, SqliteOptions};
use cicada::{ActorRef, ActorSystem, Store};
use cicada_examples::pinned_process::{
    IO_TIMEOUT, OsBackend, ProcessHost, Run, RunActor, RunMsg, RunState, child_main, create_run, pump, reconcile,
};
use tokio_util::sync::CancellationToken;

/// The hidden first argument of the child mode; never taken from a message.
const CHILD_MODE: &str = "--inert-child";

struct Options {
    db: PathBuf,
    node: NodeId,
    new_run: Option<String>,
    exit_code: Option<i32>,
}

fn parse(mut args: impl Iterator<Item = String>) -> anyhow::Result<Options> {
    let (mut db, mut node, mut new_run, mut exit_code) = (None, None, None, None);
    while let Some(flag) = args.next() {
        let mut value = || args.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--db" => db = Some(PathBuf::from(value()?)),
            "--node" => node = Some(NodeId::new(value()?)?),
            "--new-run" => new_run = Some(value()?),
            "--exit-code" => exit_code = Some(value()?.parse().context("--exit-code takes an integer")?),
            other => bail!("Unknown argument {other}; usage: --db PATH --node ID [--new-run KEY] [--exit-code N]"),
        }
    }
    Ok(Options {
        db: db.context("--db PATH is required")?,
        node: node.context("--node ID is required")?,
        new_run,
        exit_code,
    })
}

fn main() -> anyhow::Result<ExitCode> {
    if std::env::args().nth(1).as_deref() == Some(CHILD_MODE) {
        child_main();
    }
    let options = parse(std::env::args().skip(1))?;
    tokio::runtime::Runtime::new()?.block_on(serve(options))
}

async fn serve(options: Options) -> anyhow::Result<ExitCode> {
    let store = Store::sqlite(&options.db, SqliteOptions::default()).await?;
    let mut backend = OsBackend::new(std::env::current_exe()?, [CHILD_MODE]);
    if let Some(code) = options.exit_code {
        backend = backend.exit_with(code);
    }
    let node = options.node;
    // The host exists before the system that runs its actor type; the pump is attached after.
    let host = ProcessHost::new(node.clone(), Arc::new(backend));
    let system = ActorSystem::builder(store.clone())
        .config(NodeConfig { node_id: node.clone(), ..NodeConfig::default() })
        .host(RunActor::new(host.clone()))
        .build();
    system.start().await?;
    let stop_pump = CancellationToken::new();
    let pump_task = tokio::spawn(pump(host.clone(), system.clone(), stop_pump.clone()));
    println!("store: {}; node: {node}; host epoch: {}", options.db.display(), host.epoch());

    let refreshed = reconcile(&system, &node).await?;
    println!("refreshing {refreshed} existing run(s) of this node");
    if let Some(key) = &options.new_run {
        let run = create_run(&system, &host, key).await.map_err(anyhow::Error::msg)?;
        system.send(&run, RunMsg::Start).await?;
        println!("created and started run {key}");
    }

    let (mut shown, mut failures) = (BTreeMap::new(), 0);
    println!("watching; Ctrl-C stops");
    loop {
        tokio::select! {
            result = tokio::signal::ctrl_c() => { result?; break }
            () = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        show_changes(&system, &node, &mut shown).await?;
        let delivery = host.delivery();
        if delivery.failures > failures {
            failures = delivery.failures;
            eprintln!("observation send failed ({failures} so far): {}", delivery.last_failure.unwrap_or_default());
        }
    }

    println!("stopping children");
    let report = host.shutdown(IO_TIMEOUT).await;
    system.stop().await;
    stop_pump.cancel();
    pump_task.await?;
    store.close().await;
    if report.is_clean() {
        println!("every child exited and its last observation was delivered");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("cleanup not confirmed: no exit seen for {:?}", report.unconfirmed);
        eprintln!("last observation not delivered for {:?}", report.unflushed);
        Ok(ExitCode::FAILURE)
    }
}

/// Prints each run of `node` whose state differs from the last one printed.
async fn show_changes(
    system: &ActorSystem,
    node: &NodeId,
    shown: &mut BTreeMap<String, RunState>,
) -> anyhow::Result<()> {
    let prefix = format!("{}/", Run::TYPE);
    let mut cursor = None;
    loop {
        let page = system.list_actors_with_state(&prefix, cursor.as_ref(), 100).await?;
        for snapshot in page.items.into_iter().filter(|s| s.pinned_to.as_ref() == Some(node)) {
            let state: RunState = serde_json::from_value(snapshot.state)?;
            let key = snapshot.address.name();
            if shown.get(&key) != Some(&state) {
                let refusal = state.refusal.map(|r| format!(", last refusal {r:?}")).unwrap_or_default();
                println!("run {key}: desired {:?}, observed {:?}{refusal}", state.desired, state.observed);
                shown.insert(key, state);
            }
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(()),
        }
    }
}
