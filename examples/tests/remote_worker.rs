//! Example 15: the cleanup singleton in a worker that reaches only the HTTP API. Real sockets and real time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};
use cicada::store::{MessageStatus, OutcomeStatus};
use cicada::time::Timestamp;
use cicada::{ActorSystem, NoState, SendResult, Store};
use cicada_examples::remote_worker::start_cleanup_worker;
use cicada_examples::singleton::{Cleanup, CleanupMsg, TempFiles};

#[derive(Clone, Default)]
struct FakeFiles {
    purges: Arc<AtomicUsize>,
}

impl TempFiles for FakeFiles {
    async fn purge_older_than(&self, _cutoff: Timestamp, _idempotency_key: &str) -> i32 {
        self.purges.fetch_add(1, Ordering::SeqCst);
        1
    }
}

/// The host's authentication: every request needs the bearer token.
async fn authenticated(req: Request, next: Next) -> Response {
    match req.headers().get("authorization").and_then(|v| v.to_str().ok()) {
        Some("Bearer secret") => next.run(req).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_without_database_access_runs_the_singleton_over_http() {
    let server = ActorSystem::builder(Store::memory()).build();
    let app = Router::new().nest("/internal", cicada_axum::actor_api(server.clone())).layer(from_fn(authenticated));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/internal", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    assert!(start_cleanup_worker(&base, "wrong", FakeFiles::default()).await.is_err(), "the host refuses it");
    let files = FakeFiles::default();
    let worker = start_cleanup_worker(&base, "secret", files.clone()).await.unwrap();
    server.create(&Cleanup, &NoState {}).await.unwrap();
    let SendResult::Accepted(purge) = server.send(&Cleanup, CleanupMsg::Purge { retention_hours: 6 }).await.unwrap()
    else {
        panic!("accepted")
    };
    for _ in 0..500 {
        if let Some(MessageStatus::Finished(outcome)) = server.status(&purge.id).await.unwrap() {
            assert_eq!(outcome.status, OutcomeStatus::Done);
            assert_eq!(files.purges.load(Ordering::SeqCst), 1);
            worker.stop().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the purge did not finish");
}
