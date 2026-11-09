//! SQLite store for Cicada: one file, WAL, one writer.
//!
//! [`SqliteStore`] versions its schema with `PRAGMA user_version` and migrates an older database file when it opens it.
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
