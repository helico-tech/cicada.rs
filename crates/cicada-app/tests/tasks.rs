//! `App::task` through `App::run_with`, on a SQLite file in a temporary folder and a port of the loopback address, in
//! real time (ADR 0064).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::store::SqliteOptions;
use cicada::{ActorSystem, Store};
use cicada_app::{App, AppError, Stopping};

use crate::common::{CounterActor, Out, Registry, RegistryActor, Running, TempDb, args, eventually, run};

const LOOPBACK_ANY: [&str; 4] = ["myapp", "serve", "--listen", "127.0.0.1:0"];

/// A flag that a task sets.
#[derive(Clone, Default)]
struct Flag(Arc<AtomicBool>);

impl Flag {
    fn set(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A task that waits for the stop and then sets `saw`.
fn waits(saw: Flag) -> impl FnOnce(ActorSystem, Stopping) -> futures::future::BoxFuture<'static, anyhow::Result<()>> {
    move |_, stopping| {
        Box::pin(async move {
            stopping.wait().await;
            saw.set();
            Ok(())
        })
    }
}

/// The rows of the node table in the SQLite file `db`.
async fn nodes(db: &TempDb) -> usize {
    let store = Store::sqlite(&db.path, SqliteOptions::default()).await.unwrap();
    let nodes = ActorSystem::builder(store.clone()).build().nodes().await.unwrap().len();
    store.close().await;
    nodes
}

/// Scenario: After on_start.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_starts_after_on_start_finds_its_actor_and_runs_before_serve_listens() {
    let db = TempDb::new();
    let out = Out::default();
    let (listening_at_start, found, stopped) = (Flag::default(), Flag::default(), Flag::default());
    let began = Flag::default();
    let (seen, l, b, f, s) = (out.clone(), listening_at_start.clone(), began.clone(), found.clone(), stopped.clone());
    let app = App::new()
        .host(RegistryActor)
        .on_start(|system| async move {
            system.create(&Registry, &cicada::NoState {}).await?;
            Ok(())
        })
        .task("reader", move |system, stopping| async move {
            if seen.text().contains("Listening") {
                l.set();
            }
            b.set();
            if system.get_actor(&cicada::ActorRef::address(&Registry)?).await?.is_some() {
                f.set();
            }
            stopping.wait().await;
            s.set();
            Ok(())
        });
    let (node, _) = Running::serve_to(app, &db.env(), &LOOPBACK_ANY, out).await;
    assert!(began.get(), "the task began before serve listened");
    assert!(!listening_at_start.get(), "serve listened after the task began");
    eventually("the task found the actor", || async { found.get() }).await;
    node.stop().await.expect("exit code 0");
    assert!(stopped.get(), "the task saw Stopping");
}

/// Scenario: A task returns early. Scenario: A task panics. And one that returns an error, in `serve` and `worker`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_that_ends_before_the_stop_stops_the_node_and_fails_the_command() {
    for (end, command) in [("ok", "serve"), ("error", "worker"), ("panic", "serve")] {
        let db = TempDb::new();
        let saw = Flag::default();
        let app =
            App::new().host(CounterActor).task("other", waits(saw.clone())).task("runner", move |_, _| async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                match end {
                    "ok" => Ok(()),
                    "error" => anyhow::bail!("the queue is gone"),
                    _ => panic!("the runner broke"),
                }
            });
        let list: &[&str] = if command == "serve" { &LOOPBACK_ANY } else { &["myapp", "worker"] };
        let error = tokio::time::timeout(Duration::from_secs(10), run(app, &db.env(), list))
            .await
            .expect("the node stops")
            .unwrap_err();
        assert_eq!(error.exit_code(), 1, "{end}: {error}");
        let expected = match end {
            "ok" => "task runner ended before the stop".to_owned(),
            "error" => "task runner ended before the stop: the queue is gone".to_owned(),
            _ => "task runner ended before the stop: the runner broke".to_owned(),
        };
        assert_eq!(error.to_string(), expected, "{end}");
        assert!(saw.get(), "{end}: the other task saw Stopping");
        assert_eq!(nodes(&db).await, 0, "{end}: the node stopped and removed its row");
    }
}

