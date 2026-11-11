//! Cicada: durable actors on a shared transactional SQL store.
//!
//! An actor is a row of state plus a mailbox in a shared store (PostgreSQL or SQLite); it occupies memory only while
//! one of its messages is being handled. Every service that runs an [`ActorSystem`] is a *node*. Nodes are peers that
//! coordinate only through the store: there is no coordinator and no cluster protocol.
//!
//! - **Durable turns.** Claim → load → handle → commit. New state, outgoing messages, creates and the message's outcome
//!   are written in one transaction; a node that dies mid-turn loses its lease and the turn is re-run elsewhere.
//! - **One turn at a time per actor**, messages in delivery-time order, delayed messages never early, exactly one
//!   outcome per message (`done`, `failed`, `cancelled`, `purged`).
//! - **Typed and checked.** Actor addresses are typed refs: a type name and a key (`sync/acme`). The
//!   compiler checks that a ref accepts a message, and that a handler covers every message of its family.
//!
//! See [`actor::Actor`] for defining an actor type and [`ActorSystem`] for running a node.

pub mod actor;
pub mod address;
pub mod api;
pub mod contract;
pub mod message;
pub mod plugin;
pub mod retention;
pub mod retry;
pub mod runtime;
pub mod schema;
pub mod store;
pub mod time;
pub mod turn;

pub use actor::{Actor, ActorType, Unhandled};
pub use address::{ActorAddress, ActorRef, TypeName};
pub use message::NoState;
pub use runtime::{ActorSystem, CreateActorResult, OpError, SendResult, StartError};
pub use turn::{HandlerError, HandlerResult, SendOptions, Turn};

/// Everything needed to define actor types.
pub mod prelude {
    pub use schemars::JsonSchema;

    pub use crate::actor::{Actor, ActorType, Unhandled};
    pub use crate::address::{ActorAddress, ActorRef};
    pub use crate::message::NoState;
    pub use crate::retention::{Keep, Retention};
    pub use crate::retry::{Backoff, RetryPolicy};
    pub use crate::runtime::ActorSystem;
    pub use crate::store::MessageId;
    pub use crate::turn::{HandlerError, HandlerResult, SendOptions, Turn};
}
