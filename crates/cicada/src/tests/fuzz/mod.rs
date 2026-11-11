//! Randomised tests: parsers, the comparison of the in-memory store with SQLite, and a node simulation under injected
//! store faults. The first four modules hold what the tests share; `backends`, `parsers` and `guarantees` are the
//! tests.
//!
//! A short run is part of `cargo test`. For a long one, set the number of cases:
//! `PROPTEST_CASES=100000 cargo test -p cicada --lib tests::fuzz`.

#![allow(dead_code)]

pub mod checks;
pub mod compare;
pub mod faults;
pub mod simulation;

mod backends;
mod guarantees;
mod parsers;
