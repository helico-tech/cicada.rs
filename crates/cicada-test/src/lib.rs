//! Test support for Cicada: virtual clocks and the [`TestActors`] harness, which runs actor types on the in-memory
//! store with virtual time, as a web framework's test client runs routes without a network.
//!
//! The harness and the clocks live in `cicada::testing` (feature `testing`, which this crate turns on, and which also
//! gives `Store::memory`); this crate exports them under the names they always had, next to the contract snapshots and
//! state samples.

mod snapshots;

pub use cicada::testing::{
    AbandonedClaim, TestActors, TestActorsBuilder, TokioClock, VirtualClock, claim_and_abandon, default_start,
};
pub use snapshots::{
    UPDATE_CONTRACTS_ENV, assert_contracts_match, assert_state_decodes, check_contracts, check_state_decodes,
    describe_changes, record_state_sample, snapshot_file_name,
};
