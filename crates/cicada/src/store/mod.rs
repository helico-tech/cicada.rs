//! The shared durable store: clock, type registry, state, mailboxes and the atomic commit, in one transactional
//! system. Every implementation must pass the S1–S10 conformance suite (`cicada-store-conformance`).

mod model;
mod results;
pub mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;

pub use model::{
    ActorCreate, ActorRecord, ChainEntry, Claim, CommitRequest, Delivery, Message, MessageId, MessageStatus, NodeId,
    OutcomeRecord, OutcomeStatus, OutgoingMessage, Page, PageCursor, StateChange, TurnOutcome, TypeDef,
};
pub use results::{
    CancelResult, CommitResult, CreateResult, DeleteResult, EnqueueResult, RegisterResult, RenewResult, ResolvedType,
};

use crate::address::{ActorAddress, UriTemplate};
use crate::time::Timestamp;

/// A failed store operation.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The store cannot serve the request right now (it is closed, unreachable or failing). Callers may retry later
    /// or elsewhere; the HTTP API answers `503`.
    #[error("store unavailable: {0}")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The request is invalid or carries data the store refuses (a delivery time out of range, a value the backend
    /// cannot hold, a commit for a message that is not the claimed one). Retrying cannot help; the HTTP API answers
    /// `400`.
    #[error("{0}")]
    Invalid(String),
    /// Stored data cannot be read (a bug or corruption). The HTTP API answers `500`.
    #[error("corrupt store data: {0}")]
    Corrupt(String),
}

impl StoreError {
    /// An unavailable store, with a message.
    pub fn unavailable(message: impl Into<String>) -> Self {
        StoreError::Unavailable(message.into().into())
    }

    /// True when retrying later may help.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, StoreError::Unavailable(_))
    }
}

/// Result of a store operation.
pub type StoreResult<T> = Result<T, StoreError>;

/// A clock for stores that keep time themselves (SQLite, in-memory); the PostgreSQL store uses the server's clock.
pub trait Clock: Send + Sync + 'static {
    /// The current time.
    fn now(&self) -> Timestamp;
}

/// The system's wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        chrono::Utc::now()
    }
}

