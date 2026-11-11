//! Test support (feature `testing`): virtual clocks, the [`TestActors`] harness, which runs actor types on the
//! in-memory store ([`Store::memory`](crate::Store::memory)) with virtual time, as a web framework's test client runs
//! routes without a network, and [`claim_and_abandon`] for a node that dies. `cicada-test` turns the feature on and
//! exports these under the same names.

mod abandon;
mod clock;
mod harness;
mod memory;

pub use abandon::{AbandonedClaim, claim_and_abandon};
pub use clock::{TokioClock, VirtualClock, default_start};
pub use harness::{TestActors, TestActorsBuilder};
pub(crate) use memory::InMemoryStore;
