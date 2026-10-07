//! The sample node: an [`ActorSystem`] on PostgreSQL (`DATABASE_URL`) or SQLite, hosting the example groups selected by
//! `HOSTED_TYPES`, and its HTTP routes.

use std::future::Future;
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use cicada::store::{NodeId, Store};
use cicada::{ActorSystem, CreateActorResult};
use cicada_axum::{actor_api, actor_health};
use cicada_extras::{CrashLoopGuard, FailureWatcher};
use cicada_store_postgres::{PostgresStore, PostgresStoreConfig};
use cicada_store_sqlite::{SqliteOptions, SqliteStore};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::catalog::Catalog;
use crate::config::SampleConfig;
use crate::e2e::EffectLog;
use crate::telemetry::Telemetry;

/// What `GET /` answers: which node this is and what it hosts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    /// The node id.
    pub node: String,
    /// The hosted groups.
    pub groups: Vec<String>,
    /// The templates of the hosted types.
    pub types: Vec<String>,
    /// The store, e.g. `sqlite (actors.db)`.
    pub store: String,
}

/// A running sample node: the actor system on its store, the failure watcher when it hosts `supervision`, and the
/// telemetry providers when export is configured. [`Self::router`] serves it over HTTP; [`Self::shutdown`] stops it.
#[derive(Debug)]
pub struct SampleNode {
    system: ActorSystem,
    info: NodeInfo,
    effects: EffectLog,
    api_token: Option<String>,
    watcher: Option<JoinHandle<()>>,
    telemetry: Option<Telemetry>,
}

impl SampleNode {
    /// Opens the store, starts the node with the hosted types, creates the hosted singletons (create is idempotent;
    /// every node hosting the group does it) and starts the failure watcher when the node hosts `supervision`.
    pub async fn start(config: &SampleConfig) -> anyhow::Result<Self> {
        let node_id = config.node_id.clone().map(NodeId::new).transpose().map_err(anyhow::Error::msg)?;
        let node_id = node_id.unwrap_or_else(NodeId::random);
        let effects = EffectLog::new();
        let catalog = Catalog::new(node_id.as_str(), effects.clone());
        let hosted = catalog.types(&config.groups);
        let telemetry = if config.telemetry { Some(Telemetry::from_env().context("OpenTelemetry")?) } else { None };
        let store = open_store(config).await?;

        let mut builder = ActorSystem::builder(store.clone()).node_id(node_id.clone());
        for t in &hosted {
            builder = t.host(builder);
        }
        if let Some(ttl) = config.lease_ttl {
            builder = builder.lease_ttl(ttl);
        }
        if let Some(interval) = config.heartbeat_interval {
            builder = builder.heartbeat_interval(interval);
        }
        if let Some(interval) = config.poll_interval {
            builder = builder.poll_interval(interval);
        }
        if let Some(grace) = config.shutdown_grace_period {
            builder = builder.shutdown_grace_period(grace);
        }
        if let Some(max_turn) = config.default_max_turn {
            builder = builder.default_max_turn(max_turn);
        }
        if let Some(concurrency) = config.concurrency {
            builder = builder.concurrency(concurrency);
        }
        if let Some(t) = &telemetry {
            builder = builder.install(t.plugin());
        }
        if let Some(limit) = config.crash_loop_limit {
            builder = builder.install(CrashLoopGuard::new(limit).map_err(|e| anyhow::anyhow!("CRASH_LOOP_LIMIT: {e}"))?);
        }
        let system = builder.build();
        let started = async {
            system.start().await?;
            for singleton in catalog.singletons(&config.groups) {
                let result = system.create_actor(&singleton.address, singleton.initial_state.clone()).await?;
                if !matches!(result, CreateActorResult::Created | CreateActorResult::Exists(_)) {
                    anyhow::bail!("Cannot create {}: {result:?}", singleton.address);
                }
            }
            anyhow::Ok(())
        };
        if let Err(e) = started.await {
            system.stop().await;
            store.close().await;
            if let Some(t) = telemetry {
                t.shutdown().await;
            }
            return Err(e);
        }
        let watcher = config
            .groups
            .iter()
            .any(|g| g == "supervision")
            .then(|| FailureWatcher::new(system.clone()).start(config.failure_watcher_interval));

        let info = NodeInfo {
            node: node_id.as_str().to_owned(),
            groups: config.groups.clone(),
            types: hosted.iter().map(|t| t.template().to_owned()).collect(),
            store: config.store_description(),
        };
        let hosting = if info.groups.is_empty() { "nothing (send-only)".to_owned() } else { info.groups.join(", ") };
        tracing::info!("Sample node {}: hosting {hosting} on {}", info.node, info.store);
        Ok(Self { system, info, effects, api_token: config.api_token.clone(), watcher, telemetry })
    }

