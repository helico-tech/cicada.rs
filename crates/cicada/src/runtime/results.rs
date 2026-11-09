use std::collections::BTreeMap;

use serde_json::Value;

use crate::address::{ActorAddress, UriTemplate};
use crate::store::{ActorRecord, Message, StoreError};
use crate::time::Timestamp;

/// Result of creating an instance (`PUT /actors/{path}`). HTTP: `201`, `200`, `404`, `422`, `422`.
#[derive(Debug, Clone, PartialEq)]
pub enum CreateActorResult {
    /// The instance was created.
    Created,
    /// The instance already existed and was left untouched.
    Exists(ActorRecord),
    /// No registered type matches the address.
    NoSuchType,
    /// The initial state was refused: it cannot be decoded (checked on nodes that host the type), or the type is
    /// stateless and a state was given (checked on every node).
    InvalidState(String),
    /// The address cannot be an instance of its type, checked on nodes that host the type: its template parameters do
    /// not decode as the ref, or are not in the canonical form the ref writes (`actor://x/007` for an integer).
    InvalidAddress(String),
}

/// Result of a send from outside an actor (`POST /actors/{path}/messages`).
/// HTTP: `202`, `200`, `409`, `404`, `422`, `422`.
#[derive(Debug, Clone, PartialEq)]
pub enum SendResult {
    /// The message was enqueued, with its store-assigned `seq` and effective `deliver_at`.
    Accepted(Message),
    /// A message with this id was already accepted (within the retention window); nothing was enqueued. The envelope
    /// of that message, when it is still known.
    Duplicate(Option<Message>),
    /// The id was already accepted for a *different* message (another target or type); nothing was enqueued.
    IdConflict(Message),
    /// The target instance does not exist.
    NoSuchInstance,
    /// The target's type does not accept the message type name.
    UnknownType {
        /// The refused type name.
        message_type: String,
        /// The target's type.
        template: UriTemplate,
    },
    /// This node hosts the target's type and could not decode the payload.
    InvalidPayload {
        /// The type name.
        message_type: String,
        /// Why.
        error: String,
    },
}

/// Snapshot of an instance, as served by `GET /actors/{path}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorSnapshot {
    /// The instance.
    pub address: ActorAddress,
    /// Its type.
    pub template: UriTemplate,
    /// The decoded template parameters.
    pub params: BTreeMap<String, String>,
    /// Its state.
    pub state: Option<Value>,
    /// Its version.
    pub version: i64,
    /// Store time of creation.
    pub created_at: Timestamp,
}

/// A failed operation of [`super::ActorSystem`].
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    /// The request is invalid: a field out of bounds, a NUL character, a delivery time out of range, a bad cursor.
    /// HTTP `400`.
    #[error("{0}")]
    Invalid(String),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The node could not initialize (an invalid hosted type or a contract conflict).
    #[error(transparent)]
    Start(#[from] super::StartError),
}
