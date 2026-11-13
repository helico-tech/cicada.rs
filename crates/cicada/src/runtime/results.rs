use std::time::Duration;

use serde_json::Value;

use super::resolver::RegisteredType;
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
    /// The initial state was refused: it is `null` (checked on every node), it does not match the state schema (checked
    /// on every node by the check of [`ActorSystem::create_actor_checked`](super::ActorSystem::create_actor_checked)),
    /// or it matches and cannot be decoded (checked on nodes that host the type). Why.
    InvalidState(String),
    /// The address cannot be an instance of its type, checked on nodes that host the type: the ref does not accept its
    /// name (a singleton accepts only `_`, a ref with a reader of its own may refuse a name).
    InvalidAddress(String),
}

/// Result of [`ActorSystem::register_contract`](super::ActorSystem::register_contract) (`PUT /types/{name}`). HTTP:
/// `200`, `409`, `422`.
#[derive(Debug, Clone)]
pub enum RegisterContractResult {
    /// The type is registered: newly, or it already was with the same hash. The stored contract, which keeps the
    /// descriptions of the first registration.
    Registered(RegisteredType),
    /// The type is registered with a different hash; nothing changed.
    Conflict {
        /// The registration that stays.
        existing: RegisteredType,
        /// The hash computed for the refused contract.
        requested_hash: String,
    },
    /// The contract cannot be registered: a schema that is not an object or a boolean, nested deeper than
    /// [`MAX_SCHEMA_DEPTH`](super::limits::MAX_SCHEMA_DEPTH) or with a NUL character, or a message name beyond the send
    /// limits. Nothing was written. Why.
    InvalidContract(String),
}

/// Result of setting a watch from outside a turn (`PUT /watches`). HTTP: `201`, `200`, `404`, `404`, `422`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchResult {
    /// The watch was stored.
    Created,
    /// The watch was stored already; nothing changed.
    Exists,
    /// No registered type matches the watcher.
    NoSuchType,
    /// The watcher does not exist.
    NoSuchInstance,
    /// The watcher's type may not watch: its message family does not declare `Failed(cicada::Failed)` and
    /// `Stopped(cicada::Stopped)`. Why.
    NotAWatcher(String),
}

/// Result of a send from outside an actor (`POST /actors/{type}/{key}/messages`).
/// HTTP: `202`, `200`, `409`, `404`, `404`, `422`, `422`.
#[derive(Debug, Clone, PartialEq)]
pub enum SendResult {
    /// The message was enqueued, with its store-assigned `seq` and effective `deliver_at`.
    Accepted(Message),
    /// A message with this id was already accepted (within the retention window); nothing was enqueued. The envelope
    /// of that message, when it is still known.
    Duplicate(Option<Message>),
    /// The id was already accepted for a *different* message (another target or type); nothing was enqueued.
    IdConflict(Message),
    /// The target's type is registered, and the target instance does not exist.
    NoSuchInstance,
    /// No registered type matches the target, whether or not an instance of it exists.
    NoSuchType,
    /// The target's type does not accept the message type name.
    UnknownType {
        /// The refused type name.
        message_type: String,
        /// The target's type.
        type_name: TypeName,
    },
    /// The payload does not match the schema of its message type (checked on every node by the check of
    /// [`ActorSystem::send_message_checked`](super::ActorSystem::send_message_checked)), or this node hosts the
    /// target's type and could not decode it.
    InvalidPayload {
        /// The type name.
        message_type: String,
        /// Why: "Payload of 'Add' does not match the registered schema; GET /types/counter has it" on every node, or,
        /// for a payload that matches, "Payload of 'Add' cannot be decoded: …" on a node that hosts the type.
        error: String,
    },
}

