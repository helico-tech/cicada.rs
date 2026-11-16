//! The sample node on `cicada-app`: every type of the [`Catalog`], picked with `--types`, its routes and its bearer
//! token through `App::http`, and its singletons and the watch of its dead letters through `App::on_start`.
//! [`Sample::executor`] is the same `App` without that `on_start` work, so that `worker --server` can run it; both
//! binaries start through [`run_process`].

use std::ffi::OsString;
use std::process::ExitCode;
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
use crate::telemetry::Telemetry;

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

/// The two definitions of the sample, `Sample::new` and `Sample::executor`.
pub type Definition = fn(&[OsString], &Env) -> Result<Sample, ConfigError>;

impl Sample {
    /// The sample node for the arguments `args` of the process and its variables `env`. For a node command
    /// (`serve`, `worker`) a variable the sample read before 0.13.0 is refused ([`refuse_old_variables`]).
    ///
    /// Its actors record the claim owner of each turn, so `App` alone picks the node id: `--node-id`, else
    /// `CICADA_NODE_ID`, else a random one.
    pub fn new(args: &[OsString], env: &Env) -> Result<Self, ConfigError> {
        let (app, catalog) = define(args, env, concat!("cicada-sample-service ", env!("CARGO_PKG_VERSION")))?;
        let singletons = catalog.singletons().to_vec();
        Ok(Self { app: app.on_start(move |system| async move { start(system, singletons).await }) })
    }

    /// The sample without bootstrap, the binary `cicada-sample-executor`: the types, routes, token and variables of
    /// [`Sample::new`], but no `on_start` work, so that `worker --server` can run it (`App` refuses `on_start` over
    /// HTTP). It creates no singletons and installs no watch in any mode: a node of [`Sample::new`] or an API caller
    /// supplies those a workload needs.
    pub fn executor(args: &[OsString], env: &Env) -> Result<Self, ConfigError> {
        let (app, _) = define(args, env, concat!("cicada-sample-executor ", env!("CARGO_PKG_VERSION")))?;
        Ok(Self { app })
    }
}

/// What both definitions share: the old variables refused for a node command, every type of the catalog, and the
/// routes with the API token.
fn define(args: &[OsString], env: &Env, version: &'static str) -> Result<(App, Catalog), ConfigError> {
    if node_command(args) {
        refuse_old_variables(env)?;
    }
    let config = SampleConfig::from_vars(env)?;
    let effects = EffectLog::new();
    let catalog = Catalog::build(effects.clone(), Fakes::default());
    let mut app = App::new().version(version);
    for t in catalog.types() {
        app = t.add_to(app);
    }
    let token = config.api_token;
    Ok((app.http(move |api, system| routes(api, system, effects, token)), catalog))
}

/// The `main` of both binaries: builds the sample with `definition` from the process's arguments and variables, adds
/// OTLP telemetry for a node command when `OTEL_EXPORTER_OTLP_ENDPOINT` is set, and runs it with `App::run_async`,
/// which handles logging, signals, the graceful stop and the exit code. Telemetry is flushed after the run. An
/// invalid sample variable exits with code 2.
pub async fn run_process(definition: Definition) -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let env: Env =
        std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
    let Sample { app } = match definition(&args, &env) {
        Ok(sample) => sample,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let wants_telemetry = SampleConfig::from_vars(&env).is_ok_and(|c| c.telemetry);
    let telemetry = if node_command(&args) && wants_telemetry {
        match Telemetry::from_env() {
            Ok(telemetry) => Some(telemetry),
            Err(e) => {
                eprintln!("error: OpenTelemetry: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let app = match &telemetry {
        Some(t) => app.observe(t.observer()),
        None => app,
    };
    let code = app.run_async().await;
    if let Some(t) = telemetry {
        t.shutdown().await;
    }
    code
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
