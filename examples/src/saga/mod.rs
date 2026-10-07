//! Example 5 — orchestrator / saga with compensation.
//!
//! One actor per workflow id. Its state is the step machine; every step is a message: a command to a participant
//! ([`Inventory`], [`Payments`]) and the participant's reply back to the order. When a later step fails, the order runs
//! the compensations of the steps that succeeded (here: release the reserved stock). Each transition is one turn, so
//! the saga's state and the next command commit together and a crash never loses or repeats a step.
//!
//! Participants reply to a typed ref carried in the command (`reply_to: Order`), so the compiler checks the replies.
//!
//! Every send passes the turn's trace headers (`cicada_otel::trace_headers`): with the `ActorTelemetry` plugin
//! installed, the turns of one order form one trace; without it the headers are empty.
//!
//! [`Payments`] shows per-message retry: the *sender* decides how a charge is retried (see [`charge_retry`]).

mod inventory;
mod order;
mod payments;

use cicada::{Actor, SendOptions, Turn};
use cicada_otel::trace_headers;

pub use inventory::*;
pub use order::*;
pub use payments::*;

/// Send options carrying the current turn's trace headers.
fn traced<A: Actor>(turn: &Turn<A>) -> SendOptions {
    SendOptions { headers: trace_headers(turn), ..SendOptions::new() }
}
