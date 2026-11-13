//! [`SqliteStore`]: the [`Backend`] on one SQLite file.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::address::{ActorAddress, TypeName};
use crate::change::{ChangeHub, SourceState};
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::store::support::{self, Notice, Removal, check_limit};
use crate::store::{
    ActorRecord, Backend, CancelResult, Claim, Clock, CommitRequest, CommitResult, CreateResult, DeleteResult,
    DeliverNowResult, EnqueueResult, Message, MessageId, MessageStatus, NodeId, NodeRecord, OutcomeRecord,
    OutcomeStatus, OutgoingMessage, Page, PageCursor, RegisterResult, RemoveTypeResult, RenewResult, RunningTurn,
    SetWatchResult, StateChange, StoreError, StoreResult, StoreStats, SystemClock, TurnOutcome, TypeDef,
};
use crate::time::{self, DEFAULT_RETENTION, Timestamp};
use crate::watch::{self, FailureCause, Watch, WatchTarget};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::sqlite::{SqliteArguments, SqliteConnectOptions, SqliteRow, SqliteSynchronous};
use sqlx::{AssertSqlSafe, Row, Sqlite, SqliteConnection};
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::store::sqlite::conn::{Conn, Mode, db_error};
use crate::store::sqlite::rows::{self, col, instant, json, micros};
use crate::store::sqlite::schema;

/// Number of read connections of a store.
const READERS: usize = 4;

/// Turns the database file to WAL mode; a property of the file, so it holds for every later connection.
const SET_WAL: &str = "PRAGMA journal_mode = WAL";

/// The options of the SQLite store, for [`Store::sqlite`](crate::Store::sqlite). Every field has a usable default.
#[derive(Clone)]
pub struct SqliteOptions {
    /// The store clock; pass a virtual clock in tests.
    pub clock: Arc<dyn Clock>,
    /// Outcome retention and dedup window (S10).
    pub retention: Duration,
    /// How long a statement waits for another connection's (or process's) lock.
    pub busy_timeout: Duration,
}

impl Default for SqliteOptions {
    fn default() -> Self {
        Self { clock: Arc::new(SystemClock), retention: DEFAULT_RETENTION, busy_timeout: Duration::from_secs(5) }
    }
}

impl std::fmt::Debug for SqliteOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteOptions")
            .field("retention", &self.retention)
            .field("busy_timeout", &self.busy_timeout)
            .finish_non_exhaustive()
    }
}

/// [`Backend`] on one SQLite database file in WAL mode, for single-host deployments.
///
/// Concurrency: all writes of this instance go through one writer connection, serialised by an async mutex, and each
/// write is one explicit `BEGIN IMMEDIATE` … `COMMIT` transaction; that transaction is the atomic commit (S2). Several
/// instances, also in other processes, may open the same file: SQLite admits one writer at a time and the others wait
/// up to the busy timeout. A write that still cannot get the lock fails with [`StoreError::Unavailable`], without side
/// effects, and leaves the connection usable. Reads use a small pool of separate read-only connections, which WAL lets
/// run alongside the writer and which see every committed write (S3).
///
/// Durability (S1): `synchronous = FULL`, so an acknowledged commit survives power loss.
///
/// Time (S9): the clock of [`SqliteOptions`] is the store clock. Instants are stored as microseconds. A wall clock can
/// step backwards (an NTP correction), which would let a new message sort before one already due and make leases and
/// delays unreliable. The store therefore keeps a high-water mark of the time it handed out, persisted in the database
/// (so it also holds across restarts and for every process on the file): store time is `max(clock, mark)` and never
/// goes backwards; after a step back it stands still until the clock catches up.
///
/// Limitation: a wall clock that steps *forwards* moves store time forwards with it. Leases then expire early in store
/// time while the node that holds one still runs its turn by its local deadline: another claimer may take the instance
/// and run the turn as well (the first commit is fenced off by epoch). Delayed messages fall due early by the size of
/// the jump. Run the host with a disciplined clock (NTP slewing).
pub struct SqliteStore {
    clock: Arc<dyn Clock>,
    retention: Duration,
    writer: tokio::sync::Mutex<Option<Conn>>,
    readers: Mutex<Vec<Conn>>,
    reader_permits: Semaphore,
    /// The latest store time this instance handed out; see [`SqliteStore::store_now`].
    high_water: Mutex<Timestamp>,
    closed: AtomicBool,
    /// The change hints of the writes through this store, published after their commits.
    hub: Arc<ChangeHub>,
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore").field("retention", &self.retention).finish_non_exhaustive()
    }
}

fn closed() -> StoreError {
    StoreError::unavailable("Store is closed")
}

fn invalid(e: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(e.to_string())
}

// `message_columns!()`: the columns of `messages` that `rows::message` reads; `outcome_columns!()`: those of `outcomes`
// that `rows::outcome` reads. Statements name their columns and never read with `*`: sqlx keeps the columns of the
// statement it prepared first, and a column added under an open store would crash its worker (ADR 0025).

macro_rules! message_columns {
    () => {
        "id, actor_address, seq, type, payload, sender, correlation_id, headers, deliver_at, attempt, failures, \
         lost_claims, last_error, retry_policy, caused_by, retention_policy, notification"
    };
}

macro_rules! outcome_columns {
    () => {
        "message_id, actor_address, status, error, finished_at, envelope"
    };
}

/// The columns of an actor snapshot, which [`rows::actor_record`] decodes.
macro_rules! actor_columns {
    () => {
        "address, type, state, version, created_at, paused, parent"
    };
}

/// The columns of a running turn; `cancel_requested` is true only for the current epoch.
const RUNNING_TURN: &str = "address, lease_message, lease_owner, lease_until, (cancel_epoch IS NOT NULL AND cancel_epoch = epoch) AS cancel_requested";

/// `(?, ?, …)` for an IN clause with `n` parameters.
fn placeholders(n: usize) -> String {
    format!("({})", vec!["?"; n].join(", "))
}

/// The SQL `substr(column, 1, n) = prefix` compares characters; this is `n`.
fn prefix_len(prefix: &str) -> i64 {
    prefix.chars().count() as i64
}

type Query<'q> = sqlx::query::Query<'q, Sqlite, SqliteArguments>;

/// The pooled connection of a read, returned to the pool when dropped.
struct Reader<'a> {
    store: &'a SqliteStore,
    conn: Option<Conn>,
    _permit: SemaphorePermit<'a>,
}

impl Reader<'_> {
    fn conn(&mut self) -> &mut Conn {
        self.conn.as_mut().expect("a reader holds its connection until dropped")
    }
}

impl Drop for Reader<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take()
            && !self.store.closed.load(Ordering::SeqCst)
        {
            self.store.readers.lock().unwrap_or_else(|e| e.into_inner()).push(conn);
        }
    }
}

/// A transaction on the writer, holding the write lock; `now` is store time read inside it.
struct WriteTx<'a> {
    guard: tokio::sync::MutexGuard<'a, Option<Conn>>,
    now: Timestamp,
}

impl WriteTx<'_> {
    fn conn(&mut self) -> &mut SqliteConnection {
        self.guard.as_mut().expect("checked when the transaction began").raw()
    }

    async fn end<T>(mut self, result: StoreResult<T>) -> StoreResult<T> {
        self.guard.as_mut().expect("checked when the transaction began").end(result).await
    }
}

/// A read transaction on a pooled reader; `now` is store time read inside it.
struct ReadTx<'a> {
    reader: Reader<'a>,
    now: Timestamp,
}

impl ReadTx<'_> {
    fn conn(&mut self) -> &mut SqliteConnection {
        self.reader.conn().raw()
    }

    async fn end<T>(mut self, result: StoreResult<T>) -> StoreResult<T> {
        self.reader.conn().end(result).await
    }
}

