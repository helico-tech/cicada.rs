//! The node runtime: the dispatch loop, turn execution and the in-process operations behind the HTTP API.

mod completion;
pub(crate) mod effects;
pub(crate) mod executor;
pub mod limits;
pub(crate) mod resolver;
mod results;
mod source;
mod system;
mod worker;

pub use completion::{CompleteTurnResult, StateReport, TurnCompletion, TurnEffects, TurnFailure, TurnReport, TurnSend};
pub use resolver::RegisteredType;
pub use results::{
    ActorSnapshot, BulkResult, BulkSelection, CreateActorResult, LeaseTurnsResult, MailboxEntry, NodeHealth, NodeInfo,
    OpError, RegisterContractResult, RenewTurnResult, RetryResult, SendResult, TurnLease, WatchResult,
};
pub use source::{SourceError, TurnSource};
pub use system::{
    ActorSystem, ActorSystemBuilder, BULK_LIMIT, MAX_LEASE_BATCH, MAX_LEASE_TYPES, MAX_REFUSAL_DETAIL, NodeConfig,
    NodeState, StartError, StartProblem,
};
pub use worker::{Worker, WorkerBuilder, WorkerConfig, WorkerError};
