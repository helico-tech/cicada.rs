//! The variables the sample node reads itself; the store, the address, the types and the settings of the node are those
//! of `cicada-app` (`CICADA_…`).

use std::fmt;
use std::time::Duration;

use cicada::store::NodeId;
use cicada::time::parse_duration;
use cicada_app::Env;

/// An invalid environment variable; the message names it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

/// The variables the sample service read before 0.13.0, and those of `cicada-app` that took their place.
pub const OLD_VARIABLES: [(&str, &str); 13] = [
    ("PORT", "CICADA_LISTEN"),
    ("NODE_ID", "CICADA_NODE_ID"),
    ("HOSTED_TYPES", "CICADA_TYPES"),
    ("DATABASE_URL", "CICADA_STORE"),
    ("DATABASE_SCHEMA", "CICADA_STORE_SCHEMA"),
    ("DATABASE_POOL_SIZE", "CICADA_STORE_POOL_SIZE"),
    ("SQLITE_PATH", "CICADA_STORE"),
    ("LEASE_TTL", "CICADA_LEASE_TTL"),
    ("HEARTBEAT_INTERVAL", "CICADA_HEARTBEAT_INTERVAL"),
    ("POLL_INTERVAL", "CICADA_POLL_INTERVAL"),
    ("SHUTDOWN_GRACE_PERIOD", "CICADA_SHUTDOWN_GRACE_PERIOD"),
    ("MAX_TURN", "CICADA_MAX_TURN"),
    ("CONCURRENCY", "CICADA_CONCURRENCY"),
];

/// Refuses a variable of [`OLD_VARIABLES`] that is set: a deployment that still sets `DATABASE_URL` and
/// `HOSTED_TYPES=none` would otherwise start without a word, on the SQLite file of the image, hosting every type. The
/// error names the variable that took its place. For this release only.
pub fn refuse_old_variables(env: &Env) -> Result<(), ConfigError> {
    let set: Vec<String> = OLD_VARIABLES
        .iter()
        .filter(|(old, _)| value(env, old).is_some())
        .map(|(old, new)| format!("{old} (use {new})"))
        .collect();
    if set.is_empty() {
        return Ok(());
    }
    Err(ConfigError(format!(
        "the sample service no longer reads {}; it reads the variables of cicada-app since 0.13.0",
        set.join(", ")
    )))
}

/// The variables of the sample node besides those of `cicada-app`.
///
/// | Variable | Meaning | Default |
/// | --- | --- | --- |
/// | `API_TOKEN` | when set, the HTTP API requires `Authorization: Bearer <token>` | — |
/// | `FAILURE_WATCHER_INTERVAL` | poll interval of the dead-letter watcher, on a node that hosts `cicada.dead-letters` | `10s` |
/// | `OTEL_EXPORTER_OTLP_ENDPOINT` | when set, a node exports traces and metrics there over OTLP/HTTP (`ActorTelemetry`); see [`crate::telemetry`] | — |
///
/// `Debug` leaves out the API token.
#[derive(Clone, PartialEq)]
pub struct SampleConfig {
    /// When set, the HTTP API requires `Authorization: Bearer <token>`.
    pub api_token: Option<String>,
    /// Poll interval of the dead-letter watcher.
    pub failure_watcher_interval: Duration,
    /// Whether a node exports traces and metrics over OTLP.
    pub telemetry: bool,
}

impl Default for SampleConfig {
    fn default() -> Self {
        Self { api_token: None, failure_watcher_interval: Duration::from_secs(10), telemetry: false }
    }
}

impl SampleConfig {
    /// Reads the configuration from `env`. Blank values count as unset.
    pub fn from_vars(env: &Env) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let interval = match value(env, "FAILURE_WATCHER_INTERVAL") {
            Some(v) => {
                env_duration(v).ok_or_else(|| ConfigError(format!("FAILURE_WATCHER_INTERVAL: not a duration: {v}")))?
            }
            None => defaults.failure_watcher_interval,
        };
        Ok(Self {
            api_token: value(env, "API_TOKEN").map(str::to_owned),
            failure_watcher_interval: interval,
            telemetry: value(env, "OTEL_EXPORTER_OTLP_ENDPOINT").is_some(),
        })
    }
}

impl fmt::Debug for SampleConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SampleConfig")
            .field("auth", &self.api_token.is_some())
            .field("failure_watcher_interval", &self.failure_watcher_interval)
            .field("telemetry", &self.telemetry)
            .finish()
    }
}

/// The node id the sample's actors record: `CICADA_NODE_ID`, which `cicada-app` reads too, else a random one. A value
/// that is no node id gives a random one here and an error of `cicada-app` that names the variable.
pub fn node_id(env: &Env) -> NodeId {
    value(env, "CICADA_NODE_ID").and_then(|v| NodeId::new(v).ok()).unwrap_or_else(NodeId::random)
}

/// The value of variable `name`, trimmed; `None` when it is unset or blank.
fn value<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
    env.get(name).map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// A duration from the environment: the short form `250ms`, `5s`, `2m` or `1h` (one whole number and a unit), or
/// ISO-8601 (`PT5S`).
fn env_duration(text: &str) -> Option<Duration> {
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let (number, unit) = text.split_at(digits);
    let Ok(number) = number.parse::<u64>() else {
        return parse_duration(text).ok();
    };
    match unit {
        "ms" => Some(Duration::from_millis(number)),
        "s" => Some(Duration::from_secs(number)),
        "m" => number.checked_mul(60).map(Duration::from_secs),
        "h" => number.checked_mul(3600).map(Duration::from_secs),
        _ => None,
    }
}