impl SqliteStore {
    /// Opens (or creates, with its schema) the database file at `path` and migrates it to the current schema version.
    /// A new or empty file is set up. Fails with [`StoreError::Invalid`] when the file was written by a newer schema
    /// version, holds the tables of another program (it has tables but no schema version; the file is left as it
    /// was), or holds data that a migration's constraint refuses (the file stays at its version), and with
    /// [`StoreError::Unavailable`] when the file cannot be opened or is locked.
    pub async fn open(path: impl AsRef<Path>, options: SqliteOptions) -> StoreResult<Self> {
        let path = path.as_ref();
        let base = SqliteConnectOptions::new()
            .filename(path)
            .busy_timeout(options.busy_timeout)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true);
        let mut writer = Conn::connect(&base.clone().create_if_missing(true)).await?;
        // WAL mode is set after the migration, so that a database the migration refuses stays as it was.
        let setup = match schema::migrate(&mut writer, path).await {
            Ok(()) => sqlx::raw_sql(SET_WAL).execute(writer.raw()).await.map(drop).map_err(db_error),
            Err(e) => Err(e),
        };
        if let Err(e) = setup {
            writer.close().await;
            return Err(e);
        }
        // WAL is a property of the file, set by the writer; readers only read.
        let reader_options = base.read_only(true);
        let mut readers = Vec::with_capacity(READERS);
        for _ in 0..READERS {
            match Conn::connect(&reader_options).await {
                Ok(conn) => readers.push(conn),
                Err(e) => {
                    for conn in readers {
                        conn.close().await;
                    }
                    writer.close().await;
                    return Err(e);
                }
            }
        }
        Ok(Self {
            high_water: Mutex::new(time::truncate(options.clock.now())),
            clock: options.clock,
            retention: options.retention,
            writer: tokio::sync::Mutex::new(Some(writer)),
            reader_permits: Semaphore::new(readers.len()),
            readers: Mutex::new(readers),
            closed: AtomicBool::new(false),
            hub: Arc::new(ChangeHub::new(SourceState::Up)),
        })
    }

    fn check_open(&self) -> StoreResult<()> {
        if self.closed.load(Ordering::SeqCst) { Err(closed()) } else { Ok(()) }
    }

    /// Store time inside a transaction on `c`: `max(clock, the persisted mark, the latest time this instance handed
    /// out)`. A write transaction (`persist`) also moves the persisted mark forward.
    async fn store_now(&self, c: &mut SqliteConnection, persist: bool) -> StoreResult<Timestamp> {
        let mark: i64 = sqlx::query_scalar("SELECT value FROM sequences WHERE name = 'clock'")
            .fetch_optional(&mut *c)
            .await
            .map_err(db_error)?
            .ok_or_else(|| StoreError::Corrupt("the sequence 'clock' is missing".into()))?;
        let now = {
            let mut high_water = self.high_water.lock().unwrap_or_else(|e| e.into_inner());
            *high_water = (*high_water).max(time::truncate(self.clock.now())).max(instant(mark));
            *high_water
        };
        if persist && micros(now) > mark {
            sqlx::query("UPDATE sequences SET value = ? WHERE name = 'clock'")
                .bind(micros(now))
                .execute(&mut *c)
                .await
                .map_err(db_error)?;
        }
        Ok(now)
    }

    /// Starts a write transaction on the writer connection.
    async fn write(&self) -> StoreResult<WriteTx<'_>> {
        self.check_open()?;
        let mut guard = self.writer.lock().await;
        let conn = guard.as_mut().ok_or_else(closed)?;
        conn.begin(Mode::Write).await.map_err(db_error)?;
        match self.store_now(conn.raw(), true).await {
            Ok(now) => Ok(WriteTx { guard, now }),
            Err(e) => {
                conn.rollback().await;
                Err(e)
            }
        }
    }

    /// Starts a read transaction on a pooled reader connection.
    async fn read(&self) -> StoreResult<ReadTx<'_>> {
        self.check_open()?;
        let permit = self.reader_permits.acquire().await.map_err(|_| closed())?;
        let conn = self.readers.lock().unwrap_or_else(|e| e.into_inner()).pop().ok_or_else(closed)?;
        let mut reader = Reader { store: self, conn: Some(conn), _permit: permit };
        reader.conn().begin(Mode::Read).await.map_err(db_error)?;
        match self.store_now(reader.conn().raw(), false).await {
            Ok(now) => Ok(ReadTx { reader, now }),
            Err(e) => {
                reader.conn().rollback().await;
                Err(e)
            }
        }
    }

    fn cutoff(&self, now: Timestamp) -> i64 {
        micros(time::sub(now, self.retention))
    }

    // ---- registry ----

    async fn register_in(c: &mut SqliteConnection, def: TypeDef) -> StoreResult<RegisterResult> {
        let existing = fetch_optional(
            c,
            sqlx::query("SELECT name, contract, contract_hash FROM actor_types WHERE name = ?").bind(def.name.as_str()),
        )
        .await?
        .map(|row| rows::type_def(&row))
        .transpose()?;
        if let Some(existing) = existing {
            return Ok(support::register_again(existing, &def));
        }
        execute(
            c,
            sqlx::query("INSERT INTO actor_types (name, contract, contract_hash) VALUES (?, ?, ?)")
                .bind(def.name.as_str())
                .bind(json(&def.contract))
                .bind(def.contract_hash.as_str()),
        )
        .await?;
        Ok(RegisterResult::Registered(def))
    }

    // ---- state store ----

    async fn create_in(
        c: &mut SqliteConnection,
        address: &ActorAddress,
        state: &Value,
        parent: Option<&ActorAddress>,
        now: Timestamp,
    ) -> StoreResult<CreateResult> {
        if !Self::type_registered(c, address.type_name()).await? {
            return Ok(CreateResult::NoSuchType);
        }
        let inserted = execute(
            c,
            sqlx::query(
                "INSERT INTO actors (address, type, state, parent, created_at) VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT (address) DO NOTHING",
            )
            .bind(address.as_str())
            .bind(address.type_name().as_str())
            .bind(json(state))
            .bind(parent.map(ActorAddress::as_str))
            .bind(micros(now)),
        )
        .await?;
        if inserted == 1 {
            return Ok(CreateResult::Created);
        }
        let existing =
            Self::get_in(c, address).await?.ok_or_else(|| StoreError::Corrupt(format!("{address} vanished")))?;
        Ok(CreateResult::Exists(existing))
    }

    async fn type_registered(c: &mut SqliteConnection, name: &TypeName) -> StoreResult<bool> {
        Ok(fetch_optional(c, sqlx::query("SELECT 1 FROM actor_types WHERE name = ?").bind(name.as_str()))
            .await?
            .is_some())
    }

    async fn get_in(c: &mut SqliteConnection, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        fetch_optional(
            c,
            sqlx::query(concat!("SELECT ", actor_columns!(), " FROM actors WHERE address = ?")).bind(address.as_str()),
        )
        .await?
        .map(|row| rows::actor_record(&row))
        .transpose()
    }

    /// The `columns` of at most `limit + 1` instances whose URIs start with `prefix` (compared literally), in URI
    /// order, after `cursor` when it lies at or past the prefix, each decoded by `decode`: one `SELECT`.
    async fn prefix_page_in<T>(
        c: &mut SqliteConnection,
        columns: &'static str,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
        decode: impl Fn(&SqliteRow) -> StoreResult<T>,
    ) -> StoreResult<Vec<T>> {
        let (lower_bound, op) = match cursor {
            Some(c) if c.0.as_str() >= prefix => (c.0.as_str(), ">"),
            _ => (prefix, ">="),
        };
        let sql = format!(
            "SELECT {columns} FROM actors WHERE address {op} ? AND substr(address, 1, ?) = ? ORDER BY address LIMIT ?"
        );
        let query = sqlx::query(AssertSqlSafe(sql))
            .bind(lower_bound)
            .bind(prefix_len(prefix))
            .bind(prefix)
            .bind(limit as i64 + 1);
        fetch_all(c, query, decode).await
    }

    async fn actor_exists(c: &mut SqliteConnection, addr: &str) -> StoreResult<bool> {
        Ok(fetch_optional(c, sqlx::query("SELECT 1 FROM actors WHERE address = ?").bind(addr)).await?.is_some())
    }

    /// The parent of the instance at `addr`: `None` when it is absent, `Some(None)` when it is independent.
    async fn parent_of(c: &mut SqliteConnection, addr: &str) -> StoreResult<Option<Option<String>>> {
        fetch_optional(c, sqlx::query("SELECT parent FROM actors WHERE address = ?").bind(addr))
            .await?
            .map(|row| col::<Option<String>>(&row, "parent"))
            .transpose()
    }

    /// `root` and every descendant, in address order. `UNION` leaves out the rows found already, so the walk ends on
    /// parent links that form a cycle; the single writer makes it a consistent snapshot.
    async fn tree(c: &mut SqliteConnection, root: &ActorAddress) -> StoreResult<Vec<ActorAddress>> {
        fetch_all(
            c,
            sqlx::query(
                "WITH RECURSIVE tree (address) AS (
                     SELECT ?
                     UNION
                     SELECT actors.address FROM actors JOIN tree ON actors.parent = tree.address
                 )
                 SELECT address FROM tree ORDER BY address",
            )
            .bind(root.as_str()),
            |row| rows::address(&col::<String>(row, "address")?),
        )
        .await
    }

    /// Removes every instance of `tree` with its mailbox and the watches it holds as a watcher; returns how many
    /// messages were purged.
    async fn remove_all(&self, c: &mut SqliteConnection, tree: &[ActorAddress], now: Timestamp) -> StoreResult<usize> {
        let mut purged = 0;
        for address in tree {
            purged += self.purge_mailbox(c, address, now).await?;
            execute(c, sqlx::query("DELETE FROM actors WHERE address = ?").bind(address.as_str())).await?;
            execute(c, sqlx::query("DELETE FROM watches WHERE watcher = ?").bind(address.as_str())).await?;
        }
        Ok(purged)
    }

    /// The notices of an event: the `Failed` of `failed` (a message that ends `failed`, with its error) for the
    /// watchers of its failure, and a `Stopped` of each actor of `removed` for the watchers of its address. A watcher
    /// the event removes gets nothing; one that does not exist gets nothing and loses its watches.
    async fn notices(
        c: &mut SqliteConnection,
        failed: Option<(&Message, &str, FailureCause)>,
        removed: Option<(&Removal<'_>, &[ActorAddress])>,
    ) -> StoreResult<Vec<Notice>> {
        let watcher = |row: &SqliteRow| rows::address(&col::<String>(row, "watcher")?);
        let mut notices = Vec::new();
        if let Some((message, error, cause)) = failed {
            let targets = watch::failure_targets(&message.to);
            let sql = format!(
                "SELECT DISTINCT watcher FROM watches WHERE target IN {} ORDER BY watcher",
                placeholders(targets.len())
            );
            let mut query = sqlx::query(AssertSqlSafe(sql));
            for target in &targets {
                query = query.bind(target.as_str());
            }
            for w in fetch_all(c, query, watcher).await? {
                notices.push(Notice::failed(message, error, cause, &w));
            }
        }
        if let Some((removal, tree)) = removed {
            for actor in tree {
                let query =
                    sqlx::query("SELECT watcher FROM watches WHERE target = ? ORDER BY watcher").bind(actor.as_str());
                for w in fetch_all(c, query, watcher).await? {
                    notices.push(removal.notice(actor, &w));
                }
            }
        }
        let removed = removed.map_or(&[][..], |(_, tree)| tree);
        let mut kept = Vec::with_capacity(notices.len());
        let mut gone = HashSet::new();
        for notice in notices {
            if removed.contains(&notice.watcher) || gone.contains(&notice.watcher) {
                continue;
            }
            if !Self::actor_exists(c, notice.watcher.as_str()).await? {
                execute(c, sqlx::query("DELETE FROM watches WHERE watcher = ?").bind(notice.watcher.as_str())).await?;
                gone.insert(notice.watcher);
                continue;
            }
            kept.push(notice);
        }
        Ok(kept)
    }

    /// Writes the notifications of `notices`, named after `event`, into the watchers' mailboxes; one whose id is in use
    /// is skipped.
    async fn notify(
        &self,
        c: &mut SqliteConnection,
        event: &MessageId,
        notices: Vec<Notice>,
        now: Timestamp,
    ) -> StoreResult<()> {
        for message in support::notifications(event, notices) {
            if self.is_id_in_use(c, &message.id, now).await? {
                tracing::warn!("Skipping the notification {} for {}: its id is in use", message.id, message.to);
                continue;
            }
            self.insert(c, message, now, true).await?;
        }
        Ok(())
    }

    async fn purge_mailbox(
        &self,
        c: &mut SqliteConnection,
        address: &ActorAddress,
        now: Timestamp,
    ) -> StoreResult<usize> {
        let pending = fetch_all(
            c,
            sqlx::query(concat!(
                "SELECT ",
                message_columns!(),
                " FROM messages WHERE actor_address = ? ORDER BY deliver_at, seq"
            ))
            .bind(address.as_str()),
            rows::message,
        )
        .await?;
        let purged = pending.len();
        for message in pending {
            self.finish(c, message, OutcomeStatus::Purged, None, now).await?;
        }
        Ok(purged)
    }

    /// Gives `purged` to every pending message of `address` except the one being handled: `own`, the message of the
    /// committing turn when its commit purges its own instance, or else the message of a valid lease. Purging a message
    /// whose lease expired also voids that lease, so that the stale turn cannot commit. `None` when it is absent.
    async fn purge_in(
        &self,
        c: &mut SqliteConnection,
        address: &ActorAddress,
        own: Option<&MessageId>,
        now: Timestamp,
    ) -> StoreResult<Option<u64>> {
        let Some(row) = fetch_optional(
            c,
            sqlx::query("SELECT lease_owner, lease_until, lease_message FROM actors WHERE address = ?")
                .bind(address.as_str()),
        )
        .await?
        else {
            return Ok(None);
        };
        let valid = col::<Option<String>>(&row, "lease_owner")?.is_some()
            && col::<Option<i64>>(&row, "lease_until")?.is_some_and(|until| instant(until) > now);
        let held = match own {
            Some(id) => Some(id.as_str().to_owned()),
            None if valid => col::<Option<String>>(&row, "lease_message")?,
            None => None,
        };
        let pending = fetch_all(
            c,
            sqlx::query(concat!(
                "SELECT ",
                message_columns!(),
                " FROM messages WHERE actor_address = ? ORDER BY deliver_at, seq"
            ))
            .bind(address.as_str()),
            rows::message,
        )
        .await?;
        let mut purged = 0;
        for message in pending.into_iter().filter(|m| held.as_deref() != Some(m.id.as_str())) {
            self.finish(c, message, OutcomeStatus::Purged, None, now).await?;
            purged += 1;
        }
        if held.is_none() {
            // Void an expired lease whose message is gone, so that the stale turn cannot commit.
            execute(
                c,
                sqlx::query(
                    "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE address = ?",
                )
                .bind(address.as_str()),
            )
            .await?;
        }
        Self::refresh_next_due(c, address.as_str()).await?;
        Ok(Some(purged))
    }

    // ---- mailbox ----

    async fn is_id_in_use(&self, c: &mut SqliteConnection, id: &MessageId, now: Timestamp) -> StoreResult<bool> {
        if fetch_optional(c, sqlx::query("SELECT 1 FROM messages WHERE id = ?").bind(id.as_str())).await?.is_some() {
            return Ok(true);
        }
        let retained = fetch_optional(
            c,
            sqlx::query(r"SELECT 1 FROM outcomes WHERE message_id = ? AND (finished_at > ? OR visible_until > ?)")
                .bind(id.as_str())
                .bind(self.cutoff(now))
                .bind(micros(now)),
        )
        .await?;
        Ok(retained.is_some())
    }

    async fn insert(
        &self,
        c: &mut SqliteConnection,
        message: OutgoingMessage,
        now: Timestamp,
        notification: bool,
    ) -> StoreResult<Message> {
        let deliver_at = message.delivery.resolve(now).map_err(invalid)?;
        // An expired, not yet swept outcome with the same id.
        execute(c, sqlx::query("DELETE FROM outcomes WHERE message_id = ?").bind(message.id.as_str())).await?;
        let stored = message.into_message(deliver_at, next_value(c, "seq").await?, notification);
        execute(
            c,
            sqlx::query(
                "INSERT INTO messages
                    (id, actor_address, seq, type, payload, sender, correlation_id, headers, deliver_at, attempt, retry_policy,
                     caused_by, retention_policy, notification)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?)",
            )
            .bind(stored.id.as_str())
            .bind(stored.to.as_str())
            .bind(stored.seq)
            .bind(stored.message_type.as_str())
            .bind(json(&stored.payload))
            .bind(stored.from.as_ref().map(ActorAddress::as_str))
            .bind(stored.correlation_id.as_deref())
            .bind(json(&support::headers_to_json(&stored.headers)))
            .bind(micros(stored.deliver_at))
            .bind(stored.retry.as_ref().map(|r| json(&RetryPolicy::to_json(r))))
            .bind(stored.caused_by.as_ref().map(MessageId::as_str))
            .bind(stored.retention.as_ref().map(|r| json(&Retention::to_json(r))))
            .bind(i64::from(stored.notification)),
        )
        .await?;
        execute(
            c,
            sqlx::query("UPDATE actors SET next_due_at = min(coalesce(next_due_at, ?), ?) WHERE address = ?")
                .bind(micros(stored.deliver_at))
                .bind(micros(stored.deliver_at))
                .bind(stored.to.as_str()),
        )
        .await?;
        Ok(stored)
    }

    async fn finish(
        &self,
        c: &mut SqliteConnection,
        message: Message,
        status: OutcomeStatus,
        error: Option<String>,
        now: Timestamp,
    ) -> StoreResult<OutcomeRecord> {
        execute(c, sqlx::query("DELETE FROM messages WHERE id = ?").bind(message.id.as_str())).await?;
        let removed = support::removed_at_once(&message, status);
        let keep_last = support::keep_last(&message, status);
        let keep_seq = match keep_last {
            Some(_) => Some(next_value(c, "seq").await?),
            None => None,
        };
        let envelope = if removed { support::stripped(&message) } else { message.clone() };
        execute(
            c,
            sqlx::query(
                "INSERT INTO outcomes
                    (message_id, actor_address, status, error, finished_at, envelope, caused_by, visible_until, keep_last,
                     keep_seq, stripped)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (message_id) DO UPDATE SET actor_address = excluded.actor_address, status = excluded.status,
                    error = excluded.error, finished_at = excluded.finished_at, envelope = excluded.envelope,
                    caused_by = excluded.caused_by,
                    visible_until = excluded.visible_until, keep_last = excluded.keep_last,
                    keep_seq = excluded.keep_seq, stripped = excluded.stripped",
            )
            .bind(message.id.as_str())
            .bind(message.to.as_str())
            .bind(status.wire_name())
            .bind(if removed { None } else { error.as_deref() })
            .bind(micros(now))
            .bind(json(&support::envelope_to_json(&envelope)))
            .bind(message.caused_by.as_ref().map(MessageId::as_str))
            .bind(support::visible_until(&message, status, now).map(micros))
            .bind(i64::from(keep_last.is_some()))
            .bind(keep_seq)
            .bind(i64::from(removed)),
        )
        .await?;
        if let Some(count) = keep_last {
            // Hide this instance's visible keep-last outcomes of this status beyond the newest `count`.
            execute(
                c,
                sqlx::query(
                    "UPDATE outcomes SET visible_until = ?
                     WHERE message_id IN (
                         SELECT message_id FROM outcomes
                         WHERE actor_address = ? AND status = ? AND keep_last = 1
                           AND (CASE WHEN visible_until IS NULL THEN finished_at > ? ELSE visible_until > ? END)
                         ORDER BY keep_seq DESC LIMIT -1 OFFSET ?
                     )",
                )
                .bind(micros(now))
                .bind(message.to.as_str())
                .bind(status.wire_name())
                .bind(self.cutoff(now))
                .bind(micros(now))
                .bind(i64::from(count)),
            )
            .await?;
        }
        Ok(OutcomeRecord {
            message_id: message.id.clone(),
            actor: message.to.clone(),
            status,
            error,
            finished_at: now,
            envelope: message,
        })
    }

    async fn refresh_next_due(c: &mut SqliteConnection, addr: &str) -> StoreResult<()> {
        execute(
            c,
            sqlx::query("UPDATE actors SET next_due_at = (SELECT min(deliver_at) FROM messages WHERE actor_address = ?) WHERE address = ?")
                .bind(addr)
                .bind(addr),
        )
        .await?;
        Ok(())
    }

    async fn claim_due_in(
        c: &mut SqliteConnection,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
        now: Timestamp,
    ) -> StoreResult<Vec<Claim>> {
        let n = micros(now);
        let sql = format!(
            "SELECT address, type, state, version, CASE WHEN lease_owner IS NULL THEN NULL ELSE lease_message END AS lost
             FROM actors
             WHERE type IN {} AND next_due_at <= ? AND paused = 0
               AND (lease_owner IS NULL OR lease_until <= ?)
             ORDER BY next_due_at LIMIT ?",
            placeholders(types.len())
        );
        let mut query = sqlx::query(AssertSqlSafe(sql));
        for t in types {
            query = query.bind(t.as_str());
        }
        let candidates = fetch_all(c, query.bind(n).bind(n).bind(limit as i64), |row| {
            Ok((
                col::<String>(row, "address")?,
                col::<String>(row, "state")?,
                col::<i64>(row, "version")?,
                col::<Option<String>>(row, "lost")?,
            ))
        })
        .await?;
        let lease_until = time::truncate(time::add(now, ttl).map_err(invalid)?);
        let mut claims = Vec::with_capacity(candidates.len());
        for (addr, state, version, lost) in candidates {
            // A lease that ran out and that nothing cleared: its claim is lost (ADR 0025). The table holds only pending
            // messages, so a message that is gone counts nothing.
            if let Some(lost) = lost {
                execute(c, sqlx::query("UPDATE messages SET lost_claims = lost_claims + 1 WHERE id = ?").bind(lost))
                    .await?;
            }
            let head = fetch_optional(
                c,
                sqlx::query(concat!(
                    "SELECT ",
                    message_columns!(),
                    " FROM messages WHERE actor_address = ? ORDER BY deliver_at, seq LIMIT 1"
                ))
                .bind(addr.as_str()),
            )
            .await?
            .map(|row| rows::message(&row))
            .transpose()?
            .filter(|head| head.deliver_at <= now)
            .ok_or_else(|| StoreError::Corrupt(format!("next_due_at out of sync for {addr}")))?;
            let claimed = Message { attempt: head.attempt + 1, ..head };
            let epoch = next_value(c, "epoch").await?;
            execute(
                c,
                sqlx::query("UPDATE messages SET attempt = ? WHERE id = ?")
                    .bind(i64::from(claimed.attempt))
                    .bind(claimed.id.as_str()),
            )
            .await?;
            execute(
                c,
                sqlx::query(
                    "UPDATE actors SET epoch = ?, lease_owner = ?, lease_until = ?, lease_message = ? WHERE address = ?",
                )
                .bind(epoch)
                .bind(node.as_str())
                .bind(micros(lease_until))
                .bind(claimed.id.as_str())
                .bind(addr.as_str()),
            )
            .await?;
            claims.push(Claim {
                actor: rows::address(&addr)?,
                epoch,
                message: claimed,
                state: rows::parse_json(&state, "state")?,
                version,
                claimed_at: now,
                lease_until,
            });
        }
        Ok(claims)
    }

    /// The message claimed under `epoch` while the lease is held: `Some(lease_message)`, else `None`.
    async fn holds_lease(c: &mut SqliteConnection, address: &ActorAddress, epoch: i64) -> StoreResult<Option<String>> {
        let row = fetch_optional(
            c,
            sqlx::query("SELECT lease_message FROM actors WHERE address = ? AND epoch = ? AND lease_owner IS NOT NULL")
                .bind(address.as_str())
                .bind(epoch),
        )
        .await?;
        Ok(match row {
            Some(row) => col::<Option<String>>(&row, "lease_message")?,
            None => None,
        })
    }

    async fn live_outcome(
        &self,
        c: &mut SqliteConnection,
        id: &MessageId,
        now: Timestamp,
    ) -> StoreResult<Option<OutcomeRecord>> {
        fetch_optional(
            c,
            sqlx::query(concat!(
                "SELECT ",
                outcome_columns!(),
                " FROM outcomes WHERE message_id = ?
                   AND (CASE WHEN visible_until IS NULL THEN finished_at > ? ELSE visible_until > ? END)"
            ))
            .bind(id.as_str())
            .bind(self.cutoff(now))
            .bind(micros(now)),
        )
        .await?
        .map(|row| rows::outcome(&row))
        .transpose()
    }

    async fn message_in(c: &mut SqliteConnection, id: &MessageId) -> StoreResult<Option<Message>> {
        fetch_optional(
            c,
            sqlx::query(concat!("SELECT ", message_columns!(), " FROM messages WHERE id = ?")).bind(id.as_str()),
        )
        .await?
        .map(|row| rows::message(&row))
        .transpose()
    }

    async fn status_in(
        &self,
        c: &mut SqliteConnection,
        id: &MessageId,
        now: Timestamp,
    ) -> StoreResult<Option<MessageStatus>> {
        let Some(message) = Self::message_in(c, id).await? else {
            return Ok(self.live_outcome(c, id, now).await?.map(MessageStatus::Finished));
        };
        let running = fetch_optional(
            c,
            sqlx::query(
                "SELECT lease_owner, lease_until FROM actors
                 WHERE address = ? AND lease_message = ? AND lease_owner IS NOT NULL AND lease_until > ?",
            )
            .bind(message.to.as_str())
            .bind(id.as_str())
            .bind(micros(now)),
        )
        .await?;
        Ok(Some(match running {
            Some(row) => MessageStatus::Running {
                node: NodeId::new(col::<String>(&row, "lease_owner")?)
                    .map_err(|e| StoreError::Corrupt(e.to_string()))?,
                lease_until: instant(col(&row, "lease_until")?),
                message,
            },
            None => MessageStatus::Pending(message),
        }))
    }

    async fn cancel_in(&self, c: &mut SqliteConnection, id: &MessageId, now: Timestamp) -> StoreResult<CancelResult> {
        let Some(message) = Self::message_in(c, id).await? else {
            return Ok(match self.live_outcome(c, id, now).await? {
                Some(o) => CancelResult::AlreadyFinished(o),
                None => CancelResult::NotFound,
            });
        };
        let lease = fetch_optional(
            c,
            sqlx::query(
                "SELECT lease_until FROM actors WHERE address = ? AND lease_message = ? AND lease_owner IS NOT NULL",
            )
            .bind(message.to.as_str())
            .bind(id.as_str()),
        )
        .await?;
        if let Some(row) = lease {
            if col::<Option<i64>>(&row, "lease_until")?.is_some_and(|until| instant(until) > now) {
                return Ok(CancelResult::Running);
            }
            // Void the expired lease so that the stale turn cannot commit.
            execute(
                c,
                sqlx::query(
                    "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE address = ?",
                )
                .bind(message.to.as_str()),
            )
            .await?;
        }
        let addr = message.to.as_str().to_owned();
        let outcome = self.finish(c, message, OutcomeStatus::Cancelled, None, now).await?;
        Self::refresh_next_due(c, &addr).await?;
        Ok(CancelResult::Cancelled(outcome))
    }

    async fn commit_in(
        &self,
        c: &mut SqliteConnection,
        commit: CommitRequest,
        now: Timestamp,
    ) -> StoreResult<CommitResult> {
        let targets: Vec<ActorAddress> = commit.deletes().cloned().collect();
        let CommitRequest {
            event,
            address,
            epoch,
            message_id,
            outcome,
            cause,
            state,
            creates,
            sends,
            cancels,
            watches,
            controls,
        } = commit;
        let Some(claimed) = Self::holds_lease(c, &address, epoch).await? else {
            return Ok(CommitResult::LeaseLost);
        };
        if claimed != message_id.as_str() {
            return Err(StoreError::Invalid(format!(
                "Message {message_id} is not the message claimed under epoch {epoch}"
            )));
        }
        if cancels.contains(&message_id) {
            return Err(StoreError::Invalid(format!("A turn cannot cancel its own message {message_id}")));
        }

        // Validate everything before writing anything (S2); a rejected commit leaves the transaction empty.
        for create in &creates {
            if !Self::type_registered(c, create.address.type_name()).await? {
                return Ok(CommitResult::Rejected(format!("Create of {}: its type is not registered", create.address)));
            }
        }
        // The trees of the deletes, read before anything is written: deletes come before the creates and the sends.
        let mut deleted = BTreeSet::new();
        for target in &targets {
            if deleted.contains(target) || !Self::actor_exists(c, target.as_str()).await? {
                continue;
            }
            let tree = Self::tree(c, target).await?;
            if tree.contains(&address) {
                return Ok(CommitResult::Rejected(support::ancestor_delete_reason(target, &address)));
            }
            deleted.extend(tree);
        }
        for create in creates.iter().filter(|c| c.child && !deleted.contains(&c.address)) {
            if let Some(parent) = Self::parent_of(c, create.address.as_str()).await?
                && parent.as_deref() != Some(address.as_str())
            {
                return Ok(CommitResult::Rejected(support::foreign_child_reason(&create.address, &address)));
            }
        }
        let created: HashSet<&ActorAddress> = creates.iter().map(|c| &c.address).collect();
        let mut send_ids = HashSet::new();
        for send in &sends {
            let exists = created.contains(&send.to)
                || (!deleted.contains(&send.to) && Self::actor_exists(c, send.to.as_str()).await?);
            if !exists {
                return Ok(CommitResult::Rejected(format!("Send to absent instance {}", send.to)));
            }
            if !send_ids.insert(&send.id) || send.id == message_id || self.is_id_in_use(c, &send.id, now).await? {
                return Ok(CommitResult::Rejected(format!("Message id {} is already in use", send.id)));
            }
        }

        // 0. the turn's watches, in the order of its calls
        for change in &watches {
            let sql = if change.set {
                "INSERT INTO watches (target, watcher) VALUES (?, ?) ON CONFLICT DO NOTHING"
            } else {
                "DELETE FROM watches WHERE target = ? AND watcher = ?"
            };
            execute(c, sqlx::query(sql).bind(change.target.as_str()).bind(address.as_str())).await?;
        }
        // 1. outcome, or back to pending
        let message = Self::message_in(c, &message_id)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("the claimed message {message_id} is not pending")))?;
        let failed = match &outcome {
            TurnOutcome::Failed(error) if !message.notification => Some((message.clone(), error.clone())),
            _ => None,
        };
        match &outcome {
            TurnOutcome::Rescheduled { delivery, error } => {
                let deliver_at = delivery.resolve(now).map_err(invalid)?;
                let seq = next_value(c, "seq").await?;
                execute(
                    c,
                    sqlx::query("UPDATE messages SET deliver_at = ?, seq = ?, failures = failures + 1, last_error = ? WHERE id = ?")
                        .bind(micros(deliver_at))
                        .bind(seq)
                        .bind(error.as_str())
                        .bind(message_id.as_str()),
                )
                .await?;
            }
            outcome => {
                let status = outcome.status().expect("a final outcome has a status");
                self.finish(c, message, status, outcome.error().map(str::to_owned), now).await?;
            }
        }
        // 2. cancels: only this instance's pending messages (next_due_at is refreshed below)
        let mut cancelled = Vec::new();
        let mut seen = HashSet::new();
        for id in cancels.iter().filter(|id| seen.insert(*id)) {
            let pending = fetch_optional(
                c,
                sqlx::query(concat!(
                    "SELECT ",
                    message_columns!(),
                    " FROM messages WHERE id = ? AND actor_address = ?"
                ))
                .bind(id.as_str())
                .bind(address.as_str()),
            )
            .await?
            .map(|row| rows::message(&row))
            .transpose()?;
            if let Some(m) = pending {
                self.finish(c, m, OutcomeStatus::Cancelled, None, now).await?;
                cancelled.push(id.clone());
            }
        }
        // 3. deletes, 4. creates, 5. purges, 6. pauses and resumes, 7. sends
        let deleted: Vec<ActorAddress> = deleted.into_iter().collect();
        self.remove_all(c, &deleted, now).await?;
        for create in &creates {
            Self::create_in(c, &create.address, &create.state, create.child.then_some(&address), now).await?;
        }
        for target in support::purges(&controls) {
            self.purge_in(c, target, (*target == address).then_some(&message_id), now).await?;
        }
        for (target, paused) in support::pauses(&controls) {
            execute(
                c,
                sqlx::query("UPDATE actors SET paused = ? WHERE address = ?")
                    .bind(i64::from(paused))
                    .bind(target.as_str()),
            )
            .await?;
        }
        let mut sent = Vec::with_capacity(sends.len());
        for send in sends {
            sent.push(self.insert(c, send, now, false).await?);
        }
        // 8. state, 9. release, 10. the notifications of the event
        let stop = matches!(state, StateChange::Stop);
        if let StateChange::NewState(state) = state {
            execute(
                c,
                sqlx::query("UPDATE actors SET state = ?, version = version + 1 WHERE address = ?")
                    .bind(json(&state))
                    .bind(address.as_str()),
            )
            .await?;
        }
        let version: i64 =
            fetch_optional(c, sqlx::query("SELECT version FROM actors WHERE address = ?").bind(address.as_str()))
                .await?
                .map(|row| col(&row, "version"))
                .transpose()?
                .ok_or_else(|| StoreError::Corrupt(format!("the lease holder {address} vanished")))?;
        // The tree is read after the creates, so the children of this commit go with it.
        let tree = if stop { Self::tree(c, &address).await? } else { Vec::new() };
        let removed: Vec<ActorAddress> =
            deleted.iter().chain(&tree).cloned().collect::<BTreeSet<_>>().into_iter().collect();
        let removal = Removal::of_commit(&address, &message_id, &controls, stop);
        let failed = failed.as_ref().map(|(m, e)| (m, e.as_str(), cause));
        let notices = Self::notices(c, failed, (!removed.is_empty()).then_some((&removal, &removed[..]))).await?;
        if stop {
            self.remove_all(c, &tree, now).await?;
        } else {
            execute(
                c,
                sqlx::query(
                    "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE address = ?",
                )
                .bind(address.as_str()),
            )
            .await?;
            Self::refresh_next_due(c, address.as_str()).await?;
        }
        self.notify(c, &event, notices, now).await?;
        Ok(CommitResult::Committed { version, sent, cancelled })
    }
}

