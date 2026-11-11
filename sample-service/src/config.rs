//! Settings of the sample node, read from environment variables.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use cicada::store::PostgresOptions;
use cicada::time::parse_duration;

use crate::catalog::{GROUPS, parse_groups};

/// An invalid environment variable; the message names it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

/// Settings of the sample node (see [`SampleConfig::from_env`] for the variables).
///
/// `Debug` leaves out the API token and the database URL, which holds the credentials.
#[derive(Clone, PartialEq)]
pub struct SampleConfig {
    /// HTTP port.
    pub port: u16,
    /// Node id; random when `None`.
    pub node_id: Option<String>,
    /// The example groups this node hosts (runs turns for); empty for a send-only node.
    pub groups: Vec<String>,
    /// PostgreSQL URL; selects PostgreSQL. SQLite when `None`.
    pub database_url: Option<String>,
    /// PostgreSQL schema of the store.
    pub database_schema: String,
    /// PostgreSQL pool size; see [`Self::effective_pool_size`].
    pub database_pool_size: Option<u32>,
    /// SQLite file, used when `database_url` is `None`.
    pub sqlite_path: PathBuf,
    /// When set, the HTTP API requires `Authorization: Bearer <token>`.
    pub api_token: Option<String>,
    /// Lease TTL; the library default when `None`.
    pub lease_ttl: Option<Duration>,
    /// Heartbeat interval; the library default when `None`.
    pub heartbeat_interval: Option<Duration>,
    /// Poll interval; the library default when `None`.
    pub poll_interval: Option<Duration>,
    /// Shutdown grace period; the library default when `None`.
    pub shutdown_grace_period: Option<Duration>,
    /// Default maximum turn duration; the library default when `None`.
    pub default_max_turn: Option<Duration>,
    /// Maximum concurrent turns; the library default (16) when `None`.
    pub concurrency: Option<usize>,
    /// Poll interval of the dead-letter watcher (group `supervision`).
    pub failure_watcher_interval: Duration,
    /// Whether traces and metrics are exported over OTLP.
    pub telemetry: bool,
    /// When set, `CrashLoopGuard` fails a message after this many claims without a commit.
    pub crash_loop_limit: Option<u32>,
}

impl Default for SampleConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            node_id: None,
            groups: GROUPS.iter().map(|g| (*g).to_owned()).collect(),
            database_url: None,
            database_schema: "public".to_owned(),
            database_pool_size: None,
            sqlite_path: PathBuf::from("actors.db"),
            api_token: None,
            lease_ttl: None,
            heartbeat_interval: None,
            poll_interval: None,
            shutdown_grace_period: None,
            default_max_turn: None,
            concurrency: None,
            failure_watcher_interval: Duration::from_secs(10),
            telemetry: false,
            crash_loop_limit: None,
        }
    }
}

