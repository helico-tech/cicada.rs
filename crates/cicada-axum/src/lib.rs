//! The Cicada HTTP API as axum routers: [`actor_api`] serves all of it for a node's [`ActorSystem`], [`actor_api_read`]
//! only what reads, and [`actor_health`] a public readiness check. Both API routers serve their OpenAPI 3.1 document at
//! `GET /openapi.json`; [`openapi_document`] gives the document of the full API, and `docs/openapi.json` is a copy.
//!
//! | Method and path | Success | Errors |
//! | --- | --- | --- |
//! | `PUT /actors/{type}/{key}` body `{state}` | 201 created, 200 exists | 400 no body or no state, 404 no type, 422 null state, a state that does not match its schema or cannot be decoded, bad address |
//! | `GET /actors/{type}/{key}` | 200 | 404 |
//! | `DELETE /actors/{type}/{key}` | 204 | 404 |
//! | `GET /actors?prefix=&cursor=&limit=` | 200 | |
//! | `GET /actors/states?prefix=&cursor=&limit=` (each actor with its state, one store read per page) | 200 | |
//! | `GET /changes?prefix=` (change hints as Server-Sent Events) | 200 `text/event-stream` | |
//! | `POST /actors/{type}/{key}/messages` body `{id?, type, payload, deliverAt or delay, correlationId?, headers?, retry?, retention?}` | 202, 200 duplicate id | 400 invalid retry or retention policy, 404, 409 id taken, 422 unknown type, a payload that does not match its schema or cannot be decoded |
//! | `GET /actors/{type}/{key}/mailbox?cursor=&limit=` | 200 | 404 |
//! | `POST /actors/{type}/{key}/mailbox/pause` | 204 | 404 |
//! | `POST /actors/{type}/{key}/mailbox/resume` | 204 | 404 |
//! | `POST /actors/{type}/{key}/mailbox/purge` | 200 `{purged}` | 404 |
//! | `GET /actors/{type}/{key}/turn` | 200 | 404 no running turn |
//! | `POST /actors/{type}/{key}/turn/cancel` | 202 | 404 no running turn |
//! | `GET /messages?prefix=&cursor=&limit=` | 200 | |
//! | `GET /messages/{id}` | 200 | 404 |
//! | `POST /messages/{id}/cancel` | 204 | 404, 409 running or finished |
//! | `POST /messages/{id}/retry` | 202 | 404, 409 not retryable |
//! | `POST /messages/{id}/deliver-now` | 200 | 404, 409 running or finished |
//! | `POST /messages/bulk-cancel` body `{ids? or prefix?, messageType?}` | 200 `{affected, more}` | 400 |
//! | `POST /messages/bulk-retry` body `{ids? or prefix?, messageType?}` | 200 `{affected, more}` | 400 |
//! | `GET /nodes` | 200 | |
//! | `GET /nodes/{node}` | 200 | 404 |
//! | `GET /turns?node=&prefix=&cursor=&limit=` | 200 | |
//! | `GET /messages/{id}/caused?cursor=&limit=` | 200 | 404 |
//! | `GET /outcomes?status=&prefix=&since=&cursor=&limit=` | 200 | 400 unknown status |
//! | `GET /stats?prefix=` | 200 `{actors, pending, oldestDue}` | |
//! | `GET /types` | 200 | |
//! | `GET /types/{name}` | 200 | 404 |
//! | `DELETE /types/{name}` | 204 | 404, 409 actors of the type exist |
//! | `GET /health` | 200 | 503 |
//!
//! Every endpoint also answers `400` for a malformed body, path or query parameter, `503` when the store is unavailable
//! (worth retrying) and `500` for an unexpected error, a panic included (a bug; not worth retrying as such). Error
//! bodies are `application/problem+json` ([`cicada::api::Problem`], types in [`cicada::api::problem_types`]) with the
//! request's original path and query as `instance`. The bodies are those of [`cicada::api`].
//!
//! **Request bodies** are strict: an unknown key in the envelope (`{"paylaod": …}`, `{"delayMs": …}`), or inside its
//! `retry` and `retention` policies, is a `400`, so a misspelling is never silently ignored. A body larger than
//! [`ApiConfig::max_request_body_bytes`] (default 1 MiB) is refused with `413`: by its `Content-Length`, or while
//! reading, which stops one byte past the limit.
//!
//! **The schema check** (ADR 0026). The payload of a send and the state of a create are checked against the registered
//! JSON Schema of their message type or state, on every node, whether or not it hosts the type, and refused with `422
//! invalid-payload` or `422 invalid-state`; a node that hosts the type then decodes them with serde, with the same
//! answer. The detail says which check refused. A refusal by the schema names no place (`Payload of 'Add' does not
//! match the registered schema; GET /types/counter has it`): the check asks only whether the value matches, so a
//! refused request costs time and memory in proportion to its size. Nothing else runs after a refusal, not even the
//! decoder of a node that hosts the type. A send that repeats an id it finds pending or still shown is answered as a
//! repeat first. The check, with the validator `jsonschema` on draft 2020-12, asserts no `format`, loads nothing from
//! outside the schema, and lets through what a schema it cannot compile would check, with a warning once per schema.
//! [`assert_message_sample`] and [`assert_state_sample`] test a sample against both the schema and serde.
//!
//! **Paths.** The actor `sync/acme` is `/actors/sync/acme`; its key is written exactly as in the address.
//!
//! **Listings** take `cursor` (the `next` of the previous page) and `limit` (1..[`MAX_PAGE_SIZE`], default 100). A
//! `prefix` is a prefix of the address, such as `sync/` (one type) or `sync/ac`.
//!
//! **Change hints** (ADR 0035). `GET /changes?prefix=` streams the hints of [`cicada::ActorSystem::changes`] as
//! Server-Sent Events: `event: changed` with `data: {"address":"<canonical address>"}` (read that actor again) and
//! `event: resync` with `data: {}` (list the prefix again), the first event a `resync` once the store's source is live.
//! The prefix is the literal one of the listings; other query keys are ignored. No `id` or `retry` field, no replay
//! (`Last-Event-ID` is ignored), and a `: keep-alive` comment after each 15 seconds without output, which says nothing
//! about readiness. The `200` comes at once, with `Cache-Control: no-cache` and `X-Accel-Buffering: no`. Closing the
//! store ends the body without a final event (a request after the close gets an empty `200`, not `503`); stopping the
//! node does not. A host whose graceful shutdown waits for open requests ends these streams itself, e.g. by closing
//! the store in its shutdown future after the node stopped.
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

mod changes;
mod handlers;
mod problem;
mod request;
mod routes;
mod schema_check;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use cicada::ActorSystem;

use routes::Access;
pub use routes::openapi_document;
pub use schema_check::{assert_message_sample, assert_state_sample};

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

/// The whole actor HTTP API, served by `system`, with the default [`ApiConfig`]. See the crate documentation.
pub fn actor_api(system: ActorSystem) -> Router {
    actor_api_with(system, ApiConfig::default())
}

/// The whole actor HTTP API, served by `system`, with `config`.
///
/// # Panics
///
/// When `config.max_request_body_bytes` is zero.
pub fn actor_api_with(system: ActorSystem, config: ApiConfig) -> Router {
    assert!(config.max_request_body_bytes > 0, "max_request_body_bytes must be positive");
    routes::router(Access::Full, Arc::new(Api { system, max_body: config.max_request_body_bytes }))
}

/// Only the routes of the actor HTTP API that read, served by `system`. It has no route that changes anything, so it is
/// safe to expose more widely than [`actor_api`]; authentication stays the host's.
pub fn actor_api_read(system: ActorSystem) -> Router {
    routes::router(Access::Read, Arc::new(Api { system, max_body: DEFAULT_MAX_REQUEST_BODY_BYTES }))
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