// ---- statement helpers ----

async fn execute(c: &mut SqliteConnection, query: Query<'_>) -> StoreResult<u64> {
    Ok(query.execute(&mut *c).await.map_err(db_error)?.rows_affected())
}

async fn fetch_optional(c: &mut SqliteConnection, query: Query<'_>) -> StoreResult<Option<SqliteRow>> {
    query.fetch_optional(&mut *c).await.map_err(db_error)
}

async fn fetch_one(c: &mut SqliteConnection, query: Query<'_>) -> StoreResult<SqliteRow> {
    query.fetch_one(&mut *c).await.map_err(db_error)
}

async fn fetch_all<T>(
    c: &mut SqliteConnection,
    query: Query<'_>,
    map: impl Fn(&SqliteRow) -> StoreResult<T>,
) -> StoreResult<Vec<T>> {
    query.fetch_all(&mut *c).await.map_err(db_error)?.iter().map(map).collect()
}

/// Increments the named sequence and returns its new value.
async fn next_value(c: &mut SqliteConnection, name: &str) -> StoreResult<i64> {
    let row = fetch_optional(
        c,
        sqlx::query("UPDATE sequences SET value = value + 1 WHERE name = ? RETURNING value").bind(name),
    )
    .await?
    .ok_or_else(|| StoreError::Corrupt(format!("the sequence '{name}' is missing")))?;
    Ok(row.get(0))
}

