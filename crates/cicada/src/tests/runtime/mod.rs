//! Runtime tests on the in-memory store that need the store trait: they inject faults through `Faulty`, write to the
//! store directly (as a node that does not host a type would) or read it underneath the node.

mod common;

mod actor_type;
mod after_handler;
mod changes;
mod claim_limit;
mod dispatch_loop;
mod failure_cause;
mod lease;
mod lease_robustness;
mod message_retry;
mod observer;
mod operations;
mod operator;
mod overrun;
mod remote_claim_limit;
mod remote_lease;
mod shutdown_and_isolation;
mod startup;
mod turn_outcomes;
mod turn_structure;
mod typed_effects;
mod unhandled;
mod value_depth;