/// An early end fails the command also with a grace period of zero: the task records its end before the stop begins.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_early_end_with_a_grace_period_of_zero_still_fails_the_command() {
    for _ in 0..20 {
        let db = TempDb::new();
        let mut env = db.env();
        env.insert("CICADA_SHUTDOWN_GRACE_PERIOD".into(), "0s".into());
        let app = App::new()
            .host(CounterActor)
            .task("other", |_, _| std::future::pending())
            .task("runner", |_, _| async { anyhow::bail!("the queue is gone") });
        let error = run(app, &env, &["myapp", "worker"]).await.unwrap_err();
        assert_eq!(error.exit_code(), 1, "{error}");
        assert_eq!(error.to_string(), "task runner ended before the stop: the queue is gone");
    }
}

/// Scenario: A stop (in `serve`): `/ready` answers 503 while the tasks end.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_sets_stopping_and_ready_answers_503_while_the_tasks_end() {
    let db = TempDb::new();
    let ended = Flag::default();
    let e = ended.clone();
    let app = App::new().host(CounterActor).task("runner", move |_, stopping| async move {
        stopping.wait().await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        e.set();
        Ok(())
    });
    let (mut node, base) = Running::serve(app, &db.env(), &LOOPBACK_ANY).await;
    assert_eq!(reqwest::get(format!("{base}/ready")).await.unwrap().status(), StatusCode::OK);
    node.signal();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!node.is_finished(), "the task is still ending");
    assert_eq!(reqwest::get(format!("{base}/ready")).await.unwrap().status(), StatusCode::SERVICE_UNAVAILABLE);
    node.ended().await.expect("exit code 0");
    assert!(ended.get(), "the task ended in good order");
}

/// Scenario: A stop (in `worker`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_of_a_worker_ends_its_task_with_exit_code_0() {
    let db = TempDb::new();
    let (began, saw) = (Flag::default(), Flag::default());
    let (b, s) = (began.clone(), saw.clone());
    let app = App::new().host(CounterActor).task("runner", move |system, stopping| async move {
        b.set();
        stopping.wait().await;
        system.now().await?; // the node's ActorSystem still answers while the task ends
        s.set();
        Ok(())
    });
    let worker = Running::spawn(app, &db.env(), &["myapp", "worker"]);
    eventually("the task began", || async { began.get() }).await;
    worker.stop().await.expect("exit code 0");
    assert!(saw.get());
}

/// An output that cannot be written to.
struct Broken;

impl std::io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("the pipe is closed"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Scenario: The server fails after the tasks started (the write of `Listening on`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_write_of_listening_on_stops_the_tasks() {
    let db = TempDb::new();
    let saw = Flag::default();
    let app = App::new().host(CounterActor).task("runner", waits(saw.clone()));
    let error =
        app.run_with(args(&LOOPBACK_ANY), db.env(), Broken, std::future::pending()).await.expect_err("it fails");
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().contains("cannot write to the output"), "{error}");
    assert!(saw.get(), "the task saw Stopping before the command ended");
    assert_eq!(nodes(&db).await, 0, "the node stopped");
}

/// Two tasks with one name refuse `serve` and `worker`; the operator commands run no task.
#[tokio::test]
async fn two_tasks_with_one_name_are_refused_by_serve_and_worker() {
    let app = || {
        let ran = Flag::default();
        App::new()
            .host(CounterActor)
            .task("runner", waits(ran.clone()))
            .task("pool", waits(ran.clone()))
            .task("runner", waits(ran.clone()))
    };
    let db = TempDb::new();
    for list in [&LOOPBACK_ANY[..], &["myapp", "worker"]] {
        let error: AppError = run(app(), &db.env(), list).await.unwrap_err();
        assert_eq!(error.exit_code(), 2, "{error}");
        assert_eq!(error.to_string(), "two tasks named runner");
    }
    let error = run(app(), &db.env(), &["myapp", "get", "actors", "--server", "http://127.0.0.1:1"]).await.unwrap_err();
    assert_eq!(error.exit_code(), 1, "not refused: the request fails: {error}");
    assert!(!db.path.exists(), "no store was opened");
}