#[async_trait]
impl Backend for SqliteStore {
    async fn now(&self) -> StoreResult<Timestamp> {
        let tx = self.read().await?;
        let now = tx.now;
        tx.end(Ok(now)).await
    }

    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult> {
        let mut tx = self.write().await?;
        let result = Self::register_in(tx.conn(), def).await;
        tx.end(result).await
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        let mut tx = self.read().await?;
        let result = fetch_all(
            tx.conn(),
            sqlx::query("SELECT name, contract, contract_hash FROM actor_types ORDER BY name"),
            rows::type_def,
        )
        .await;
        tx.end(result).await
    }

    async fn remove_type(&self, name: &TypeName) -> StoreResult<RemoveTypeResult> {
        let prefix = format!("{name}/");
        let mut tx = self.write().await?;
        let result = async {
            let c = tx.conn();
            let registered =
                fetch_optional(c, sqlx::query("SELECT 1 FROM actor_types WHERE name = ?").bind(name.as_str()));
            if registered.await?.is_none() {
                return Ok(RemoveTypeResult::Absent);
            }
            let used = fetch_optional(
                c,
                sqlx::query("SELECT 1 FROM actors WHERE address >= ? AND address < ? LIMIT 1")
                    .bind(&prefix)
                    .bind(format!("{prefix}{}", char::MAX)),
            );
            if used.await?.is_some() {
                return Ok(RemoveTypeResult::InUse);
            }
            execute(c, sqlx::query("DELETE FROM actor_types WHERE name = ?").bind(name.as_str())).await?;
            Ok(RemoveTypeResult::Removed)
        }
        .await;
        tx.end(result).await
    }

