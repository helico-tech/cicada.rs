//! The node runtime: the dispatch loop, turn execution and the in-process operations behind the HTTP API.

pub(crate) mod effects;
pub(crate) mod executor;
pub mod limits;
pub(crate) mod resolver;
mod results;
mod system;

pub use resolver::RegisteredType;
pub use results::{
    ActorSnapshot, BulkResult, BulkSelection, CreateActorResult, MailboxEntry, NodeHealth, NodeInfo, OpError,
    RetryResult, SendResult,
};
pub use system::{ActorSystem, ActorSystemBuilder, BULK_LIMIT, NodeConfig, NodeState, StartError, StartProblem};
