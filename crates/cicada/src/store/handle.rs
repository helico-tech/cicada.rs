//! [`Store`]: the one public store type, a handle on one backend.

use std::collections::BTreeSet;
use std::fmt;
#[cfg(feature = "sqlite")]
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;

#[cfg(feature = "testing")]
use super::Clock;
#[cfg(feature = "postgres")]
use super::PostgresOptions;
#[cfg(feature = "sqlite")]
use super::SqliteOptions;
use super::{
    ActorRecord, Backend, CancelResult, Claim, CommitRequest, CommitResult, CreateResult, DeleteResult,
    DeliverNowResult, EnqueueResult, Message, MessageId, MessageStatus, NodeDetails, NodeId, NodeRecord, OutcomeRecord,
    OutcomeStatus, OutgoingMessage, Page, PageCursor, RegisterResult, RemoveTypeResult, RenewResult, RunningTurn,
    SetWatchResult, StoreResult, StoreStats, TypeDef,
};
use crate::address::{ActorAddress, TypeName};
use crate::change::ChangeHub;
use crate::time::Timestamp;
use crate::watch::{Watch, WatchTarget};

/// The shared durable store of a node: clock, type registry, state, mailboxes and the atomic commit, in one
/// transactional system (PostgreSQL, or SQLite for one host). A host opens one, passes it to
/// [`ActorSystem::builder`](crate::ActorSystem::builder) and closes it when the node has stopped. Cheap to clone: clones
/// share one backend, so several systems can run on one store.
///
/// Each SQL backend is a cargo feature, and none is on by default: name `sqlite` or `postgres` on the `cicada`
/// dependency to get `Store::sqlite` or `Store::postgres`. Tests use the in-memory store, `Store::memory` (feature
/// `testing`, which `cicada-test` turns on).
///
/// ```no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use cicada::store::SqliteOptions;
/// use cicada::{ActorSystem, Store};
///
/// let store = Store::sqlite("cicada.db", SqliteOptions::default()).await?;
/// let system = ActorSystem::builder(store.clone()) /* .host(Counter) */ .build();
/// system.start().await?;
/// // …
/// system.stop().await;
/// store.close().await;
/// # Ok(())
/// # }
/// ```
///
/// A store is not an extension point: the backends are the ones in this crate, and what a backend does is private to
/// it. A host reads what a store holds through [`ActorSystem`](crate::ActorSystem).
///
/// ```compile_fail
/// use cicada::store::CommitRequest; // the request of a commit is private to the core
/// ```
///
/// ```compile_fail
/// use cicada::store::Claim; // and so is a claim
/// ```
///
/// ```compile_fail
/// use cicada::store::Backend; // and the backend trait
/// ```
#[derive(Clone)]
pub struct Store {
    backend: Arc<dyn Backend>,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    /// A store on `backend`.
    pub(crate) fn from_backend(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }

    /// Opens the SQLite store in the file at `path` (feature `sqlite`): one file, WAL, one writer. Sets up a new or
    /// empty file and migrates an older one; refuses a file that holds the tables of another program, and one written
    /// by a newer version. A migration that violates a constraint of the database fails with
    /// [`StoreError::Invalid`](super::StoreError::Invalid), which names the schema version it could not reach; the file
    /// stays at its version.
    ///
    /// ```no_run
    /// # async fn example() -> cicada::store::StoreResult<()> {
    /// use cicada::Store;
    /// use cicada::store::SqliteOptions;
    ///
    /// let store = Store::sqlite("actors.db", SqliteOptions::default()).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "sqlite")]
    pub async fn sqlite(path: impl AsRef<Path>, options: SqliteOptions) -> StoreResult<Self> {
        Ok(Self::from_backend(Arc::new(super::sqlite::SqliteStore::open(path, options).await?)))
    }

