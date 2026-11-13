//! Example 15 — a worker outside the database network.
//!
//! The cleanup singleton of [`crate::singleton`], unchanged, runs in a process that cannot reach the database: a
//! [`Worker`] leases its turns over HTTP from any server that serves the full API (`cicada_axum::actor_api`), runs
//! them here, and renews and completes them through whichever server the load balancer picks (ADR 0040). The server
//! needs no copy of the handler, and no session sticks to one server.
//!
//! The worker writes: the host's authentication must cover `/leases` and its children, and `PUT /types/{name}`, with
//! which the worker registers its contract. The HTTP client disables redirects (a `307` or `308` would send a POST
//! again) and has no retry policy of its own; `ActorClient` itself never repeats a lease, renewal, release or
//! completion.

use std::time::Duration;

use cicada::runtime::{Worker, WorkerConfig};
use cicada_client::ActorClient;

use crate::singleton::{CleanupActor, TempFiles};

/// What starting a worker can fail with.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The HTTP client of a worker: `token` as a bearer token on every request, no redirects, no retries of its own.
pub fn worker_http_client(token: &str) -> Result<reqwest::Client, Error> {
    let mut bearer = reqwest::header::HeaderValue::try_from(format!("Bearer {token}"))?;
    bearer.set_sensitive(true);
    let headers = reqwest::header::HeaderMap::from_iter([(reqwest::header::AUTHORIZATION, bearer)]);
    Ok(reqwest::Client::builder().default_headers(headers).redirect(reqwest::redirect::Policy::none()).build()?)
}

/// Starts a worker of the cleanup singleton, with `files`, against the full API at `base_url`. It registers the
/// contract through the server and polls for turns until it is stopped.
pub async fn start_cleanup_worker<F: TempFiles>(base_url: &str, token: &str, files: F) -> Result<Worker, Error> {
    let source = ActorClient::with_client(base_url, worker_http_client(token)?).turn_timeout(Duration::from_secs(5));
    let worker = Worker::builder(source)
        .host(CleanupActor { files })
        .config(WorkerConfig { concurrency: 4, ..WorkerConfig::default() })
        .build();
    worker.start().await?;
    Ok(worker)
}
