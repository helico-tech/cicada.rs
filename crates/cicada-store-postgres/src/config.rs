//! Settings of a [`crate::PostgresStore`].

use std::time::Duration;

use cicada::store::StoreError;
use cicada::time::DEFAULT_RETENTION;

/// Settings of a [`crate::PostgresStore`]. Every field has a usable default.
///
/// ```
/// use std::time::Duration;
/// use cicada_store_postgres::PostgresStoreConfig;
///
/// let config = PostgresStoreConfig { schema: "orders".into(), max_pool_size: 20, ..Default::default() };
/// assert_eq!(config.connection_timeout, Duration::from_secs(10));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PostgresStoreConfig {
    /// The database schema that holds the store's tables and sequences; created on start when absent. Several stores
    /// (e.g. separate environments or tests) can share one database in separate schemas. Must be a plain lower-case
    /// identifier (`[a-z_][a-z0-9_]*`, at most 50 characters). Default `public`.
    pub schema: String,
    /// How long outcomes are retained; also the dedup window for message ids (S10). Default 7 days.
    pub retention: Duration,
    /// Maximum number of pooled connections. Default 20: the node's default turn concurrency (16) plus headroom for
    /// heartbeats, the dispatch loop and HTTP traffic. Size it to at least the node's `concurrency` plus a few; a pool
    /// that is too small delays lease renewals, and a renewal that waits too long makes the node give up the turn.
    pub max_pool_size: u32,
    /// Number of idle connections the pool keeps open. Default 1.
    pub min_idle: u32,
    /// How long an operation waits for a pooled connection before failing. Default 10 s.
    pub connection_timeout: Duration,
    /// The longest one transaction may take on its connection before it fails as unavailable and the connection is
    /// closed. Default 30 s, far above any transaction of the store, so a hanging connection (a dead peer without a
    /// TCP reset) turns into an error instead of blocking a claim, a commit or a node's stop forever. Zero disables
    /// it. (The Kotlin store sets pgjdbc's `socketTimeout`, which bounds every single read; sqlx has no equivalent.)
    pub socket_timeout: Duration,
    /// The `LISTEN/NOTIFY` channel for wake-ups. `None` (the default) means `cicada_<schema>`, i.e. one channel per
    /// store, so stores in other schemas of the same database do not wake each other.
    pub channel: Option<String>,
    /// First delay before the wake-up listener reconnects after losing its connection; doubles up to 30 seconds.
    /// Default 250 ms.
    pub listener_reconnect_delay: Duration,
    /// How many times a transaction is re-run after PostgreSQL aborted it for a deadlock or a serialization failure
    /// (both are transient under concurrent commits from several nodes) before the error is reported. Default 8.
    pub transaction_retries: u32,
}

impl Default for PostgresStoreConfig {
    fn default() -> Self {
        Self {
            schema: "public".to_owned(),
            retention: DEFAULT_RETENTION,
            max_pool_size: 20,
            min_idle: 1,
            connection_timeout: Duration::from_secs(10),
            socket_timeout: Duration::from_secs(30),
            channel: None,
            listener_reconnect_delay: Duration::from_millis(250),
            transaction_retries: 8,
        }
    }
}

impl PostgresStoreConfig {
    /// The default settings on `schema`.
    pub fn for_schema(schema: impl Into<String>) -> Self {
        Self { schema: schema.into(), ..Self::default() }
    }

    /// Checks every setting; the error names the first invalid one.
    pub fn validate(&self) -> Result<(), StoreError> {
        let check = |ok: bool, message: String| if ok { Ok(()) } else { Err(StoreError::Invalid(message)) };
        check(
            is_identifier(&self.schema, 50),
            format!("Invalid schema name '{}': use [a-z_][a-z0-9_]*, at most 50 characters", self.schema),
        )?;
        check(!self.retention.is_zero(), "retention must be positive".into())?;
        check(self.max_pool_size > 0, "max_pool_size must be positive".into())?;
        check(self.min_idle <= self.max_pool_size, "min_idle must be between 0 and max_pool_size".into())?;
        check(!self.connection_timeout.is_zero(), "connection_timeout must be positive".into())?;
        check(!self.listener_reconnect_delay.is_zero(), "listener_reconnect_delay must be positive".into())?;
        if let Some(channel) = &self.channel {
            check(
                is_identifier(channel, 63),
                format!("Invalid channel name '{channel}': use [a-z_][a-z0-9_]*, at most 63 characters"),
            )?;
        }
        Ok(())
    }

    /// The channel actually used: [`Self::channel`] or `cicada_<schema>`.
    pub fn effective_channel(&self) -> String {
        self.channel.clone().unwrap_or_else(|| format!("cicada_{}", self.schema))
    }
}

/// `[a-z_][a-z0-9_]*` with at most `max` characters.
fn is_identifier(name: &str, max: usize) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    name.len() <= max
        && (first.is_ascii_lowercase() || first == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_settings_are_refused() {
        let invalid = |config: PostgresStoreConfig| matches!(config.validate(), Err(StoreError::Invalid(_)));
        assert!(PostgresStoreConfig::default().validate().is_ok());
        assert!(invalid(PostgresStoreConfig::for_schema("Robert'); DROP TABLE students;--")));
        assert!(invalid(PostgresStoreConfig::for_schema("")));
        assert!(invalid(PostgresStoreConfig::for_schema("a".repeat(51))));
        assert!(PostgresStoreConfig::for_schema("a".repeat(50)).validate().is_ok());
        assert!(invalid(PostgresStoreConfig { max_pool_size: 0, ..Default::default() }));
        assert!(invalid(PostgresStoreConfig { min_idle: 21, ..Default::default() }));
        assert!(invalid(PostgresStoreConfig { retention: Duration::ZERO, ..Default::default() }));
        assert!(invalid(PostgresStoreConfig { channel: Some("no spaces".into()), ..Default::default() }));
        assert!(PostgresStoreConfig { socket_timeout: Duration::ZERO, ..Default::default() }.validate().is_ok());
    }

    #[test]
    fn the_channel_defaults_to_one_per_schema() {
        assert_eq!(PostgresStoreConfig::for_schema("orders").effective_channel(), "cicada_orders");
        let custom = PostgresStoreConfig { channel: Some("wake".into()), ..Default::default() };
        assert_eq!(custom.effective_channel(), "wake");
    }

    #[test]
    fn the_default_pool_has_room_for_a_nodes_default_concurrency_plus_heartbeats_and_http() {
        let concurrency = cicada::runtime::NodeConfig::default().concurrency;
        assert!(PostgresStoreConfig::default().max_pool_size as usize > concurrency);
    }

    #[test]
    fn connections_get_a_socket_timeout_by_default() {
        assert_eq!(PostgresStoreConfig::default().socket_timeout, Duration::from_secs(30));
    }
}
