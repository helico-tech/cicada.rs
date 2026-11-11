//! The SQLite store (feature `sqlite`): one file, WAL, one writer. A host opens it with
//! [`Store::sqlite`](crate::Store::sqlite).
//!
//! [`SqliteStore`] versions its schema with `PRAGMA user_version` and migrates an older database file when it opens it.
//! It sets up a new or empty file and refuses one that holds the tables of another program.

mod conn;
mod rows;
pub(crate) mod schema;
mod store;

pub use store::SqliteOptions;
pub(crate) use store::SqliteStore;

#[cfg(test)]
mod upgrade_tests;
