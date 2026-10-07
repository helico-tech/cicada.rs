//! Test support for Cicada: an in-memory store, virtual clocks and the [`TestActors`] harness, which runs actor types
//! with virtual time, as a web framework's test client runs routes without a network.

mod clock;
mod harness;
mod memory;

pub use clock::{TokioClock, VirtualClock, default_start};
pub use harness::{TestActors, TestActorsBuilder};
pub use memory::InMemoryStore;
