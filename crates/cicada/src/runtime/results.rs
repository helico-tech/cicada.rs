use std::collections::BTreeMap;

use serde_json::Value;

use crate::address::{ActorAddress, TypeName};
use crate::store::{ActorRecord, Message, MessageId, NodeId, RunningTurn, StoreError};
use crate::time::Timestamp;

/// Result of creating an instance (`PUT /actors/{type}/{key}`). HTTP: `201`, `200`, `404`, `422`, `422`.
#[derive(Debug, Clone, PartialEq)]
pub enum CreateActorResult {
    /// The instance was created.
    Created,
    /// The instance already existed and was left untouched.
    Exists(ActorRecord),
    /// No registered type matches the address.
    NoSuchType,
    /// The initial state was refused: it is `null` (checked on every node), or it cannot be decoded (checked on nodes
    /// that host the type).
    InvalidState(String),
    /// The address cannot be an instance of its type, checked on nodes that host the type: its key does
    /// not decode as the ref, or are not in the canonical form the ref writes (`x/007` for an integer).
    InvalidAddress(String),
}

/// Result of a send from outside an actor (`POST /actors/{type}/{key}/messages`).
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
        type_name: TypeName,
    },
    /// This node hosts the target's type and could not decode the payload.
    InvalidPayload {
        /// The type name.
        message_type: String,
        /// Why.
        error: String,
    },
}

/// Snapshot of an instance, as served by `GET /actors/{type}/{key}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorSnapshot {
    /// The instance.
    pub address: ActorAddress,
    /// The values of its key, by field name.
    pub params: BTreeMap<String, String>,
    /// Its state.
    pub state: Value,
    /// Its version.
    pub version: i64,
    /// Store time of creation.
    pub created_at: Timestamp,
    /// True while the mailbox is paused.
    pub paused: bool,
}

/// How long ago a node last reported itself, in the store's clock; display only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NodeHealth {
    /// Last seen less than 30 s ago.
    Alive,
    /// Last seen less than 2 min ago.
    Suspect,
    /// Last seen 2 min ago or more.
    Dead,
}

/// One node as `ActorSystem::nodes` reports it: its row, its health from the store's clock, its running turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    /// The node.
    pub id: NodeId,
    /// The names of the types it hosts; empty for a send-only node.
    pub hosted: Vec<TypeName>,
    /// Store time of its first heartbeat.
    pub started_at: Timestamp,
    /// Store time of its latest heartbeat.
    pub last_seen: Timestamp,
    /// Alive, suspect or dead, from the age of [`Self::last_seen`] at store now.
    pub status: NodeHealth,
    /// Its turns running now.
    pub running_turns: Vec<RunningTurn>,
}

/// One message of a mailbox listing.
#[derive(Debug, Clone, PartialEq)]
pub struct MailboxEntry {
    /// The message.
    pub message: Message,
    /// True when a turn is handling this message; approximate, like any listing of running work.
    pub running: bool,
}

/// Result of [`super::ActorSystem::retry_message`]. HTTP: `202`, `404`, `404`, `409`.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum RetryResult {
    /// A new message with a new id was enqueued.
    Accepted(Message),
    /// The id is unknown, removed or past retention.
    NotFound,
    /// The target instance no longer exists.
    NoSuchInstance(ActorAddress),
    /// The message is pending, running, done or purged; only `failed` and `cancelled` messages can be retried.
    NotRetryable {
        /// The message's current state, e.g. `running`.
        state: String,
    },
}

/// Which messages a bulk action applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkSelection {
    /// These messages, at most [`super::BULK_LIMIT`].
    Ids(Vec<MessageId>),
    /// Every message under the address prefix (empty: all), of one message type when given.
    Filter {
        /// A prefix of the address.
        prefix: String,
        /// Only messages of this type name.
        message_type: Option<String>,
    },
}

/// What a bulk action did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkResult {
    /// How many messages were acted on; messages that were not eligible are not counted.
    pub affected: u64,
    /// True when matches remain beyond the cap of [`super::BULK_LIMIT`].
    pub more: bool,
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