/// The shared durable backend. Methods are grouped as in the architecture: clock, type registry, state store,
/// mailbox and commit. All instants are store time with microsecond precision (S9).
///
/// Implementations must guarantee S1–S10:
///
/// | # | Guarantee |
/// | --- | --- |
/// | S1 Durability | An acknowledged write survives process and machine crash. |
/// | S2 Atomicity | `commit` applies all of its parts or none. `delete` removes instance and mailbox together. |
/// | S3 Per-instance linearizability | All operations on one instance and its mailbox have a single order. |
/// | S4 Lease exclusivity | At most one unexpired lease per instance; every claim issues a greater epoch. |
/// | S5 Fencing | `commit` and `renew` succeed only with the current epoch. |
/// | S6 Order | `claim_due` returns, per instance, only the pending message with the smallest `(deliver_at, seq)`, when due. |
/// | S7 No loss, no duplicate | A message leaves pending only by commit, cancel or delete; ids are unique. |
/// | S8 Liveness | An instance with a due head message and no valid lease is eventually returned by `claim_due`. |
/// | S9 One clock | Due-ness and lease expiry use the store clock only. |
/// | S10 Retention | Outcomes are kept for the retention period, which is also the dedup window. |
#[async_trait]
pub trait Store: Send + Sync + 'static {
    /// How long outcomes are retained; also the dedup window for message ids (S10).
    fn retention(&self) -> Duration;

    // ---- clock ----

    /// Current store time, truncated to microseconds. Also serves as the store's liveness probe.
    async fn now(&self) -> StoreResult<Timestamp>;

    // ---- type registry ----

    /// Registers `def`. Idempotent for an equal contract hash. A conflict when the template is registered with a
    /// different hash, or another registered template overlaps it. Concurrent registrations of overlapping templates
    /// admit at most one.
    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult>;

    /// The type registered for exactly `template`.
    async fn type_def(&self, template: &UriTemplate) -> StoreResult<Option<TypeDef>>;

    /// All registered types, ordered by template string.
    async fn list_types(&self) -> StoreResult<Vec<TypeDef>>;

    /// The type whose template matches `address`, with the decoded parameters. Registered templates do not overlap,
    /// so at most one does.
    async fn resolve(&self, address: &ActorAddress) -> StoreResult<Option<ResolvedType>> {
        let types = self.list_types().await?;
        support::resolve(&types, address)
    }

    // ---- state store ----

    /// Creates the instance with initial `state`, version 0, no lease and an empty mailbox. Idempotent: an existing
    /// instance is left untouched. The store does not check that `address` matches `template`.
    async fn create(&self, address: &ActorAddress, template: &UriTemplate, state: Option<Value>)
    -> StoreResult<CreateResult>;

    /// Snapshot of the instance, or `None` when it is absent.
    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>>;

    /// Instance URIs that start with `prefix` (a plain string prefix of the canonical URI), in ascending URI order,
    /// at most `limit` per page.
    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>>;

    /// Admin delete: atomically removes the instance and its whole mailbox; every pending message, including one being
    /// handled, receives outcome `purged`. Any lease is voided, and every epoch issued after a re-create is greater
    /// than all earlier ones for this URI.
    async fn delete(&self, address: &ActorAddress) -> StoreResult<DeleteResult>;

    // ---- mailbox ----

    /// Enqueues a message for an existing instance: `deliver_at` becomes `max(requested, store now)`, `seq` comes from
    /// a store-wide strictly increasing sequence, `attempt` starts at 0. An id that is pending or has a retained
    /// outcome is a duplicate; a message to an absent instance is refused.
    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult>;

    /// Claims up to `limit` instances of `types` that have a due head message and no valid lease, for `node`. Per
    /// claimed instance, atomically: a new, greater epoch; the lease set to `store now + ttl`; the head message's
    /// `attempt` increased by 1.
    async fn claim_due(
        &self,
        types: &BTreeSet<UriTemplate>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>>;

    /// Extends the lease to `store now + ttl`; succeeds only with the current epoch while the lease is held.
    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult>;

    /// Gives up the lease without an outcome. The message stays pending with its incremented `attempt`. A no-op unless
    /// `epoch` is current and held.
    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()>;

    /// The earliest store time at which an instance of `types` may become claimable: the minimum of their head's
    /// `deliver_at`, or of their lease expiry when that is later. `None` when nothing is pending.
    async fn next_due_at(&self, types: &BTreeSet<UriTemplate>) -> StoreResult<Option<Timestamp>>;

    /// The pending messages of `address` in delivery order `(deliver_at, seq)`, including one being handled. Paging is
    /// a keyset, not a snapshot.
    async fn pending(&self, address: &ActorAddress, cursor: Option<&PageCursor>, limit: usize)
    -> StoreResult<Page<Message>>;

    /// The pending messages of every instance whose URI starts with `prefix`, ordered by `(to, deliver_at, seq)`.
    async fn list_pending(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize)
    -> StoreResult<Page<Message>>;

    /// Admin cancel of a pending message: outcome `cancelled`. Refused for a message being handled under a valid
    /// lease. Cancelling a claimed message whose lease expired also voids that lease.
    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult>;

    /// The retained, visible outcome of `id`.
    async fn outcome(&self, id: &MessageId) -> StoreResult<Option<OutcomeRecord>>;

    /// Life-cycle status of `id`: pending, running (valid lease) or finished; `None` when unknown or past retention.
    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>>;

    /// Retained, visible `failed` outcomes of instances whose URI starts with `prefix` that finished at or after
    /// `since`, ordered by `(finished_at, message_id)`.
    async fn failures(
        &self,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>>;

    /// The messages with the given ids, for following message chains: pending ones (as running when their turn holds
    /// a valid lease), retained outcomes, and removed records within the dedup window, in no particular order.
    async fn chain_entries(&self, ids: &[MessageId]) -> StoreResult<Vec<ChainEntry>>;

    /// The messages caused by one of `parents`: pending ones and retained outcomes, at most `limit` of them.
    async fn chain_children(&self, parents: &[MessageId], limit: usize) -> StoreResult<Vec<ChainEntry>>;

    /// Removes outcomes past retention and strips hidden records; returns how many outcomes were removed.
    async fn sweep_expired(&self) -> StoreResult<u64>;

    // ---- commit ----

    /// Ends the turn for the claimed message, fenced by the epoch (S5). Applies all of the following or nothing (S2),
    /// in this order: the outcome (or the reschedule), the cancels of this instance's pending messages, the creates
    /// (idempotent), the sends (in order), the state change, the release of the lease.
    ///
    /// `LeaseLost` when the epoch is not current or the lease is no longer held; `Rejected` when a send targets an
    /// instance that is absent after the creates, or a send id is already in use (the lease is then still held).
    /// [`StoreError::Invalid`] when the message is not the one claimed under the epoch, the cancels contain the turn's
    /// own message, or the data cannot be stored; nothing is applied then either.
    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult>;

    // ---- optional ----

    /// Wake-up signals: a stream that yields whenever a message for an instance of one of `types` may have become
    /// claimable (e.g. PostgreSQL `LISTEN/NOTIFY`). An optimisation only; `None` when the store has none.
    fn wake_ups(&self, _types: &BTreeSet<UriTemplate>) -> Option<BoxStream<'static, ()>> {
        None
    }

    /// Releases resources such as connections. The store must not be used afterwards.
    async fn close(&self) {}
}
