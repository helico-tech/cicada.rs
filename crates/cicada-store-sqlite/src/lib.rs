//! SQLite store for Cicada: one file, WAL, one writer.
//!
//! [`SqliteStore`] keeps the schema of the Kotlin implementation (`actors-store-sqlite`), migrations and
//! `PRAGMA user_version` included, so a database file written by either implementation opens with the other.
//!
//! ```no_run
//! # async fn example() -> cicada::store::StoreResult<()> {
//! use cicada_store_sqlite::{SqliteOptions, SqliteStore};
//!
//! let store = SqliteStore::open("actors.db", SqliteOptions::default()).await?;
//! # Ok(())
//! # }
//! ```

mod conn;
mod rows;
pub mod schema;
mod store;

pub use store::{SqliteOptions, SqliteStore};
