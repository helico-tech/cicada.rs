//! The shared durable store: clock, type registry, state, mailboxes and the atomic commit, in one transactional
//! system. A host opens a [`Store`] and reads what it holds through [`ActorSystem`](crate::ActorSystem); the backends
//! are private to this crate (ADR 0018). Every backend passes the S1–S10 conformance suite, test code of this crate.

// Without a backend feature the crate opens no store, and what only the backends use is unused.
#![cfg_attr(not(any(feature = "sqlite", feature = "postgres", feature = "testing")), allow(dead_code))]

mod handle;
mod model;
#[cfg(feature = "postgres")]
pub(crate) mod postgres;
mod results;
#[cfg(feature = "sqlite")]
pub(crate) mod sqlite;
pub(crate) mod support;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;

pub use handle::Store;
pub use model::{
    ActorCreate, ActorRecord, Control, ControlEffect, Delivery, IdError, Message, MessageId, MessageStatus, NodeId,
    OutcomeRecord, OutcomeStatus, Page, PageCursor, RunningTurn, StoreStats, TurnOutcome, TypeDef,
};
pub(crate) use model::{Claim, CommitRequest, NodeDetails, NodeRecord, OutgoingMessage, StateChange};
#[cfg(feature = "postgres")]
pub use postgres::PostgresOptions;
pub use results::{CancelResult, DeleteResult, DeliverNowResult, RemoveTypeResult};
pub(crate) use results::{CommitResult, CreateResult, EnqueueResult, RegisterResult, RenewResult, SetWatchResult};
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteOptions;

use crate::address::{ActorAddress, TypeName};
use crate::change::ChangeHub;
use crate::time::Timestamp;
use crate::watch::{Watch, WatchTarget};

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

