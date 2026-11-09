//! The Cicada HTTP API as axum routers: [`actor_api`] serves the twelve endpoints of the actor HTTP API for a node's
//! [`ActorSystem`], [`actor_health`] a public readiness check.
//!
//! | Method and path | Success | Errors |
//! | --- | --- | --- |
//! | `PUT /actors/{path}` body `{state?}` | 201 created, 200 exists | 404 no type, 422 bad state or address |
//! | `GET /actors/{path}` | 200 | 404 |
//! | `DELETE /actors/{path}` | 204 | 404 |
//! | `GET /actors?prefix=&cursor=&limit=` | 200 | |
//! | `POST /actors/{path}/messages` body `{id?, type, payload, deliverAt or delay, correlationId?, headers?, retry?, retention?}` | 202, 200 duplicate id | 400 invalid retry or retention policy, 404, 409 id taken, 422 type or payload |
//! | `GET /actors/{path}/messages?cursor=&limit=` | 200 | 404 |
//! | `GET /messages?prefix=&cursor=&limit=` | 200 | |
//! | `GET /messages/{id}` | 200 | 404 |
//! | `DELETE /messages/{id}` | 204 | 404, 409 running or finished |
//! | `GET /failures?prefix=&since=&cursor=&limit=` | 200 | |
//! | `GET /types` | 200 | |
//! | `GET /health` | 200 | 503 |
//!
//! Every endpoint also answers `400` for a malformed body, path or query parameter, `503` when the store is unavailable
//! (worth retrying) and `500` for an unexpected error, a panic included (a bug; not worth retrying as such). Error
//! bodies are `application/problem+json` ([`cicada::api::Problem`], types in [`cicada::api::problem_types`]) with the
//! request's original path and query as `instance`. The bodies are those of [`cicada::api`].
//!
//! **Request bodies** are strict: an unknown key in the envelope (`{"paylaod": …}`, `{"delayMs": …}`), or inside its
//! `retry` and `retention` policies, is a `400`, so a misspelling is never silently ignored. Payloads and states inside
//! them are decoded leniently, like every message. A body larger than [`ApiConfig::max_request_body_bytes`] (default
//! 1 MiB) is refused with `413`: by its `Content-Length`, or while reading, which stops one byte past the limit.
//!
//! **Paths.** `actor://a/b` is `/actors/a/b`; segments are percent-encoded exactly as in the actor URI, so
//! `actor://a/b%20c` is `/actors/a/b%20c`. A trailing segment that is literally `messages` (in the raw, still encoded
//! path) names the mailbox for `GET` and `POST`; to address an instance whose last segment is `messages`, escape it as
//! `%6Dessages` (as [`cicada::ActorAddress::to_http_path`] does). `PUT` and `DELETE` always address the instance.
//!
//! **Listings** take `cursor` (the `next` of the previous page) and `limit` (1..[`MAX_PAGE_SIZE`], default 100). A
//! `prefix` is an actor-URI prefix such as `actor://sync/`; the scheme may be left out (`sync/`).
//!
//! **Mounting.** The routers have no fallback, so a host can [`Router::nest`] or [`Router::merge`] them, and wraps the
//! API in its own authentication layers:
//!
//! ```no_run
//! # fn example(system: cicada::ActorSystem) {
//! use axum::Router;
//! use cicada_axum::{actor_api, actor_health};
//!
//! let app: Router = Router::new()
//!     .merge(actor_health(system.clone(), "/ready"))   // public readiness probe
//!     .nest("/internal", actor_api(system));           // .layer(your_auth_layer)
//! # }
//! ```

mod handlers;
mod problem;
mod request;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use cicada::ActorSystem;

/// Largest accepted `limit` of a paged listing.
pub const MAX_PAGE_SIZE: usize = 1000;

/// Default largest request body: 1 MiB.
pub const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 1 << 20;

/// Content type of error bodies (RFC 9457).
pub const PROBLEM_JSON: &str = "application/problem+json";

/// Settings of [`actor_api_with`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiConfig {
    /// The largest request body accepted; a larger one gets `413`. Positive; default
    /// [`DEFAULT_MAX_REQUEST_BODY_BYTES`].
    pub max_request_body_bytes: usize,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self { max_request_body_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES }
    }
}

pub(crate) struct Api {
    system: ActorSystem,
    max_body: usize,
}

/// The twelve endpoints of the actor HTTP API, served by `system`, with the default [`ApiConfig`]. See the crate
/// documentation.
pub fn actor_api(system: ActorSystem) -> Router {
    actor_api_with(system, ApiConfig::default())
}

/// The twelve endpoints of the actor HTTP API, served by `system`, with `config`.
///
/// # Panics
///
/// When `config.max_request_body_bytes` is zero.
pub fn actor_api_with(system: ActorSystem, config: ApiConfig) -> Router {
    assert!(config.max_request_body_bytes > 0, "max_request_body_bytes must be positive");
    let api = Arc::new(Api { system, max_body: config.max_request_body_bytes });
    Router::new()
        .route("/actors", get(handlers::list_actors))
        .route(
            "/actors/{*path}",
            get(handlers::get_path).put(handlers::put_path).delete(handlers::delete_path).post(handlers::post_path),
        )
        .route("/messages", get(handlers::list_pending))
        .route("/messages/{id}", get(handlers::message_status).delete(handlers::cancel_message))
        .route("/failures", get(handlers::failures))
        .route("/types", get(handlers::types))
        .route("/health", get(handlers::health))
        .with_state(api)
}

/// A readiness check at `path` (e.g. `/ready`): `200` with the body of the API's `GET /health` while the node is
/// running and its store is reachable, else `503 unhealthy` (problem+json). Mount it **outside** the host's
/// authentication, so probes (Kubernetes, load balancers) reach it without credentials; it tells only the node id and
/// its life-cycle state.
///
/// Use it as a readiness probe, not as a liveness probe: a store outage makes every node unready, and restarting the
/// nodes would not bring the store back.
pub fn actor_health(system: ActorSystem, path: &str) -> Router {
    let path = if path.starts_with('/') { path.to_owned() } else { format!("/{path}") };
    let api = Arc::new(Api { system, max_body: DEFAULT_MAX_REQUEST_BODY_BYTES });
    Router::new().route(&path, get(handlers::health)).with_state(api)
}
