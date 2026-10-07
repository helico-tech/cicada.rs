//! PostgreSQL store for Cicada: SKIP LOCKED claims and LISTEN/NOTIFY wake-ups.
//!
//! [`PostgresStore`] is the multi-node backend: any number of nodes open a store on the same database and schema and
//! coordinate only through it. The schema is the Kotlin implementation's (ADR 0004), so Kotlin and Rust nodes can share
//! one database.
//!
//! ```no_run
//! # async fn example() -> Result<(), cicada::store::StoreError> {
//! use std::sync::Arc;
//! use cicada_store_postgres::{PostgresStore, PostgresStoreConfig};
//!
//! let store = PostgresStore::connect(
//!     "postgres://actors:secret@localhost:5432/actors",
//!     PostgresStoreConfig::for_schema("orders"),
//! )
//! .await?;
//! let system = cicada::ActorSystem::builder(Arc::new(store)).build();
//! # Ok(()) }
//! ```

mod config;
mod connection;
mod error;
mod listener;
pub mod schema;
mod store;

pub use config::PostgresStoreConfig;
pub use connection::PostgresConnection;
pub use store::PostgresStore;
/// sqlx connect options, for [`PostgresStore::connect_with`].
pub use sqlx::postgres::{PgConnectOptions, PgSslMode};
