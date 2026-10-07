use std::collections::BTreeMap;

use super::model::{ActorRecord, Message, MessageId, OutcomeRecord, TypeDef};
use crate::time::Timestamp;

/// Result of registering a type.
#[derive(Debug, Clone, PartialEq)]
pub enum RegisterResult {
    /// The type is registered (newly, or it already was with the same contract hash).
    Registered(TypeDef),
    /// The registration conflicts with `existing`: the same template with a different contract hash, or a different
    /// template that overlaps it.
    Conflict {
        /// The registered type it conflicts with.
        existing: TypeDef,
        /// Why.
        reason: String,
    },
}

/// The matching type of an address and the address's decoded template parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedType {
    /// The type.
    pub type_def: TypeDef,
    /// The decoded parameters.
    pub params: BTreeMap<String, String>,
}

/// Result of creating an instance in the store.
#[derive(Debug, Clone, PartialEq)]
pub enum CreateResult {
    /// The instance did not exist and was created.
    Created,
    /// The instance already existed and was left untouched.
    Exists(ActorRecord),
}

/// Result of deleting an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteResult {
    /// The instance was removed; that many pending messages received outcome `purged`.
    Deleted {
        /// Number of purged messages.
        purged: usize,
    },
    /// There was no such instance.
    Absent,
}

/// Result of enqueueing a message.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum EnqueueResult {
    /// The message is pending, with its store-assigned `seq` and effective `deliver_at`.
    Enqueued(Message),
    /// A message with this id is pending or has a retained outcome; nothing was enqueued.
    Duplicate(MessageId),
    /// The target instance does not exist; nothing was enqueued.
    NoSuchInstance,
}

/// Result of renewing a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenewResult {
    /// The lease now runs until this store time.
    Renewed(Timestamp),
    /// The epoch is no longer current or the lease was released; the turn must be abandoned.
    LeaseLost,
}

/// Result of an admin cancel.
#[derive(Debug, Clone, PartialEq)]
pub enum CancelResult {
    /// The message was pending and now has outcome `cancelled`.
    Cancelled(OutcomeRecord),
    /// The message is being handled under a valid lease and cannot be cancelled.
    Running,
    /// The message already has a retained outcome.
    AlreadyFinished(OutcomeRecord),
    /// No pending message and no retained outcome has this id.
    NotFound,
}

/// Result of a commit.
#[derive(Debug, Clone, PartialEq)]
pub enum CommitResult {
    /// Everything was applied atomically and the lease released.
    Committed {
        /// The instance's version after the commit (for a stop, the last version before removal).
        version: i64,
        /// The turn's sends as enqueued, with their `seq` and effective `deliver_at`.
        sent: Vec<Message>,
        /// The ids of the commit's cancels that were pending and are now `cancelled`.
        cancelled: Vec<MessageId>,
    },
    /// The epoch is not current or the lease was released. Nothing was applied.
    LeaseLost,
    /// The commit is invalid, e.g. a send targets an absent instance or reuses a message id. Nothing was applied and
    /// the lease is still held, so the caller can commit again with outcome `failed`.
    Rejected(String),
}