    /// Opens the PostgreSQL store on the database at `url` (feature `postgres`), creating or migrating its schema.
    /// `url` is a libpq-style URL as usually found in `DATABASE_URL`
    /// (`postgres://actors:secret@db:5432/actors?sslmode=require`): special characters in the user and password are
    /// percent-encoded, and the query parameters `sslmode`, `sslrootcert`, `application_name` and `options` have an
    /// effect.
    ///
    /// Fails with [`StoreError::Invalid`](super::StoreError::Invalid) for invalid options or a URL that is not a
    /// PostgreSQL URL (the message never repeats the URL, which can hold a password), a database migrated by a newer
    /// version, or a migration that violates a constraint of the database (it names the schema version it could not
    /// reach; the schema stays at its version), and with [`StoreError::Unavailable`](super::StoreError::Unavailable)
    /// when the database cannot be reached.
    ///
    /// ```no_run
    /// # async fn example() -> Result<(), cicada::store::StoreError> {
    /// use cicada::store::PostgresOptions;
    /// use cicada::{ActorSystem, Store};
    ///
    /// let store = Store::postgres(
    ///     "postgres://actors:secret@localhost:5432/actors",
    ///     PostgresOptions::for_schema("orders"),
    /// )
    /// .await?;
    /// let system = ActorSystem::builder(store.clone()).build();
    /// # Ok(()) }
    /// ```
    #[cfg(feature = "postgres")]
    pub async fn postgres(url: &str, options: PostgresOptions) -> StoreResult<Self> {
        Ok(Self::from_backend(Arc::new(super::postgres::PostgresStore::connect(url, options).await?)))
    }

    /// An in-memory store on the system clock with the default retention (feature `testing`). It keeps everything in
    /// the process and offers S2–S10, not S1.
    #[cfg(feature = "testing")]
    pub fn memory() -> Self {
        Self::from_backend(Arc::new(crate::testing::InMemoryStore::default()))
    }

    /// An in-memory store on `clock` with the default retention (feature `testing`), e.g. on a
    /// [`TokioClock`](crate::testing::TokioClock) for virtual time.
    #[cfg(feature = "testing")]
    pub fn memory_on(clock: Arc<dyn Clock>) -> Self {
        Self::from_backend(Arc::new(crate::testing::InMemoryStore::with_clock(clock)))
    }

    /// An in-memory store on `clock` that keeps outcomes for `retention` (feature `testing`).
    #[cfg(feature = "testing")]
    pub fn memory_with(clock: Arc<dyn Clock>, retention: Duration) -> Self {
        Self::from_backend(Arc::new(crate::testing::InMemoryStore::new(clock, retention)))
    }

    /// Releases the store's resources, such as its connections, for every clone. Afterwards every operation fails as
    /// unavailable, and a system on the store answers `503`; the one exception are change streams
    /// ([`ActorSystem::changes`](crate::ActorSystem::changes)), which end normally, without their backlog, and a new
    /// one ends at once. Stopping a system does not close its store, nor its change streams: the host closes the store
    /// it opened, after its systems have stopped.
    ///
    /// The store counts as closed from the first poll of this future: a future that is dropped without being polled
    /// does nothing. It returns once the cleanup is done, which has no time bound: on PostgreSQL it waits for the
    /// connections that operations still borrow, on SQLite for the writer, but not for reads in flight (their
    /// connections are dropped when they end). A caller that stops waiting leaves the store closed and the cleanup
    /// unfinished; nothing goes on with it in the background. The next call resumes it, and concurrent calls all
    /// return once it is done, so a host may bound a close with a timeout and call it again. Connections whose own
    /// close was cancelled are left to the SQL driver, which may finish them later; after the runtime shuts down,
    /// what is left is dropped without a graceful close. The in-memory store closes at once.
    pub async fn close(&self) {
        self.backend.close().await
    }
}

/// A `Store` is itself a backend, so that the runtime and the tests of this crate use it as one.
#[async_trait]
impl Backend for Store {
    async fn now(&self) -> StoreResult<Timestamp> {
        self.backend.now().await
    }

    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult> {
        self.backend.register(def).await
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        self.backend.list_types().await
    }

    async fn remove_type(&self, name: &TypeName) -> StoreResult<RemoveTypeResult> {
        self.backend.remove_type(name).await
    }

