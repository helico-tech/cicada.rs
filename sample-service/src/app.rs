//! The sample node on `cicada-app`: every type of the [`Catalog`], picked with `--types`, its routes and its bearer
//! token through `App::http`, and its singletons and failure watcher through `App::on_start`.

use std::ffi::OsString;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use cicada::runtime::NodeConfig;
use cicada::store::NodeId;
use cicada::{ActorRef, ActorSystem, CreateActorResult};
use cicada_app::{App, Env, node_command};
use cicada_extras::{DeadLetters, FailureWatcher};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use crate::catalog::{Catalog, Singleton};
use crate::config::{ConfigError, SampleConfig, node_id, refuse_old_variables};
use crate::e2e::EffectLog;

/// What `GET /` answers: which node this is and what it hosts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    /// The node id.
    pub node: String,
    /// The type names of the hosted types.
    pub types: Vec<String>,
}

/// The sample node: its `App`, and the failure watcher that its `on_start` may start.
#[derive(Debug)]
pub struct Sample {
    /// The binary, ready to run.
    pub app: App,
    /// The failure watcher, once started; abort it when the run of `app` returned.
    pub watcher: Watcher,
}

/// The failure watcher of a node that hosts the dead letters.
#[derive(Debug, Clone, Default)]
pub struct Watcher(Arc<Mutex<Option<JoinHandle<()>>>>);

impl Watcher {
    fn set(&self, handle: JoinHandle<()>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
    }

    /// Stops the watcher, when it was started.
    pub fn abort(&self) {
        if let Some(handle) = self.0.lock().unwrap_or_else(PoisonError::into_inner).take() {
            handle.abort();
        }
    }
}

impl Sample {
    /// The sample node for the arguments `args` of the process and its variables `env`. For a node command
    /// (`serve`, `worker`) a variable the sample read before 0.13.0 is refused ([`refuse_old_variables`]).
    ///
    /// Its actors record the node id when they are built, before the node exists, so it reads `CICADA_NODE_ID` itself
    /// and passes it to `App` in `node_config`; `on_start` fails when the node runs under another id (`--node-id`).
    pub fn new(args: &[OsString], env: &Env) -> Result<Self, ConfigError> {
        if node_command(args) {
            refuse_old_variables(env)?;
        }
        let config = SampleConfig::from_vars(env)?;
        let node_id = node_id(env);
        let effects = EffectLog::new();
        let catalog = Catalog::new(node_id.as_str(), effects.clone());
        let mut app = App::new()
            .version(concat!("cicada-sample-service ", env!("CARGO_PKG_VERSION")))
            .node_config(NodeConfig { node_id: node_id.clone(), ..NodeConfig::default() });
        for t in catalog.types() {
            app = t.add_to(app);
        }
        let token = config.api_token.clone();
        let watcher = Watcher::default();
        let started = watcher.clone();
        let singletons = catalog.singletons().to_vec();
        let interval = config.failure_watcher_interval;
        let app = app
            .http(move |api, system| routes(api, system, effects, token))
            .on_start(move |system| async move { start(system, node_id, singletons, interval, started).await });
        Ok(Self { app, watcher })
    }
}

/// The work after the start: the node id check, the singletons of the hosted types, and the failure watcher when the
/// node hosts the dead letters.
async fn start(
    system: ActorSystem,
    node_id: NodeId,
    singletons: Vec<Singleton>,
    interval: std::time::Duration,
    watcher: Watcher,
) -> anyhow::Result<()> {
    if system.node_id() != &node_id {
        anyhow::bail!(
            "the node runs as {} while the actors of the sample record {node_id}: set CICADA_NODE_ID, not --node-id",
            system.node_id()
        );
    }
    let hosted = system.hosted_types();
    for singleton in singletons.iter().filter(|s| hosted.contains(s.address.type_name())) {
        let result = system.create_actor(&singleton.address, singleton.initial_state.clone()).await?;
        if !matches!(result, CreateActorResult::Created | CreateActorResult::Exists(_)) {
            anyhow::bail!("Cannot create {}: {result:?}", singleton.address);
        }
    }
    if hosted.iter().any(|t| t.as_str() == DeadLetters::TYPE) {
        watcher.set(FailureWatcher::new(system.clone()).start(interval));
    }
    let types: Vec<&str> = hosted.iter().map(|t| t.as_str()).collect();
    let hosting = if types.is_empty() { "nothing (send-only)".to_owned() } else { types.join(", ") };
    tracing::info!("Sample node {node_id}: hosting {hosting}");
    Ok(())
}

/// The node's routes besides those of `actor_api()` (`api`), at the root:
///
/// - `GET /`: [`NodeInfo`], without authentication;
/// - `GET /e2e/effects[?actor=<address>]`: this node's in-memory [`EffectLog`].
///
/// With an API token the API and `/e2e/effects` require `Authorization: Bearer <token>`. `GET /ready` is `App`'s.
fn routes(api: Router, system: &ActorSystem, effects: EffectLog, token: Option<String>) -> Router {
    let info = NodeInfo {
        node: system.node_id().as_str().to_owned(),
        types: system.hosted_types().iter().map(|t| t.as_str().to_owned()).collect(),
    };
    let mut protected = api.route("/e2e/effects", get(list_effects).with_state(effects));
    if let Some(token) = token {
        let token: Arc<[u8]> = token.as_bytes().into();
        protected = protected.layer(middleware::from_fn_with_state(token, require_bearer));
    }
    Router::new().route("/", get(move || async move { Json(info.clone()) })).merge(protected)
}

#[derive(Deserialize)]
struct EffectsQuery {
    actor: Option<String>,
}

async fn list_effects(State(effects): State<EffectLog>, Query(query): Query<EffectsQuery>) -> Response {
    Json(effects.list(query.actor.as_deref())).into_response()
}

/// Lets a request through only with `Authorization: Bearer <token>`, compared in constant time; else `401`.
async fn require_bearer(State(expected): State<Arc<[u8]>>, request: Request, next: Next) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim());
    match given {
        Some(token) if constant_time_eq(token.as_bytes(), &expected) => next.run(request).await,
        _ => {
            let mut response = StatusCode::UNAUTHORIZED.into_response();
            response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer realm=\"api\""));
            response
        }
    }
}

/// Compares in time that depends on the lengths only, not on where the inputs differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn tokens_compare_by_content() {
        assert!(constant_time_eq(b"s3cret", b"s3cret"));
        assert!(!constant_time_eq(b"s3cret", b"s3creT"));
        assert!(!constant_time_eq(b"s3cret", b"s3cret!"));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
