//! The node runtime: the dispatch loop, turn execution and the in-process operations behind the HTTP API.

pub(crate) mod effects;
pub(crate) mod executor;
pub mod limits;
pub(crate) mod resolver;
mod results;
mod system;

pub use resolver::RegisteredType;
pub use results::{
    ActorSnapshot, BulkResult, BulkSelection, CreateActorResult, LeaseTurnsResult, MailboxEntry, NodeHealth, NodeInfo,
    OpError, RegisterContractResult, RenewTurnResult, RetryResult, SendResult, TurnLease, WatchResult,
};
pub use system::{
    ActorSystem, ActorSystemBuilder, BULK_LIMIT, MAX_LEASE_BATCH, MAX_LEASE_TYPES, MAX_REFUSAL_DETAIL, NodeConfig,
    NodeState, StartError, StartProblem,
};
