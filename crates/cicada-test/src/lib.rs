//! Test support for Cicada: an in-memory store, virtual clocks and the [`TestActors`] harness, which runs actor types
//! with virtual time, as a web framework's test client runs routes without a network.
//!
//! The harness, the clocks and the store live in `cicada::testing` (feature `testing`, which this crate turns on);
//! this crate exports them under the names they always had, next to the contract snapshots and state samples.

mod snapshots;

pub use cicada::testing::{
    AbandonedClaim, InMemoryStore, TestActors, TestActorsBuilder, TokioClock, VirtualClock, claim_and_abandon,
    default_start,
};
pub use snapshots::{
    UPDATE_CONTRACTS_ENV, assert_contracts_match, assert_state_decodes, check_contracts, check_state_decodes,
    describe_changes, record_state_sample, snapshot_file_name,
};
