//! Settings of a [`crate::store::postgres::PostgresStore`].

use std::time::Duration;

use crate::store::StoreError;
use crate::time::DEFAULT_RETENTION;

/// Settings of a [`crate::store::postgres::PostgresStore`]. Every field has a usable default.
///
/// ```
/// use std::time::Duration;
/// use cicada::store::postgres::PostgresStoreConfig;
///
/// let config = PostgresStoreConfig { schema: "orders".into(), max_pool_size: 20, ..Default::default() };
/// assert_eq!(config.socket_timeout, Duration::from_secs(30));
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
    /// The longest one transaction may take on its connection before it fails as unavailable and the connection is
    /// closed. Default 30 s, far above any transaction of the store, so a hanging connection (a dead peer without a
    /// TCP reset) turns into an error instead of blocking a claim, a commit or a node's stop forever. Zero disables
    /// it. (sqlx has no timeout on a single read.)
    pub socket_timeout: Duration,
}

impl Default for PostgresStoreConfig {
    fn default() -> Self {
        Self {
            schema: "public".to_owned(),
            retention: DEFAULT_RETENTION,
            max_pool_size: 20,
            socket_timeout: Duration::from_secs(30),
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
        Ok(())
    }

    /// The `LISTEN/NOTIFY` channel for wake-ups: `cicada_<schema>`, i.e. one channel per store, so stores in other
    /// schemas of the same database do not wake each other.
    pub fn effective_channel(&self) -> String {
        format!("cicada_{}", self.schema)
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
        assert!(invalid(PostgresStoreConfig { retention: Duration::ZERO, ..Default::default() }));
        assert!(PostgresStoreConfig { socket_timeout: Duration::ZERO, ..Default::default() }.validate().is_ok());
    }

    #[test]
    fn there_is_one_channel_per_schema() {
        assert_eq!(PostgresStoreConfig::for_schema("orders").effective_channel(), "cicada_orders");
    }

    #[test]
    fn the_default_pool_has_room_for_a_nodes_default_concurrency_plus_heartbeats_and_http() {
        let concurrency = crate::runtime::NodeConfig::default().concurrency;
        assert!(PostgresStoreConfig::default().max_pool_size as usize > concurrency);
    }

    #[test]
    fn connections_get_a_socket_timeout_by_default() {
        assert_eq!(PostgresStoreConfig::default().socket_timeout, Duration::from_secs(30));
    }
}
