//! Test support for Cicada: an in-memory store, virtual clocks and the [`TestActors`] harness, which runs actor types
//! with virtual time, as a web framework's test client runs routes without a network.

mod clock;
mod harness;
mod memory;
mod snapshots;

pub use clock::{TokioClock, VirtualClock, default_start};
pub use harness::{TestActors, TestActorsBuilder};
pub use memory::InMemoryStore;
pub use snapshots::{
    UPDATE_CONTRACTS_ENV, assert_contracts_match, assert_state_decodes, check_contracts, check_state_decodes,
    describe_changes, record_state_sample, snapshot_file_name,
};