    /// The actor system.
    pub fn system(&self) -> &ActorSystem {
        &self.system
    }

    /// What `GET /` answers.
    pub fn info(&self) -> &NodeInfo {
        &self.info
    }

    /// This node's log of the E2E actors' side effects.
    pub fn effects(&self) -> &EffectLog {
        &self.effects
    }

    /// The node's routes besides the fourteen of `actor_api()`, which are mounted at the root:
    ///
    /// - `GET /`: [`NodeInfo`], without authentication;
    /// - `/console/`: the actor console, static files without authentication; it asks for the token when the API
    ///   answers `401`;
    /// - `GET /ready`: readiness, without authentication, for container and Kubernetes probes;
    /// - `GET /e2e/effects[?actor=actor://…]`: this node's in-memory [`EffectLog`].
    ///
    /// With an API token the API and `/e2e/effects` require `Authorization: Bearer <token>`.
    pub fn router(&self) -> Router {
        let info = self.info.clone();
        let effects = self.effects.clone();
        let mut protected = actor_api(self.system.clone()).route("/e2e/effects", get(list_effects).with_state(effects));
        if let Some(token) = &self.api_token {
            let token: Arc<[u8]> = token.as_bytes().into();
            protected = protected.layer(middleware::from_fn_with_state(token, require_bearer));
        }
        Router::new()
            .route("/", get(move || async move { Json(info.clone()) }))
            .merge(cicada_ui::actor_ui("/console", "/"))
            .merge(actor_health(self.system.clone(), "/ready"))
            .merge(protected)
    }

    /// Stops the node: the failure watcher, then the actor system (no new claims; running turns get the shutdown grace
    /// period, then their leases are released), then the store, then the telemetry providers (flushed).
    pub async fn shutdown(self) {
        if let Some(watcher) = self.watcher {
            watcher.abort();
        }
        self.system.stop().await;
        self.system.store().close().await;
        if let Some(t) = self.telemetry {
            t.shutdown().await;
        }
        tracing::info!("Sample node {} stopped", self.info.node);
    }
}

/// Serves `node` on `listener` until `shutdown` completes, then stops accepting requests, lets the open ones finish,
/// and stops the node ([`SampleNode::shutdown`]).
pub async fn serve(node: SampleNode, listener: TcpListener, shutdown: impl Future<Output = ()> + Send + 'static) -> anyhow::Result<()> {
    let served = axum::serve(listener, node.router()).with_graceful_shutdown(shutdown).await;
    node.shutdown().await;
    Ok(served?)
}

async fn open_store(config: &SampleConfig) -> anyhow::Result<Arc<dyn Store>> {
    Ok(match &config.database_url {
        Some(url) => {
            let mut store_config = PostgresStoreConfig::for_schema(&config.database_schema);
            if let Some(size) = config.effective_pool_size() {
                store_config.max_pool_size = size;
            }
            let store = PostgresStore::connect_as(
                url,
                config.database_user.as_deref(),
                config.database_password.as_deref(),
                store_config,
            )
            .await
            .context("Opening the PostgreSQL store")?;
            Arc::new(store)
        }
        None => {
            let store = SqliteStore::open(&config.sqlite_path, SqliteOptions::default())
                .await
                .with_context(|| format!("Opening the SQLite store {}", config.sqlite_path.display()))?;
            Arc::new(store)
        }
    })
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
