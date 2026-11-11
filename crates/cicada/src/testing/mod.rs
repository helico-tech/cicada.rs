//! Test support (feature `testing`): an in-memory store, virtual clocks and the [`TestActors`] harness, which runs
//! actor types with virtual time, as a web framework's test client runs routes without a network. `cicada-test`
//! turns the feature on and exports these under the same names.

mod abandon;
mod clock;
mod harness;
mod memory;

pub use abandon::{AbandonedClaim, claim_and_abandon};
pub use clock::{TokioClock, VirtualClock, default_start};
pub use harness::{TestActors, TestActorsBuilder};
pub use memory::InMemoryStore;
