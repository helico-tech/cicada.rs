//! The PostgreSQL store (feature `postgres`): SKIP LOCKED claims and LISTEN/NOTIFY wake-ups. A host opens it with
//! [`Store::postgres`](crate::Store::postgres).
//!
//! [`PostgresStore`] is the multi-node backend: any number of nodes open a store on the same database and schema and
//! coordinate only through it.

mod config;
mod error;
mod listener;
pub(crate) mod schema;
mod store;

pub use config::PostgresOptions;
pub(crate) use store::PostgresStore;

#[cfg(test)]
mod upgrade_tests;