impl SampleConfig {
    /// Reads the configuration from the process environment; see [`Self::from_vars`].
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(&std::env::vars().collect())
    }

    /// Reads the configuration from `vars`. Blank values count as unset.
    ///
    /// | Variable | Meaning | Default |
    /// | --- | --- | --- |
    /// | `PORT` | HTTP port | 8080 |
    /// | `NODE_ID` | node id (lease owner, shown by `/health`, recorded by the E2E actors) | random |
    /// | `HOSTED_TYPES` | `all`, `none`, or a comma-separated list of groups (see [`GROUPS`]) | `all` |
    /// | `DATABASE_URL` | PostgreSQL, `postgres://user:pass@host:5432/db`; selects PostgreSQL | — |
    /// | `DATABASE_SCHEMA` | PostgreSQL schema of the store | `public` |
    /// | `DATABASE_POOL_SIZE` | PostgreSQL connection pool size | the store's default (20), or `CONCURRENCY` + 4 when larger |
    /// | `SQLITE_PATH` | SQLite file, used when `DATABASE_URL` is not set | `actors.db` |
    /// | `API_TOKEN` | when set, the HTTP API requires `Authorization: Bearer <token>` | — |
    /// | `LEASE_TTL`, `HEARTBEAT_INTERVAL`, `POLL_INTERVAL`, `SHUTDOWN_GRACE_PERIOD`, `MAX_TURN` | node timings, as short (`5s`, `250ms`) or ISO-8601 (`PT5S`) durations | library defaults |
    /// | `CONCURRENCY` | maximum concurrent turns | 16 |
    /// | `FAILURE_WATCHER_INTERVAL` | poll interval of the dead-letter watcher (group `supervision`) | `10s` |
    /// | `OTEL_EXPORTER_OTLP_ENDPOINT` | when set, traces and metrics go there over OTLP/HTTP (`ActorTelemetry`); see [`crate::telemetry`] | — |
    /// | `CRASH_LOOP_LIMIT` | when set, `CrashLoopGuard` fails a message after this many claims without a commit | — |
    pub fn from_vars(vars: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let get = |name: &str| vars.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()).map(str::to_owned);
        let duration = |name: &str| -> Result<Option<Duration>, ConfigError> {
            get(name)
                .map(|v| env_duration(&v).ok_or_else(|| ConfigError(format!("{name}: not a duration: {v}"))))
                .transpose()
        };
        fn number<T: std::str::FromStr>(name: &str, value: Option<String>) -> Result<Option<T>, ConfigError> {
            value.map(|v| v.parse().map_err(|_| ConfigError(format!("{name}: not a number: {v}")))).transpose()
        }
        let defaults = Self::default();
        Ok(Self {
            port: number("PORT", get("PORT"))?.unwrap_or(defaults.port),
            node_id: get("NODE_ID"),
            groups: parse_groups(&get("HOSTED_TYPES").unwrap_or_else(|| "all".to_owned())).map_err(ConfigError)?,
            database_url: get("DATABASE_URL"),
            database_schema: get("DATABASE_SCHEMA").unwrap_or(defaults.database_schema),
            database_pool_size: number("DATABASE_POOL_SIZE", get("DATABASE_POOL_SIZE"))?,
            sqlite_path: get("SQLITE_PATH").map(PathBuf::from).unwrap_or(defaults.sqlite_path),
            api_token: get("API_TOKEN"),
            lease_ttl: duration("LEASE_TTL")?,
            heartbeat_interval: duration("HEARTBEAT_INTERVAL")?,
            poll_interval: duration("POLL_INTERVAL")?,
            shutdown_grace_period: duration("SHUTDOWN_GRACE_PERIOD")?,
            default_max_turn: duration("MAX_TURN")?,
            concurrency: number("CONCURRENCY", get("CONCURRENCY"))?,
            failure_watcher_interval: duration("FAILURE_WATCHER_INTERVAL")?
                .unwrap_or(defaults.failure_watcher_interval),
            telemetry: get("OTEL_EXPORTER_OTLP_ENDPOINT").is_some(),
            crash_loop_limit: number("CRASH_LOOP_LIMIT", get("CRASH_LOOP_LIMIT"))?,
        })
    }

    /// The PostgreSQL pool size to configure, or `None` for the store's default (20, enough for the default concurrency
    /// of 16 plus heartbeats and HTTP): [`Self::database_pool_size`] when set, else room for [`Self::concurrency`]
    /// turns plus 4.
    pub fn effective_pool_size(&self) -> Option<u32> {
        self.database_pool_size.or_else(|| {
            self.concurrency.map(|c| {
                let needed = u32::try_from(c.saturating_add(4)).unwrap_or(u32::MAX);
                needed.max(PostgresOptions::default().max_pool_size)
            })
        })
    }

    /// Which store the node uses, for logs and `GET /`.
    pub fn store_description(&self) -> String {
        if self.database_url.is_some() {
            format!("postgres (schema {})", self.database_schema)
        } else {
            format!("sqlite ({})", self.sqlite_path.display())
        }
    }
}

impl fmt::Debug for SampleConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SampleConfig")
            .field("port", &self.port)
            .field("node_id", &self.node_id)
            .field("groups", &self.groups)
            .field("store", &self.store_description())
            .field("auth", &self.api_token.is_some())
            .field("lease_ttl", &self.lease_ttl)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("poll_interval", &self.poll_interval)
            .field("shutdown_grace_period", &self.shutdown_grace_period)
            .field("default_max_turn", &self.default_max_turn)
            .field("concurrency", &self.concurrency)
            .field("telemetry", &self.telemetry)
            .field("crash_loop_limit", &self.crash_loop_limit)
            .finish()
    }
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