/// A store backend: the in-memory store, SQLite and PostgreSQL. Private to this crate, so that a new method or field is
/// not a change of the public API; a host holds a [`Store`]. Methods are grouped as in the architecture: clock, type
/// registry, state store, mailbox and commit. All instants are store time with microsecond precision (S9).
///
/// Implementations must guarantee S1–S10:
///
/// | # | Guarantee |
/// | --- | --- |
/// | S1 Durability | An acknowledged write survives process and machine crash. |
/// | S2 Atomicity | `commit` applies all of its parts or none. `delete`, and a commit that stops, remove the instance, its descendants and their mailboxes together. A commit that ends a message `failed`, a stop and a delete write their notifications with them. |
/// | S3 Per-instance linearizability | All operations on one instance and its mailbox have a single order. |
/// | S4 Lease exclusivity | At most one unexpired lease per instance; every claim issues a greater epoch. |
/// | S5 Fencing | `commit` and `renew` succeed only with the current epoch; `renew` never shortens a held lease. |
/// | S6 Order | `claim_due` returns, per instance, only the pending message with the smallest `(deliver_at, seq)`, when due. |
/// | S7 No loss, no duplicate | A message leaves pending only by commit, cancel or delete; ids are unique. |
/// | S8 Liveness | An instance that is not paused, with a due head message and no valid lease, is eventually returned by `claim_due`. |
/// | S9 One clock | Due-ness and lease expiry use the store clock only. |
/// | S10 Retention | Outcomes are kept for the retention period, which is also the dedup window. |
///
/// **Watches** (ADR 0018, ADR 0023). A watch is a stored pair of a target and a watcher. Three events write
/// notifications into the mailboxes of watchers, in their own transaction: a commit that ends a message `failed` (a
/// [`Failed`](crate::Failed) for each watcher of the actor's address, of a prefix of its type at a dot, or of `*`,
/// unless the message is itself a notification), and a commit that stops or deletes and a delete (a
/// [`Stopped`](crate::Stopped) for each watcher of the address of each removed actor). Each watcher gets one
/// notification per actor; a watcher that does not exist, or that the same event removes, gets nothing, and the
/// watches of a watcher that does not exist are removed. Removing an actor removes the watches it holds as a watcher;
/// the watches on it as a target stay. The notifications of one event are written last, in the order of the actor they
/// are about and then of the watcher, named after the event's id with their number (`<event>.0`, …), and marked as
/// notifications ([`Message::notification`]). One whose id is in use is skipped. A watch changes only under the lock of
/// its watcher's row.
#[async_trait]
pub(crate) trait Backend: Send + Sync + 'static {
    // ---- clock ----

    /// Current store time, truncated to microseconds. Also serves as the store's liveness probe.
    async fn now(&self) -> StoreResult<Timestamp>;

    // ---- type registry ----

    /// Registers `def`. Idempotent for an equal contract hash. A conflict when the name is registered with a different
    /// hash.
    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult>;

    /// All registered types, ordered by name.
    async fn list_types(&self) -> StoreResult<Vec<TypeDef>>;

    /// Removes the registration of `name`, unless instances of that type exist ([`RemoveTypeResult::InUse`]). Checking
    /// and removing are one atomic step, and a concurrent [`Self::create`] (or a commit's create) either lands first
    /// (then this is `InUse`) or finds the type gone and refuses.
    async fn remove_type(&self, name: &TypeName) -> StoreResult<RemoveTypeResult>;

    // ---- state store ----

    /// Creates the instance with initial `state`, version 0, no lease, no parent and an empty mailbox. Idempotent: an
    /// existing instance is left untouched. The type is the address's own and must be registered, checked atomically
    /// with the create: otherwise [`CreateResult::NoSuchType`] and nothing is created.
    async fn create(&self, address: &ActorAddress, state: Value) -> StoreResult<CreateResult>;

    /// Snapshot of the instance, or `None` when it is absent.
    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>>;

    /// Instance URIs that start with `prefix` (a plain string prefix of the canonical URI), in ascending URI order,
    /// at most `limit` per page.
    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>>;

    /// The instances [`Self::list`] pages, with the order, prefix rule, cursors and limit of that listing, each as the
    /// snapshot [`Self::get`] returns. A page is one read of the store (one `SELECT` per read attempt, one lock in
    /// memory), so its rows share one committed snapshot; there is no snapshot across pages.
    async fn list_records(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorRecord>>;

    /// The direct children of `parent` (the instances its commits created as children), in ascending address order,
    /// at most `limit` per page. An empty page when `parent` is absent or has none.
    async fn children(
        &self,
        parent: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorAddress>>;

    /// Admin delete: atomically removes the instance, every descendant and their whole mailboxes; every pending
    /// message, including one being handled, receives outcome `purged`, and [`DeleteResult::Deleted`] counts them over
    /// the whole tree. Any lease is voided, and every epoch issued after a re-create is greater than all earlier ones
    /// for this URI. The walk visits each instance once, so it ends on parent links that form a cycle. The removed
    /// instances' watches as watchers go, and the watchers of each removed instance get a `Stopped` (cause `deleted`
    /// for the instance, `parent` for a descendant, no `by` and no `causedBy`), named after `event`.
    async fn delete(&self, address: &ActorAddress, event: &MessageId) -> StoreResult<DeleteResult>;

    // ---- watches ----

    /// Sets the watch of `watcher` on `target`, under the lock of the watcher's row. Idempotent. Refused when the
    /// watcher does not exist. The store checks neither the target nor the watcher's type.
    async fn watch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<SetWatchResult>;

    /// Removes the watch of `watcher` on `target`, under the lock of the watcher's row; false when it was not stored.
    async fn unwatch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<bool>;

    /// The stored watches in `(target, watcher)` order, at most `limit` per page: of one `watcher`, of exactly one
    /// stored `target` (not every target that would match an actor), of both, or all.
    async fn watches(
        &self,
        watcher: Option<&ActorAddress>,
        target: Option<&WatchTarget>,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Watch>>;

    // ---- mailbox ----

    /// Enqueues a message for an existing instance: `deliver_at` becomes `max(requested, store now)`, `seq` comes from
    /// a store-wide strictly increasing sequence, `attempt` starts at 0. An id that is pending or has a retained
    /// outcome is a duplicate; a message to an absent instance is refused.
    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult>;

    /// Claims up to `limit` instances of `types` that have a due head message and no valid lease, for `node`. Per
    /// claimed instance, atomically: a new, greater epoch; the lease set to `store now + ttl`; the head message's
    /// `attempt` increased by 1. Paused instances are not claimed.
    ///
    /// A claim of an instance that still holds a lease that ran out, which no commit, release, cancel or purge cleared,
    /// replaces a lost claim (ADR 0025): it adds 1 to the [`Message::lost_claims`] of the message of that lease, when
    /// that message is still pending, in the same transaction. That is always the head it claims, which it returns
    /// with the new count. Nothing else changes the count.
    async fn claim_due(
        &self,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>>;

    /// Extends the lease to the later of its expiry and `store now + ttl`, atomically, and returns that expiry: a renewal
    /// never shortens a held lease, so a shorter TTL from a peer cannot end a lease whose holder still follows an
    /// earlier confirmed deadline (ADR 0037). Succeeds only with the current epoch while the lease is held, also after
    /// it ran out when no claim replaced it. Reports [`RenewResult::CancelRequested`] when [`Self::request_cancel`]
    /// recorded this epoch; the lease is renewed either way. It counts no lost claim.
    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult>;

    /// Gives up the lease without an outcome. The message stays pending with its incremented `attempt`, and the claim
    /// is not lost: the lease is cleared, so the next claim counts nothing. A no-op unless `epoch` is current and held.
    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()>;

    /// The earliest store time at which an instance of `types` may become claimable: the minimum of their head's
    /// `deliver_at`, or of their lease expiry when that is later. `None` when nothing is pending. Paused instances do
    /// not count.
    async fn next_due_at(&self, types: &BTreeSet<TypeName>) -> StoreResult<Option<Timestamp>>;

    /// The pending messages of `address` in delivery order `(deliver_at, seq)`, including one being handled. Paging is
    /// a keyset, not a snapshot.
    async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>>;

    /// The pending messages of every instance whose URI starts with `prefix`, ordered by `(to, deliver_at, seq)`.
    async fn list_pending(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize)
    -> StoreResult<Page<Message>>;

    /// Makes a pending message that is not running due at store now, with a new `seq` (a new place in delivery order,
    /// as a reschedule does).
    async fn deliver_now(&self, id: &MessageId) -> StoreResult<DeliverNowResult>;

    /// Admin cancel of a pending message: outcome `cancelled`. Refused for a message being handled under a valid
    /// lease. Cancelling a claimed message whose lease expired also voids that lease.
    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult>;

    /// Pauses or resumes the mailbox of `address`: paused, messages still queue up, but no new turn is claimed; a
    /// running turn finishes. False when the instance is absent.
    async fn set_paused(&self, address: &ActorAddress, paused: bool) -> StoreResult<bool>;

    /// Atomically gives outcome `purged` to every pending message of `address`, except one being handled under a
    /// valid lease; the instance stays. Returns how many were purged, or `None` when the instance is absent. Purging a
    /// message whose lease expired also voids that lease.
    async fn purge(&self, address: &ActorAddress) -> StoreResult<Option<u64>>;

    /// Life-cycle status of `id`: pending, running (valid lease) or finished with a retained, visible outcome; `None`
    /// when the id is unknown, its outcome is hidden by its retention policy, or it is past retention.
    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>>;

    /// Retained, visible outcomes of instances whose URI starts with `prefix` that finished at or after `since`,
    /// ordered by `(finished_at, message_id)`. `status` limits them to one status; `None` lists all.
    async fn outcomes(
        &self,
        status: Option<OutcomeStatus>,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>>;

    /// The messages whose `caused_by` is `id`, one level down: pending (or running) ones and finished ones with a
    /// retained, visible outcome, in ascending message-id order, at most `limit` per page. Paging is a keyset, not a
    /// snapshot. It does not check that `id` itself exists.
    async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<MessageStatus>>;

    /// Counts the instances and the pending messages of instances whose URI starts with `prefix` (empty: all), and
    /// finds the earliest `deliver_at` among those messages that is due at store now. The cost grows with the number
    /// of rows under the prefix. The counts are read one after the other, not as one snapshot.
    async fn stats(&self, prefix: &str) -> StoreResult<StoreStats>;

    /// Removes outcomes past retention and strips hidden records; returns how many outcomes were removed.
    async fn sweep_expired(&self) -> StoreResult<u64>;

    // ---- nodes and running turns (display only; leases stay the only authority) ----

    /// Upserts the row of node `id` with its `hosted` types and its `details`, stamped with store time (`started_at`
    /// only on the first write), and removes the rows of every node last seen more than a day ago. The details replace
    /// all three fields of the row, absent ones included.
    async fn heartbeat_node(&self, id: &NodeId, hosted: &BTreeSet<TypeName>, details: &NodeDetails) -> StoreResult<()>;

    /// Every node row, ordered by id.
    async fn list_nodes(&self) -> StoreResult<Vec<NodeRecord>>;

    /// Removes the row of node `id`. A no-op when it has none.
    async fn remove_node(&self, id: &NodeId) -> StoreResult<()>;

    /// The turns running now: instances under `prefix` (of `node`, or of any node) with a lease valid at store now, in
    /// ascending address order, at most `limit` per page. A snapshot of running work, approximate by nature.
    async fn running_turns(
        &self,
        node: Option<&NodeId>,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<RunningTurn>>;

    /// The turn running now on `address`, or `None` when no lease on it is valid at store now.
    async fn running_turn(&self, address: &ActorAddress) -> StoreResult<Option<RunningTurn>>;

    /// Asks the node that holds the lease on `address` to cancel the running turn. Records the current epoch, only
    /// while a valid lease is held, so a request can never hit a later turn. False when no valid lease is held.
    async fn request_cancel(&self, address: &ActorAddress) -> StoreResult<bool>;

    // ---- commit ----

    /// Ends the turn for the claimed message, fenced by the epoch (S5). Applies all of the following or nothing (S2),
    /// in this order (ADR 0024): the watches of the turn (set and removed in the order of the calls, the committing
    /// instance the watcher), the outcome (or the reschedule), the cancels of this instance's pending messages, the
    /// deletes, the creates (idempotent; a child create gives the new instance the committing instance as its
    /// parent), the purges, the pauses and resumes, the sends (in order), the state change, the release of the lease,
    /// and last the notifications of the event, named after [`CommitRequest::event`]. A stop removes the instance and
    /// every descendant, the children this commit created included, and purges their mailboxes, this commit's sends
    /// to them included. A commit changes no [`Message::lost_claims`]: an outcome keeps the count of its message, and a
    /// reschedule carries it along.
    ///
    /// Each control effect does what the call from outside a turn does ([`Self::set_paused`], [`Self::purge`],
    /// [`Self::delete`]), and nothing to an instance that is absent when it is applied. A purge leaves the message
    /// being handled: of this instance, the turn's own. A delete removes the instance, its descendants and their
    /// mailboxes, every message ending `purged`, and voids their leases, so a turn running on one of them commits
    /// nothing. Deletes come before creates, so a delete and a create of one address restart it; an address that is
    /// absent then is only created, and nothing is removed. Creates come before the other effects, so a new instance
    /// can be paused or purged and sent to in one commit.
    ///
    /// An outcome `failed` writes a `Failed` for the watchers of the message's failure (no `causedBy`), unless the
    /// message is a notification; a rescheduled, done or cancelled message writes none. A stop and a delete write a
    /// `Stopped` for the watchers of each removed instance, with `by` this instance and `causedBy` the turn's message.
    /// The cause does not depend on the order of the removals: `deleted` for an instance the commit names in a delete,
    /// `stopped` for this instance when it stops, and `parent` for every other. A watcher the commit removes gets
    /// nothing, also when the commit creates it again. A notification is not a send of the turn: a watcher that does
    /// not exist never rejects the commit.
    ///
    /// `LeaseLost` when the epoch is not current or the lease is no longer held; `Rejected` when a create names a type
    /// that is not registered, a child create names an instance that exists after the deletes with another parent or
    /// none, a delete names an ancestor of this instance, a send targets an instance that is absent after the deletes
    /// and the creates, or a send id is already in use (the lease is then still held).
    /// [`StoreError::Invalid`] when creates of one address disagree on being a child, the commit deletes this instance,
    /// or it both pauses and resumes one instance (checked first, before the store is read, see
    /// [`support::check_creates`] and [`support::check_controls`]), the message is not the one claimed under the epoch,
    /// the cancels contain the turn's own message, or the data cannot be stored; nothing is applied then either.
    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult>;

    // ---- change hints (ADR 0034) ----

    /// The hub of this store's change hints, shared by its clones; starts the source it needs (PostgreSQL's listener),
    /// so it must be called inside a tokio runtime. Every committed transaction that creates, removes, pauses or
    /// resumes an instance, or writes its state, publishes its addresses there after it committed (or one resync).
    /// [`Self::close`] closes the hub.
    fn changes(&self) -> Arc<ChangeHub>;

    // ---- optional ----

    /// Wake-up signals: a stream that yields whenever a message for an instance of one of `types` may have become
    /// claimable (e.g. PostgreSQL `LISTEN/NOTIFY`). An optimisation only; `None` when the store has none.
    fn wake_ups(&self, _types: &BTreeSet<TypeName>) -> Option<BoxStream<'static, ()>> {
        None
    }

    /// Releases resources such as connections. The store must not be used afterwards.
    ///
    /// The first poll, before any await, makes operations unavailable, closes the hub and stops new connections; a
    /// future dropped unpolled has no effect. Cleanup is resumable: a cancelled call leaves what it did not finish to
    /// the next call, which awaits it like any concurrent caller; no call returns only because the store is already
    /// closed, and no background task finishes it. See [`Store::close`](crate::Store::close) for each backend's limits.
    async fn close(&self) {}
}
