//! The sample node on `cicada-app`: every type of the [`Catalog`], picked with `--types`, its routes and its bearer
//! token through `App::http`, and its singletons and the watch of its dead letters through `App::on_start`.

use std::ffi::OsString;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use cicada::{ActorRef, ActorSystem, CreateActorResult, WatchTarget};
use cicada_actors::DeadLetters;
use cicada_app::{App, Env, node_command};
use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Singleton};
use crate::config::{ConfigError, SampleConfig, refuse_old_variables};
use crate::e2e::EffectLog;
use crate::fakes::Fakes;

/// What `GET /` answers: which node this is and what it hosts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    /// The node id.
    pub node: String,
    /// The type names of the hosted types.
    pub types: Vec<String>,
}

/// The sample node: its `App`.
#[derive(Debug)]
pub struct Sample {
    /// The binary, ready to run.
    pub app: App,
}

impl Sample {
    /// The sample node for the arguments `args` of the process and its variables `env`. For a node command
    /// (`serve`, `worker`) a variable the sample read before 0.13.0 is refused ([`refuse_old_variables`]).
    ///
    /// Its actors record the claim owner of each turn, so `App` alone picks the node id: `--node-id`, else
    /// `CICADA_NODE_ID`, else a random one.
    pub fn new(args: &[OsString], env: &Env) -> Result<Self, ConfigError> {
        if node_command(args) {
            refuse_old_variables(env)?;
        }
        let config = SampleConfig::from_vars(env)?;
        let effects = EffectLog::new();
        let catalog = Catalog::build(effects.clone(), Fakes::default());
        let mut app = App::new().version(concat!("cicada-sample-service ", env!("CARGO_PKG_VERSION")));
        for t in catalog.types() {
            app = t.add_to(app);
        }
        let token = config.api_token.clone();
        let singletons = catalog.singletons().to_vec();
        let app = app
            .http(move |api, system| routes(api, system, effects, token))
            .on_start(move |system| async move { start(system, singletons).await });
        Ok(Self { app })
    }
}

/// The work after the start: the singletons of the hosted types, and the watch on every type of
/// `cicada.dead-letters/_` when the node hosts the dead letters.
async fn start(system: ActorSystem, singletons: Vec<Singleton>) -> anyhow::Result<()> {
    let hosted = system.hosted_types();
    for singleton in singletons.iter().filter(|s| hosted.contains(s.address.type_name())) {
        let result = system.create_actor(&singleton.address, singleton.initial_state.clone()).await?;
        if !matches!(result, CreateActorResult::Created | CreateActorResult::Exists(_)) {
            anyhow::bail!("Cannot create {}: {result:?}", singleton.address);
        }
    }
    if hosted.iter().any(|t| t.as_str() == DeadLetters::TYPE) {
        system.watch(&WatchTarget::Every, &DeadLetters::default().address()?).await?;
    }
    let types: Vec<&str> = hosted.iter().map(|t| t.as_str()).collect();
    let hosting = if types.is_empty() { "nothing (send-only)".to_owned() } else { types.join(", ") };
    tracing::info!("Sample node {}: hosting {hosting}", system.node_id());
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
