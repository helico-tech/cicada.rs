//! The tests that need the store trait, which is private to this crate: the store conformance suite, the tests of
//! the SQLite and PostgreSQL stores, the randomised tests and the runtime tests that inject faults or write to a
//! store directly. The tests that need only the public API are in `tests/` and in the other crates.

pub(crate) mod conformance;
mod fuzz;
pub(crate) mod postgres;
mod retry_policy;
pub(crate) mod runtime;
mod sqlite;
mod store_support;
