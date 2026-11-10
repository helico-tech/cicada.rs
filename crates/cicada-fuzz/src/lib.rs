//! Randomised tests for Cicada. The tests live in `tests/`; this crate holds what they share.
//!
//! A short run is part of `cargo test`. For a long one, set the number of cases:
//! `PROPTEST_CASES=100000 cargo test -p cicada-fuzz`.

pub mod checks;
pub mod compare;
pub mod faults;
pub mod simulation;
