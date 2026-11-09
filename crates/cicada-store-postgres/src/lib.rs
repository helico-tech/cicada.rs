//! PostgreSQL store for Cicada: SKIP LOCKED claims and LISTEN/NOTIFY wake-ups.
//!
//! [`PostgresStore`] is the multi-node backend: any number of nodes open a store on the same database and schema and
//! coordinate only through it.
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
mod error;
mod listener;
pub mod schema;
mod store;

pub use config::PostgresStoreConfig;
/// sqlx connect options, for [`PostgresStore::connect_with`].
pub use sqlx::postgres::{PgConnectOptions, PgSslMode};
pub use store::PostgresStore;
