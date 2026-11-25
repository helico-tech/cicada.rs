//! The warnings of `App::task` at a stop, in a test binary of their own: tracing caches whether a log line is wanted
//! per call site, so another test that logs the same line without a subscriber at the same time could hide it here.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cicada_app::App;

use crate::common::{CounterActor, Out, Running, TempDb, eventually};

/// Scenario: A task that does not stop. Scenario: An error after the stop.
#[tokio::test]
async fn a_task_that_ignores_the_stop_is_aborted_and_an_error_after_the_stop_is_a_warning() {
    let log = Out::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || writer.clone()).finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let db = TempDb::new();
    let mut env = db.env();
    env.insert("CICADA_SHUTDOWN_GRACE_PERIOD".into(), "300ms".into());
    let began = Arc::new(AtomicBool::new(false));
    let b = began.clone();
    let app = App::new()
        .host(CounterActor)
        .task("stubborn", move |_, _| async move {
            b.store(true, Ordering::SeqCst);
            std::future::pending::<anyhow::Result<()>>().await
        })
        .task("sloppy", |_, stopping| async move {
            stopping.wait().await;
            anyhow::bail!("the connection was already closed")
        });
    let mut node = Running::spawn(app, &env, &["myapp", "worker"]);
    eventually("the task began", || async { began.load(Ordering::SeqCst) }).await;
    let signalled = Instant::now();
    node.signal();
    node.ended().await.expect("exit code 0");
    let took = signalled.elapsed();
    assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(3), "{took:?}");
    let log = log.text();
    assert!(log.contains("Task stubborn did not end within the shutdown grace period of 300ms; aborted"), "{log}");
    assert!(log.contains("Task sloppy failed after the stop: the connection was already closed"), "{log}");
}
