//! SQLite store for Cicada. The store lives in `cicada::store::sqlite` (feature `sqlite`); this crate re-exports it
//! until it is removed.

pub use cicada::store::sqlite::{SqliteOptions, SqliteStore, schema};