/// Snapshot of an instance, as served by `GET /actors/{type}/{key}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorSnapshot {
    /// The instance.
    pub address: ActorAddress,
    /// Its state.
    pub state: Value,
    /// Its version.
    pub version: i64,
    /// Store time of creation.
    pub created_at: Timestamp,
    /// True while the mailbox is paused.
    pub paused: bool,
    /// The actor whose turn created this one as a child (`Turn::create_child`); `None` for an independent actor.
    pub parent: Option<ActorAddress>,
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

/// Result of [`super::ActorSystem::retry_message`]. HTTP: `202`, `404`, `404`, `409`, `409`.
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
    /// The stored payload is nested deeper than [`MAX_DEPTH`](super::limits::MAX_DEPTH): it was written before the
    /// limit (ADR 0029), or it is a `Failed` notification about a payload at the limit. Nothing was enqueued. Why: "The
    /// payload is nested deeper than 64 levels".
    TooDeep(String),
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

/// A turn leased by [`ActorSystem::lease_turns`](super::ActorSystem::lease_turns) for a caller that runs it outside
/// this system (ADR 0037): the claim the store granted and the TTL it was granted for. The caller owns its lease once it
/// has it: it renews it with [`renew_turn`](super::ActorSystem::renew_turn) and gives it back with
/// [`release_turn`](super::ActorSystem::release_turn); dropping a `TurnLease` does nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnLease {
    /// The leased actor.
    pub actor: ActorAddress,
    /// The fencing token of the lease; renew and release name the lease by the actor and this epoch.
    pub epoch: i64,
    /// The head message, with `attempt` already incremented for this lease and its `lost_claims`.
    pub message: Message,
    /// The actor's state.
    pub state: Value,
    /// The actor's version.
    pub version: i64,
    /// Store time at the claim; the handler's `now`.
    pub claimed_at: Timestamp,
    /// Store time at which the lease expires unless renewed. For display and the handler's clock, not for a deadline:
    /// store time is not the caller's clock.
    pub lease_until: Timestamp,
    /// The TTL the serving system asked the store for. The caller's conservative deadline is its monotonic time when it
    /// sent the request plus this TTL, less the smaller of a tenth of it and one second.
    pub ttl: Duration,
}

/// Result of [`ActorSystem::lease_turns`](super::ActorSystem::lease_turns).
#[derive(Debug, Clone, PartialEq)]
pub enum LeaseTurnsResult {
    /// The leased turns, at most the limit asked for. Empty when nothing was due, and also when every claim was filtered
    /// by the claim limit or arrived too late to hand out.
    Leased(Vec<TurnLease>),
    /// A selected type is not registered with the requested hash; nothing was claimed. The first such type in type
    /// name order.
    ContractMismatch {
        /// The type.
        type_name: TypeName,
        /// The registered hash, or `None` when the type is not registered.
        registered: Option<String>,
        /// The hash the caller asked for.
        requested: String,
    },
}

/// Result of [`ActorSystem::renew_turn`](super::ActorSystem::renew_turn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenewTurnResult {
    /// The lease is extended.
    Renewed {
        /// Store time at which the lease now expires: the later of its expiry before and store now plus `ttl`, so it
        /// can be later than this renewal asked for.
        lease_until: Timestamp,
        /// The TTL the serving system asked the store for.
        ttl: Duration,
        /// An operator asked to cancel this turn ([`ActorSystem::cancel_turn`](super::ActorSystem::cancel_turn)). The
        /// lease is extended all the same; the request ends nothing by itself.
        cancel_requested: bool,
    },
    /// The epoch is no longer current or its lease was released, completed or deleted: the turn must be abandoned.
    LeaseLost,
}

/// A failed operation of [`super::ActorSystem`].
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    /// The request is invalid: a field out of bounds, a NUL character, a payload or a state nested deeper than
    /// [`MAX_DEPTH`](super::limits::MAX_DEPTH), a delivery time out of range, a bad cursor. HTTP `400`.
    #[error("{0}")]
    Invalid(String),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The node could not initialize (an invalid hosted type or a contract conflict).
    #[error(transparent)]
    Start(#[from] super::StartError),
}