    async fn create(&self, address: &ActorAddress, state: Value) -> StoreResult<CreateResult> {
        self.backend.create(address, state).await
    }

    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        self.backend.get(address).await
    }

    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        self.backend.list(prefix, cursor, limit).await
    }

    async fn list_records(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorRecord>> {
        self.backend.list_records(prefix, cursor, limit).await
    }

    async fn children(
        &self,
        parent: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorAddress>> {
        self.backend.children(parent, cursor, limit).await
    }

    async fn delete(&self, address: &ActorAddress, event: &MessageId) -> StoreResult<DeleteResult> {
        self.backend.delete(address, event).await
    }

    async fn watch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<SetWatchResult> {
        self.backend.watch(target, watcher).await
    }

    async fn unwatch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<bool> {
        self.backend.unwatch(target, watcher).await
    }

    async fn watches(
        &self,
        watcher: Option<&ActorAddress>,
        target: Option<&WatchTarget>,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Watch>> {
        self.backend.watches(watcher, target, cursor, limit).await
    }

    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult> {
        self.backend.enqueue(message).await
    }

    async fn claim_due(
        &self,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        self.backend.claim_due(types, node, ttl, limit).await
    }

    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult> {
        self.backend.renew(address, epoch, ttl).await
    }

    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        self.backend.release(address, epoch).await
    }

    async fn next_due_at(&self, types: &BTreeSet<TypeName>) -> StoreResult<Option<Timestamp>> {
        self.backend.next_due_at(types).await
    }

    async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        self.backend.pending(address, cursor, limit).await
    }

    async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        self.backend.list_pending(prefix, cursor, limit).await
    }

    async fn deliver_now(&self, id: &MessageId) -> StoreResult<DeliverNowResult> {
        self.backend.deliver_now(id).await
    }

    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        self.backend.cancel(id).await
    }

    async fn set_paused(&self, address: &ActorAddress, paused: bool) -> StoreResult<bool> {
        self.backend.set_paused(address, paused).await
    }

    async fn purge(&self, address: &ActorAddress) -> StoreResult<Option<u64>> {
        self.backend.purge(address).await
    }

    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        self.backend.status(id).await
    }

    async fn outcomes(
        &self,
        status: Option<OutcomeStatus>,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>> {
        self.backend.outcomes(status, prefix, since, cursor, limit).await
    }

    async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<MessageStatus>> {
        self.backend.caused_by(id, cursor, limit).await
    }

    async fn stats(&self, prefix: &str) -> StoreResult<StoreStats> {
        self.backend.stats(prefix).await
    }

    async fn sweep_expired(&self) -> StoreResult<u64> {
        self.backend.sweep_expired().await
    }

    async fn heartbeat_node(&self, id: &NodeId, hosted: &BTreeSet<TypeName>, details: &NodeDetails) -> StoreResult<()> {
        self.backend.heartbeat_node(id, hosted, details).await
    }

    async fn list_nodes(&self) -> StoreResult<Vec<NodeRecord>> {
        self.backend.list_nodes().await
    }

    async fn remove_node(&self, id: &NodeId) -> StoreResult<()> {
        self.backend.remove_node(id).await
    }

    async fn running_turns(
        &self,
        node: Option<&NodeId>,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<RunningTurn>> {
        self.backend.running_turns(node, prefix, cursor, limit).await
    }

    async fn running_turn(&self, address: &ActorAddress) -> StoreResult<Option<RunningTurn>> {
        self.backend.running_turn(address).await
    }

    async fn request_cancel(&self, address: &ActorAddress) -> StoreResult<bool> {
        self.backend.request_cancel(address).await
    }

    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult> {
        self.backend.commit(commit).await
    }

    fn changes(&self) -> Arc<ChangeHub> {
        self.backend.changes()
    }

    fn wake_ups(&self, types: &BTreeSet<TypeName>) -> Option<BoxStream<'static, ()>> {
        self.backend.wake_ups(types)
    }

    async fn close(&self) {
        self.backend.close().await
    }
}
