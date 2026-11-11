//! PostgreSQL store for Cicada. The store lives in `cicada::store::postgres` (feature `postgres`); this crate
//! re-exports it until it is removed.

pub use cicada::store::postgres::{PgConnectOptions, PgSslMode, PostgresStore, PostgresStoreConfig, schema};