    async fn create(&self, address: &ActorAddress, state: Value) -> StoreResult<CreateResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = Self::create_in(tx.conn(), address, &state, None, now).await;
        tx.end(result).await
    }

    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        let mut tx = self.read().await?;
        let result = Self::get_in(tx.conn(), address).await;
        tx.end(result).await
    }

    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        check_limit(limit)?;
        let mut tx = self.read().await?;
        let result = Self::prefix_page_in(tx.conn(), "address", prefix, cursor, limit, |row| {
            rows::address(&col::<String>(row, "address")?)
        })
        .await
        .map(|items| support::page_of(items, limit, |a| PageCursor(a.to_string())));
        tx.end(result).await
    }

    async fn list_records(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorRecord>> {
        check_limit(limit)?;
        let mut tx = self.read().await?;
        let result = Self::prefix_page_in(tx.conn(), actor_columns!(), prefix, cursor, limit, rows::actor_record)
            .await
            .map(|items| support::page_of(items, limit, |r: &ActorRecord| PageCursor(r.address.to_string())));
        tx.end(result).await
    }

    async fn children(
        &self,
        parent: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorAddress>> {
        check_limit(limit)?;
        let after = cursor.map_or("", |c| c.0.as_str());
        let mut tx = self.read().await?;
        let result = fetch_all(
            tx.conn(),
            sqlx::query("SELECT address FROM actors WHERE parent = ? AND address > ? ORDER BY address LIMIT ?")
                .bind(parent.as_str())
                .bind(after)
                .bind(limit as i64 + 1),
            |row| rows::address(&col::<String>(row, "address")?),
        )
        .await
        .map(|items| support::page_of(items, limit, |a| PageCursor(a.to_string())));
        tx.end(result).await
    }

    async fn delete(&self, address: &ActorAddress, event: &MessageId) -> StoreResult<DeleteResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = async {
            let c = tx.conn();
            if !Self::actor_exists(c, address.as_str()).await? {
                return Ok(DeleteResult::Absent);
            }
            let tree = Self::tree(c, address).await?;
            let removal = Removal::delete(address);
            let notices = Self::notices(c, None, Some((&removal, &tree[..]))).await?;
            let purged = self.remove_all(c, &tree, now).await?;
            self.notify(c, event, notices, now).await?;
            Ok(DeleteResult::Deleted { purged })
        }
        .await;
        tx.end(result).await
    }

    async fn watch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<SetWatchResult> {
        let mut tx = self.write().await?;
        let result = async {
            let c = tx.conn();
            if !Self::actor_exists(c, watcher.as_str()).await? {
                return Ok(SetWatchResult::NoSuchWatcher);
            }
            let inserted = execute(
                c,
                sqlx::query("INSERT INTO watches (target, watcher) VALUES (?, ?) ON CONFLICT DO NOTHING")
                    .bind(target.as_str())
                    .bind(watcher.as_str()),
            )
            .await?;
            Ok(if inserted == 1 { SetWatchResult::Created } else { SetWatchResult::Exists })
        }
        .await;
        tx.end(result).await
    }

    async fn unwatch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<bool> {
        let mut tx = self.write().await?;
        let result = execute(
            tx.conn(),
            sqlx::query("DELETE FROM watches WHERE target = ? AND watcher = ?")
                .bind(target.as_str())
                .bind(watcher.as_str()),
        )
        .await
        .map(|removed| removed == 1);
        tx.end(result).await
    }

    async fn watches(
        &self,
        watcher: Option<&ActorAddress>,
        target: Option<&WatchTarget>,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Watch>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_watch_cursor).transpose()?;
        let mut sql = "SELECT target, watcher FROM watches WHERE 1 = 1".to_owned();
        if target.is_some() {
            sql.push_str(" AND target = ?");
        }
        if watcher.is_some() {
            sql.push_str(" AND watcher = ?");
        }
        if after.is_some() {
            sql.push_str(" AND (target > ? OR (target = ? AND watcher > ?))");
        }
        sql.push_str(" ORDER BY target, watcher LIMIT ?");
        let mut query = sqlx::query(AssertSqlSafe(sql));
        if let Some(target) = target {
            query = query.bind(target.as_str());
        }
        if let Some(watcher) = watcher {
            query = query.bind(watcher.as_str());
        }
        if let Some((at, aw)) = &after {
            query = query.bind(at.as_str()).bind(at.as_str()).bind(aw.as_str());
        }
        let mut tx = self.read().await?;
        let result = fetch_all(tx.conn(), query.bind(limit as i64 + 1), |row| {
            support::stored_watch(&col::<String>(row, "target")?, &col::<String>(row, "watcher")?)
        })
        .await
        .map(|items| support::page_of(items, limit, support::watch_cursor));
        tx.end(result).await
    }

    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = async {
            let c = tx.conn();
            if self.is_id_in_use(c, &message.id, now).await? {
                return Ok(EnqueueResult::Duplicate(message.id));
            }
            if !Self::actor_exists(c, message.to.as_str()).await? {
                return Ok(EnqueueResult::NoSuchInstance);
            }
            Ok(EnqueueResult::Enqueued(self.insert(c, message, now, false).await?))
        }
        .await;
        tx.end(result).await
    }

    async fn claim_due(
        &self,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        if !support::check_claim(types, ttl, limit)? {
            return Ok(Vec::new());
        }
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = Self::claim_due_in(tx.conn(), types, node, ttl, limit, now).await;
        tx.end(result).await
    }

    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = async {
            let lease_until = time::truncate(time::add(now, ttl).map_err(invalid)?);
            let renewed = fetch_optional(
                tx.conn(),
                sqlx::query(
                    "UPDATE actors SET lease_until = ? WHERE address = ? AND epoch = ? AND lease_owner IS NOT NULL
                     RETURNING cancel_epoch",
                )
                .bind(micros(lease_until))
                .bind(address.as_str())
                .bind(epoch),
            )
            .await?;
            Ok(match renewed {
                Some(row) if col::<Option<i64>>(&row, "cancel_epoch")? == Some(epoch) => {
                    RenewResult::CancelRequested(lease_until)
                }
                Some(_) => RenewResult::Renewed(lease_until),
                None => RenewResult::LeaseLost,
            })
        }
        .await;
        tx.end(result).await
    }

    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        let mut tx = self.write().await?;
        let result = execute(
            tx.conn(),
            sqlx::query(
                "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL
                 WHERE address = ? AND epoch = ? AND lease_owner IS NOT NULL",
            )
            .bind(address.as_str())
            .bind(epoch),
        )
        .await
        .map(|_| ());
        tx.end(result).await
    }

    async fn next_due_at(&self, types: &BTreeSet<TypeName>) -> StoreResult<Option<Timestamp>> {
        if types.is_empty() {
            return Ok(None);
        }
        let sql = format!(
            "SELECT min(CASE WHEN lease_owner IS NOT NULL AND lease_until > ? THEN max(next_due_at, lease_until)
                            ELSE next_due_at END) AS due
             FROM actors WHERE type IN {} AND next_due_at IS NOT NULL AND paused = 0",
            placeholders(types.len())
        );
        let mut tx = self.read().await?;
        let mut query = sqlx::query(AssertSqlSafe(sql)).bind(micros(tx.now));
        for t in types {
            query = query.bind(t.as_str());
        }
        let result = fetch_optional(tx.conn(), query)
            .await
            .and_then(|row| row.map(|row| col::<Option<i64>>(&row, "due")).transpose())
            .map(|due| due.flatten().map(instant));
        tx.end(result).await
    }

    async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        check_limit(limit)?;
        let (after_at, after_seq) =
            cursor.map(support::decode_mailbox_cursor).transpose()?.unwrap_or((i64::MIN, i64::MIN));
        let mut tx = self.read().await?;
        let result = fetch_all(
            tx.conn(),
            sqlx::query(concat!(
                "SELECT ",
                message_columns!(),
                " FROM messages WHERE actor_address = ? AND (deliver_at > ? OR (deliver_at = ? AND seq > ?))
                 ORDER BY deliver_at, seq LIMIT ?"
            ))
            .bind(address.as_str())
            .bind(after_at)
            .bind(after_at)
            .bind(after_seq)
            .bind(limit as i64 + 1),
            rows::message,
        )
        .await
        .map(|items| support::page_of(items, limit, support::mailbox_cursor));
        tx.end(result).await
    }

    async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_pending_cursor).transpose()?;
        let query = match &after {
            None => sqlx::query(concat!(
                "SELECT ",
                message_columns!(),
                " FROM messages WHERE actor_address >= ? AND substr(actor_address, 1, ?) = ?
                 ORDER BY actor_address, deliver_at, seq LIMIT ?"
            ))
            .bind(prefix)
            .bind(prefix_len(prefix))
            .bind(prefix)
            .bind(limit as i64 + 1),
            Some((addr, at, seq)) => {
                // The lower bound lets the scan start in the mailbox index; substr() keeps the prefix filter exact.
                let lower_bound = if addr.as_str() >= prefix { addr.as_str() } else { prefix };
                sqlx::query(concat!(
                    "SELECT ",
                    message_columns!(),
                    " FROM messages WHERE actor_address >= ? AND substr(actor_address, 1, ?) = ?
                       AND (actor_address > ? OR (actor_address = ? AND (deliver_at > ? OR (deliver_at = ? AND seq > ?))))
                     ORDER BY actor_address, deliver_at, seq LIMIT ?"
                ))
                .bind(lower_bound)
                .bind(prefix_len(prefix))
                .bind(prefix)
                .bind(addr.as_str())
                .bind(addr.as_str())
                .bind(*at)
                .bind(*at)
                .bind(*seq)
                .bind(limit as i64 + 1)
            }
        };
        let mut tx = self.read().await?;
        let result = fetch_all(tx.conn(), query, rows::message)
            .await
            .map(|items| support::page_of(items, limit, support::pending_cursor));
        tx.end(result).await
    }

    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = self.cancel_in(tx.conn(), id, now).await;
        tx.end(result).await
    }

    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        let mut tx = self.read().await?;
        let now = tx.now;
        let result = self.status_in(tx.conn(), id, now).await;
        tx.end(result).await
    }

    async fn outcomes(
        &self,
        status: Option<OutcomeStatus>,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>> {
        check_limit(limit)?;
        let (after_at, after_id) =
            cursor.map(support::decode_outcome_cursor).transpose()?.unwrap_or((i64::MIN, String::new()));
        let mut tx = self.read().await?;
        let now = tx.now;
        // Without a status the condition is left out, so that `outcomes_failures` or `outcomes_finished` serves the order.
        let sql = format!(
            concat!(
                "SELECT ",
                outcome_columns!(),
                " FROM outcomes
             WHERE {}finished_at >= ?
               AND (CASE WHEN visible_until IS NULL THEN finished_at > ? ELSE visible_until > ? END)
               AND substr(actor_address, 1, ?) = ?
               AND (finished_at > ? OR (finished_at = ? AND message_id > ?))
             ORDER BY finished_at, message_id LIMIT ?"
            ),
            if status.is_some() { "status = ? AND " } else { "" }
        );
        let mut query = sqlx::query(AssertSqlSafe(sql));
        if let Some(status) = status {
            query = query.bind(status.wire_name());
        }
        let result = fetch_all(
            tx.conn(),
            query
                .bind(micros(since))
                .bind(self.cutoff(now))
                .bind(micros(now))
                .bind(prefix_len(prefix))
                .bind(prefix)
                .bind(after_at)
                .bind(after_at)
                .bind(after_id.as_str())
                .bind(limit as i64 + 1),
            rows::outcome,
        )
        .await
        .map(|items| support::page_of(items, limit, |o| support::outcome_cursor(o.finished_at, &o.message_id)));
        tx.end(result).await
    }

    async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<MessageStatus>> {
        check_limit(limit)?;
        let after = cursor.map_or("", |c| c.0.as_str());
        let mut tx = self.read().await?;
        let now = tx.now;
        let result = async {
            // The ids first, from the pending messages and the visible outcomes; then each one's status.
            let ids = fetch_all(
                tx.conn(),
                sqlx::query(
                    "SELECT id FROM (
                         SELECT id FROM messages WHERE caused_by = ? AND id > ?
                         UNION
                         SELECT message_id AS id FROM outcomes WHERE caused_by = ? AND message_id > ?
                           AND (CASE WHEN visible_until IS NULL THEN finished_at > ? ELSE visible_until > ? END)
                     ) ORDER BY id LIMIT ?",
                )
                .bind(id.as_str())
                .bind(after)
                .bind(id.as_str())
                .bind(after)
                .bind(self.cutoff(now))
                .bind(micros(now))
                .bind(limit as i64 + 1),
                |row| rows::message_id(col(row, "id")?),
            )
            .await?;
            let page = support::page_of(ids, limit, |id| PageCursor(id.to_string()));
            let mut items = Vec::with_capacity(page.items.len());
            for id in &page.items {
                items.extend(self.status_in(tx.conn(), id, now).await?);
            }
            Ok(Page::new(items, page.next))
        }
        .await;
        tx.end(result).await
    }

    async fn stats(&self, prefix: &str) -> StoreResult<StoreStats> {
        let upper = format!("{prefix}{}", char::MAX);
        let mut tx = self.read().await?;
        let now = tx.now;
        let result = async {
            let actors: i64 = col(
                &fetch_one(
                    tx.conn(),
                    sqlx::query("SELECT count(*) AS n FROM actors WHERE address >= ? AND address < ? AND substr(address, 1, ?) = ?")
                        .bind(prefix)
                        .bind(&upper)
                        .bind(prefix_len(prefix))
                        .bind(prefix),
                )
                .await?,
                "n",
            )?;
            let row = fetch_one(
                tx.conn(),
                sqlx::query(
                    "SELECT count(*) AS n, min(deliver_at) AS oldest FROM messages
                     WHERE actor_address >= ? AND actor_address < ? AND substr(actor_address, 1, ?) = ?",
                )
                .bind(prefix)
                .bind(&upper)
                .bind(prefix_len(prefix))
                .bind(prefix),
            )
            .await?;
            let oldest = col::<Option<i64>>(&row, "oldest")?.map(instant).filter(|at| *at <= now);
            Ok(StoreStats { actors: actors as u64, pending: col::<i64>(&row, "n")? as u64, oldest_due: oldest })
        }
        .await;
        tx.end(result).await
    }

    async fn sweep_expired(&self) -> StoreResult<u64> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = async {
            // Hidden records lose their content (the minimal record stays for dedup).
            execute(
                tx.conn(),
                sqlx::query(
                    "UPDATE outcomes SET error = NULL, stripped = 1,
                         envelope = json_set(envelope, '$.payload', json('null'), '$.headers', json('{}'),
                                             '$.correlationId', json('null'), '$.lastError', json('null'))
                     WHERE stripped = 0 AND visible_until IS NOT NULL AND visible_until <= ?",
                )
                .bind(micros(now)),
            )
            .await?;
            execute(
                tx.conn(),
                sqlx::query(
                    "DELETE FROM outcomes WHERE finished_at <= ? AND (visible_until IS NULL OR visible_until <= ?)",
                )
                .bind(self.cutoff(now))
                .bind(micros(now)),
            )
            .await
        }
        .await;
        tx.end(result).await
    }

    async fn deliver_now(&self, id: &MessageId) -> StoreResult<DeliverNowResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = async {
            let c = tx.conn();
            let Some(message) = Self::message_in(c, id).await? else {
                return Ok(match self.live_outcome(c, id, now).await? {
                    Some(o) => DeliverNowResult::AlreadyFinished(o),
                    None => DeliverNowResult::NotFound,
                });
            };
            let running = fetch_optional(
                c,
                sqlx::query(
                    "SELECT 1 FROM actors
                     WHERE address = ? AND lease_message = ? AND lease_owner IS NOT NULL AND lease_until > ?",
                )
                .bind(message.to.as_str())
                .bind(id.as_str())
                .bind(micros(now)),
            )
            .await?;
            if running.is_some() {
                return Ok(DeliverNowResult::Running);
            }
            let seq = next_value(c, "seq").await?;
            execute(
                c,
                sqlx::query("UPDATE messages SET deliver_at = ?, seq = ? WHERE id = ?")
                    .bind(micros(now))
                    .bind(seq)
                    .bind(id.as_str()),
            )
            .await?;
            Self::refresh_next_due(c, message.to.as_str()).await?;
            Ok(DeliverNowResult::Delivered(Message { deliver_at: now, seq, ..message }))
        }
        .await;
        tx.end(result).await
    }

    async fn set_paused(&self, address: &ActorAddress, paused: bool) -> StoreResult<bool> {
        let mut tx = self.write().await?;
        let result = execute(
            tx.conn(),
            sqlx::query("UPDATE actors SET paused = ? WHERE address = ?")
                .bind(i64::from(paused))
                .bind(address.as_str()),
        )
        .await
        .map(|updated| updated == 1);
        tx.end(result).await
    }

    async fn purge(&self, address: &ActorAddress) -> StoreResult<Option<u64>> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = self.purge_in(tx.conn(), address, None, now).await;
        tx.end(result).await
    }

    async fn heartbeat_node(&self, id: &NodeId, hosted: &BTreeSet<TypeName>) -> StoreResult<()> {
        let mut tx = self.write().await?;
        let now = micros(tx.now);
        let result = async {
            execute(
                tx.conn(),
                sqlx::query(
                    "INSERT INTO nodes (id, hosted, started_at, last_seen) VALUES (?, ?, ?, ?)
                     ON CONFLICT (id) DO UPDATE SET hosted = excluded.hosted, last_seen = excluded.last_seen",
                )
                .bind(id.as_str())
                .bind(json(&support::hosted_to_json(hosted)))
                .bind(now)
                .bind(now),
            )
            .await?;
            let oldest = micros(time::sub(tx.now, support::NODE_ROW_KEPT));
            execute(tx.conn(), sqlx::query("DELETE FROM nodes WHERE last_seen < ?").bind(oldest)).await.map(|_| ())
        }
        .await;
        tx.end(result).await
    }

    async fn list_nodes(&self) -> StoreResult<Vec<NodeRecord>> {
        let mut tx = self.read().await?;
        let result = fetch_all(
            tx.conn(),
            sqlx::query("SELECT id, hosted, started_at, last_seen FROM nodes ORDER BY id"),
            rows::node_record,
        )
        .await;
        tx.end(result).await
    }

    async fn remove_node(&self, id: &NodeId) -> StoreResult<()> {
        let mut tx = self.write().await?;
        let result =
            execute(tx.conn(), sqlx::query("DELETE FROM nodes WHERE id = ?").bind(id.as_str())).await.map(|_| ());
        tx.end(result).await
    }

    async fn running_turns(
        &self,
        node: Option<&NodeId>,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<RunningTurn>> {
        check_limit(limit)?;
        let after = cursor.map_or("", |c| c.0.as_str());
        let sql = format!(
            "SELECT {RUNNING_TURN} FROM actors
             WHERE lease_owner IS NOT NULL AND lease_until > ? AND address >= ? AND substr(address, 1, ?) = ?
               AND address > ?{}
             ORDER BY address LIMIT ?",
            if node.is_some() { " AND lease_owner = ?" } else { "" }
        );
        let mut tx = self.read().await?;
        let mut query = sqlx::query(AssertSqlSafe(sql))
            .bind(micros(tx.now))
            .bind(prefix)
            .bind(prefix_len(prefix))
            .bind(prefix)
            .bind(after);
        if let Some(node) = node {
            query = query.bind(node.as_str());
        }
        let result = fetch_all(tx.conn(), query.bind(limit as i64 + 1), rows::running_turn)
            .await
            .map(|items| support::page_of(items, limit, |t| PageCursor(t.actor.to_string())));
        tx.end(result).await
    }

    async fn running_turn(&self, address: &ActorAddress) -> StoreResult<Option<RunningTurn>> {
        let sql = format!(
            "SELECT {RUNNING_TURN} FROM actors WHERE address = ? AND lease_owner IS NOT NULL AND lease_until > ?"
        );
        let mut tx = self.read().await?;
        let now = micros(tx.now);
        let result = fetch_optional(tx.conn(), sqlx::query(AssertSqlSafe(sql)).bind(address.as_str()).bind(now))
            .await
            .and_then(|row| row.as_ref().map(rows::running_turn).transpose());
        tx.end(result).await
    }

    async fn request_cancel(&self, address: &ActorAddress) -> StoreResult<bool> {
        let mut tx = self.write().await?;
        let now = micros(tx.now);
        let result = execute(
            tx.conn(),
            sqlx::query(
                "UPDATE actors SET cancel_epoch = epoch WHERE address = ? AND lease_owner IS NOT NULL AND lease_until > ?",
            )
            .bind(address.as_str())
            .bind(now),
        )
        .await
        .map(|updated| updated == 1);
        tx.end(result).await
    }

    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult> {
        support::check_creates(&commit.creates)?;
        support::check_controls(&commit.address, &commit.controls)?;
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = self.commit_in(tx.conn(), commit, now).await;
        tx.end(result).await
    }

    fn changes(&self) -> Arc<ChangeHub> {
        self.hub.clone()
    }

    async fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.hub.close();
        self.reader_permits.close();
        if let Some(writer) = self.writer.lock().await.take() {
            writer.close().await;
        }
        let readers = std::mem::take(&mut *self.readers.lock().unwrap_or_else(|e| e.into_inner()));
        for reader in readers {
            reader.close().await;
        }
    }
}
