//! SQLite store: one file, WAL, one writer (feature `sqlite`).
//!
//! [`SqliteStore`] versions its schema with `PRAGMA user_version` and migrates an older database file when it opens it.
//! It sets up a new or empty file and refuses one that holds the tables of another program.
//!
//! ```no_run
//! # async fn example() -> cicada::store::StoreResult<()> {
//! use cicada::store::sqlite::{SqliteOptions, SqliteStore};
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

#[cfg(test)]
mod upgrade_tests;
