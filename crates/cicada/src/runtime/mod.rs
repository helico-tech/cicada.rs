//! The node runtime: the dispatch loop, turn execution and the in-process operations behind the HTTP API.

pub(crate) mod effects;
pub(crate) mod executor;
pub mod limits;
pub(crate) mod resolver;
mod results;
mod system;

pub use resolver::{RegisteredType, ResolvedActorType};
pub use results::{ActorSnapshot, CreateActorResult, MessageChain, OpError, SendResult};
pub use system::{ActorSystem, ActorSystemBuilder, NodeConfig, NodeState, StartError};
