//! [`SqliteStore`]: the [`Store`] on one SQLite file.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::address::{ActorAddress, UriTemplate};
use cicada::retention::Retention;
use cicada::retry::RetryPolicy;
use cicada::store::support::{self, check_limit};
use cicada::store::{
    ActorRecord, CancelResult, ChainEntry, Claim, Clock, CommitRequest, CommitResult, CreateResult, DeleteResult,
    EnqueueResult, Message, MessageId, MessageStatus, NodeId, OutcomeRecord, OutcomeStatus, OutgoingMessage, Page,
    PageCursor, RegisterResult, RenewResult, StateChange, Store, StoreError, StoreResult, SystemClock, TurnOutcome,
    TypeDef,
};
use cicada::time::{self, DEFAULT_RETENTION, Timestamp};
use serde_json::Value;
use sqlx::sqlite::{SqliteArguments, SqliteConnectOptions, SqliteJournalMode, SqliteRow, SqliteSynchronous};
use sqlx::{AssertSqlSafe, Row, Sqlite, SqliteConnection};
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::conn::{Conn, Mode, db_error};
use crate::rows::{self, col, instant, json, micros};
use crate::schema;

/// Settings of a [`SqliteStore`].
#[derive(Clone)]
pub struct SqliteOptions {
    /// The store clock; pass a virtual clock in tests.
    pub clock: Arc<dyn Clock>,
    /// Outcome retention and dedup window (S10).
    pub retention: Duration,
    /// How long a statement waits for another connection's (or process's) lock.
    pub busy_timeout: Duration,
    /// Number of read connections.
    pub readers: usize,
}

impl Default for SqliteOptions {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            retention: DEFAULT_RETENTION,
            busy_timeout: Duration::from_secs(5),
            readers: 4,
        }
    }
}

impl std::fmt::Debug for SqliteOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteOptions")
            .field("retention", &self.retention)
            .field("busy_timeout", &self.busy_timeout)
            .field("readers", &self.readers)
            .finish_non_exhaustive()
    }
}

/// [`Store`] on one SQLite database file in WAL mode, for single-host deployments. The schema is the Kotlin
/// implementation's, so either implementation opens a file the other wrote (ADR 0004).
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
    /// Fails with [`StoreError::Invalid`] when `readers` is 0 or the file was written by a newer schema version, and
    /// with [`StoreError::Unavailable`] when the file cannot be opened.
    pub async fn open(path: impl AsRef<Path>, options: SqliteOptions) -> StoreResult<Self> {
        if options.readers == 0 {
            return Err(StoreError::Invalid("readers must be positive".into()));
        }
        let base = SqliteConnectOptions::new()
            .filename(path.as_ref())
            .busy_timeout(options.busy_timeout)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true);
        let mut writer =
            Conn::connect(&base.clone().create_if_missing(true).journal_mode(SqliteJournalMode::Wal)).await?;
        if let Err(e) = schema::migrate(&mut writer).await {
            writer.close().await;
            return Err(e);
        }
        // WAL is a property of the file, set by the writer; readers only read.
        let reader_options = base.read_only(true);
        let mut readers = Vec::with_capacity(options.readers);
        for _ in 0..options.readers {
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
        let existing =
            fetch_optional(c, sqlx::query("SELECT * FROM actor_types WHERE template = ?").bind(def.template.as_str()))
                .await?
                .map(|row| rows::type_def(&row))
                .transpose()?;
        if let Some(existing) = existing {
            return Ok(if existing.contract_hash == def.contract_hash {
                RegisterResult::Registered(existing)
            } else {
                RegisterResult::Conflict { reason: support::hash_conflict_reason(&def.template), existing }
            });
        }
        // BEGIN IMMEDIATE serialises registrations, so the overlap check sees every committed template.
        let registered = fetch_all(c, sqlx::query("SELECT * FROM actor_types"), rows::type_def).await?;
        if let Some(overlapping) = support::overlapping(&registered, &def.template) {
            return Ok(RegisterResult::Conflict {
                reason: support::overlap_reason(&def.template, &overlapping.template),
                existing: overlapping.clone(),
            });
        }
        execute(
            c,
            sqlx::query("INSERT INTO actor_types (template, shape, contract, contract_hash) VALUES (?, ?, ?, ?)")
                .bind(def.template.as_str())
                .bind(def.template.shape())
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
        template: &UriTemplate,
        state: Option<&Value>,
        now: Timestamp,
    ) -> StoreResult<CreateResult> {
        let inserted = execute(
            c,
            sqlx::query(
                "INSERT INTO actors (uri, type, state, created_at) VALUES (?, ?, ?, ?) ON CONFLICT (uri) DO NOTHING",
            )
            .bind(address.as_str())
            .bind(template.as_str())
            .bind(state.map(json))
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

    async fn get_in(c: &mut SqliteConnection, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        fetch_optional(
            c,
            sqlx::query("SELECT uri, type, state, version, created_at FROM actors WHERE uri = ?")
                .bind(address.as_str()),
        )
        .await?
        .map(|row| rows::actor_record(&row))
        .transpose()
    }

    async fn actor_exists(c: &mut SqliteConnection, uri: &str) -> StoreResult<bool> {
        Ok(fetch_optional(c, sqlx::query("SELECT 1 FROM actors WHERE uri = ?").bind(uri)).await?.is_some())
    }

    async fn purge_mailbox(
        &self,
        c: &mut SqliteConnection,
        address: &ActorAddress,
        now: Timestamp,
    ) -> StoreResult<usize> {
        let pending = fetch_all(
            c,
            sqlx::query("SELECT * FROM messages WHERE actor_uri = ? ORDER BY deliver_at, seq").bind(address.as_str()),
            rows::message,
        )
        .await?;
        let purged = pending.len();
        for message in pending {
            self.finish(c, message, OutcomeStatus::Purged, None, now).await?;
        }
        Ok(purged)
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

    async fn insert(&self, c: &mut SqliteConnection, message: OutgoingMessage, now: Timestamp) -> StoreResult<Message> {
        let deliver_at = message.delivery.resolve(now).map_err(invalid)?;
        // An expired, not yet swept outcome with the same id.
        execute(c, sqlx::query("DELETE FROM outcomes WHERE message_id = ?").bind(message.id.as_str())).await?;
        let stored = Message {
            id: message.id,
            to: message.to,
            from: message.from,
            message_type: message.message_type,
            payload: message.payload,
            deliver_at,
            correlation_id: message.correlation_id,
            headers: message.headers,
            seq: next_value(c, "seq").await?,
            attempt: 0,
            failures: 0,
            last_error: None,
            retry: message.retry,
            caused_by: message.caused_by,
            retention: message.retention,
        };
        execute(
            c,
            sqlx::query(
                "INSERT INTO messages
                    (id, actor_uri, seq, type, payload, sender, correlation_id, headers, deliver_at, attempt, retry_policy,
                     caused_by, retention_policy)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?)",
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
            .bind(stored.retention.as_ref().map(|r| json(&Retention::to_json(r)))),
        )
        .await?;
        execute(
            c,
            sqlx::query("UPDATE actors SET next_due_at = min(coalesce(next_due_at, ?), ?) WHERE uri = ?")
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
                    (message_id, actor_uri, status, error, finished_at, envelope, caused_by, visible_until, keep_last, keep_seq,
                     stripped)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (message_id) DO UPDATE SET actor_uri = excluded.actor_uri, status = excluded.status,
                    error = excluded.error, finished_at = excluded.finished_at, envelope = excluded.envelope,
                    caused_by = excluded.caused_by, visible_until = excluded.visible_until, keep_last = excluded.keep_last,
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
                         WHERE actor_uri = ? AND status = ? AND keep_last = 1
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

    async fn refresh_next_due(c: &mut SqliteConnection, uri: &str) -> StoreResult<()> {
        execute(
            c,
            sqlx::query("UPDATE actors SET next_due_at = (SELECT min(deliver_at) FROM messages WHERE actor_uri = ?) WHERE uri = ?")
                .bind(uri)
                .bind(uri),
        )
        .await?;
        Ok(())
    }

    async fn claim_due_in(
        c: &mut SqliteConnection,
        types: &BTreeSet<UriTemplate>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
        now: Timestamp,
    ) -> StoreResult<Vec<Claim>> {
        let n = micros(now);
        let sql = format!(
            "SELECT uri, type, state, version FROM actors
             WHERE type IN {} AND next_due_at <= ?
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
                col::<String>(row, "uri")?,
                rows::template(&col::<String>(row, "type")?)?,
                col::<Option<String>>(row, "state")?,
                col::<i64>(row, "version")?,
            ))
        })
        .await?;
        let lease_until = time::truncate(time::add(now, ttl).map_err(invalid)?);
        let mut claims = Vec::with_capacity(candidates.len());
        for (uri, template, state, version) in candidates {
            let head = fetch_optional(
                c,
                sqlx::query("SELECT * FROM messages WHERE actor_uri = ? ORDER BY deliver_at, seq LIMIT 1")
                    .bind(uri.as_str()),
            )
            .await?
            .map(|row| rows::message(&row))
            .transpose()?
            .filter(|head| head.deliver_at <= now)
            .ok_or_else(|| StoreError::Corrupt(format!("next_due_at out of sync for {uri}")))?;
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
                    "UPDATE actors SET epoch = ?, lease_owner = ?, lease_until = ?, lease_message = ? WHERE uri = ?",
                )
                .bind(epoch)
                .bind(node.as_str())
                .bind(micros(lease_until))
                .bind(claimed.id.as_str())
                .bind(uri.as_str()),
            )
            .await?;
            claims.push(Claim {
                actor: rows::address(&uri)?,
                template,
                epoch,
                message: claimed,
                state: state.map(|s| rows::parse_json(&s, "state")).transpose()?,
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
            sqlx::query("SELECT lease_message FROM actors WHERE uri = ? AND epoch = ? AND lease_owner IS NOT NULL")
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
            sqlx::query(
                "SELECT * FROM outcomes WHERE message_id = ?
                   AND (CASE WHEN visible_until IS NULL THEN finished_at > ? ELSE visible_until > ? END)",
            )
            .bind(id.as_str())
            .bind(self.cutoff(now))
            .bind(micros(now)),
        )
        .await?
        .map(|row| rows::outcome(&row))
        .transpose()
    }

    async fn message_in(c: &mut SqliteConnection, id: &MessageId) -> StoreResult<Option<Message>> {
        fetch_optional(c, sqlx::query("SELECT * FROM messages WHERE id = ?").bind(id.as_str()))
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
                 WHERE uri = ? AND lease_message = ? AND lease_owner IS NOT NULL AND lease_until > ?",
            )
            .bind(message.to.as_str())
            .bind(id.as_str())
            .bind(micros(now)),
        )
        .await?;
        Ok(Some(match running {
            Some(row) => MessageStatus::Running {
                node: NodeId::new(col::<String>(&row, "lease_owner")?).map_err(StoreError::Corrupt)?,
                lease_until: instant(col(&row, "lease_until")?),
                message,
            },
            None => MessageStatus::Pending(message),
        }))
    }

    async fn chain_entry_in(
        &self,
        c: &mut SqliteConnection,
        id: &MessageId,
        now: Timestamp,
    ) -> StoreResult<Option<ChainEntry>> {
        if let Some(status) = self.status_in(c, id, now).await? {
            return Ok(Some(ChainEntry::Live(status)));
        }
        let removed = fetch_optional(
            c,
            sqlx::query("SELECT * FROM outcomes WHERE message_id = ? AND (finished_at > ? OR visible_until > ?)")
                .bind(id.as_str())
                .bind(self.cutoff(now))
                .bind(micros(now)),
        )
        .await?
        .map(|row| rows::outcome(&row))
        .transpose()?;
        Ok(removed.map(|o| ChainEntry::Removed {
            status: o.status,
            finished_at: o.finished_at,
            envelope: support::stripped(&o.envelope),
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
                "SELECT lease_until FROM actors WHERE uri = ? AND lease_message = ? AND lease_owner IS NOT NULL",
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
                    "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE uri = ?",
                )
                .bind(message.to.as_str()),
            )
            .await?;
        }
        let uri = message.to.as_str().to_owned();
        let outcome = self.finish(c, message, OutcomeStatus::Cancelled, None, now).await?;
        Self::refresh_next_due(c, &uri).await?;
        Ok(CancelResult::Cancelled(outcome))
    }

    async fn commit_in(
        &self,
        c: &mut SqliteConnection,
        commit: CommitRequest,
        now: Timestamp,
    ) -> StoreResult<CommitResult> {
        let CommitRequest { address, epoch, message_id, outcome, state, creates, sends, cancels } = commit;
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
        let created: HashSet<&ActorAddress> = creates.iter().map(|c| &c.address).collect();
        let mut send_ids = HashSet::new();
        for send in &sends {
            let exists = created.contains(&send.to) || Self::actor_exists(c, send.to.as_str()).await?;
            if !exists {
                return Ok(CommitResult::Rejected(format!("Send to absent instance {}", send.to)));
            }
            if !send_ids.insert(&send.id) || send.id == message_id || self.is_id_in_use(c, &send.id, now).await? {
                return Ok(CommitResult::Rejected(format!("Message id {} is already in use", send.id)));
            }
        }

        // 1. outcome, or back to pending
        let message = Self::message_in(c, &message_id)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("the claimed message {message_id} is not pending")))?;
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
                sqlx::query("SELECT * FROM messages WHERE id = ? AND actor_uri = ?")
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
        // 3. creates, 4. sends
        for create in &creates {
            Self::create_in(c, &create.address, &create.template, create.state.as_ref(), now).await?;
        }
        let mut sent = Vec::with_capacity(sends.len());
        for send in sends {
            sent.push(self.insert(c, send, now).await?);
        }
        // 5. state, 6. release
        let stop = matches!(state, StateChange::Stop);
        match state {
            StateChange::Unchanged => {}
            StateChange::NewState(state) => {
                execute(
                    c,
                    sqlx::query("UPDATE actors SET state = ?, version = version + 1 WHERE uri = ?")
                        .bind(state.as_ref().map(json))
                        .bind(address.as_str()),
                )
                .await?;
            }
            StateChange::Stop => {
                self.purge_mailbox(c, &address, now).await?;
            }
        }
        let version: i64 =
            fetch_optional(c, sqlx::query("SELECT version FROM actors WHERE uri = ?").bind(address.as_str()))
                .await?
                .map(|row| col(&row, "version"))
                .transpose()?
                .ok_or_else(|| StoreError::Corrupt(format!("the lease holder {address} vanished")))?;
        if stop {
            execute(c, sqlx::query("DELETE FROM actors WHERE uri = ?").bind(address.as_str())).await?;
        } else {
            execute(
                c,
                sqlx::query(
                    "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE uri = ?",
                )
                .bind(address.as_str()),
            )
            .await?;
            Self::refresh_next_due(c, address.as_str()).await?;
        }
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

/// The first `limit` of `items` (which holds at most `limit + 1`), with the cursor of the last one when there are more.
fn page_of<T>(mut items: Vec<T>, limit: usize, cursor: impl Fn(&T) -> PageCursor) -> Page<T> {
    if items.len() > limit {
        items.truncate(limit);
        let next = items.last().map(&cursor);
        Page::new(items, next)
    } else {
        Page::new(items, None)
    }
}

#[async_trait]
impl Store for SqliteStore {
    fn retention(&self) -> Duration {
        self.retention
    }

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

    async fn type_def(&self, template: &UriTemplate) -> StoreResult<Option<TypeDef>> {
        let mut tx = self.read().await?;
        let result = fetch_optional(
            tx.conn(),
            sqlx::query("SELECT * FROM actor_types WHERE template = ?").bind(template.as_str()),
        )
        .await
        .and_then(|row| row.map(|row| rows::type_def(&row)).transpose());
        tx.end(result).await
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        let mut tx = self.read().await?;
        let result =
            fetch_all(tx.conn(), sqlx::query("SELECT * FROM actor_types ORDER BY template"), rows::type_def).await;
        tx.end(result).await
    }

    async fn create(
        &self,
        address: &ActorAddress,
        template: &UriTemplate,
        state: Option<Value>,
    ) -> StoreResult<CreateResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = Self::create_in(tx.conn(), address, template, state.as_ref(), now).await;
        tx.end(result).await
    }

    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        let mut tx = self.read().await?;
        let result = Self::get_in(tx.conn(), address).await;
        tx.end(result).await
    }

    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        check_limit(limit)?;
        let (lower_bound, inclusive) = match cursor {
            Some(c) if c.0.as_str() >= prefix => (c.0.as_str(), false),
            _ => (prefix, true),
        };
        let sql = if inclusive {
            "SELECT uri FROM actors WHERE uri >= ? AND substr(uri, 1, ?) = ? ORDER BY uri LIMIT ?"
        } else {
            "SELECT uri FROM actors WHERE uri > ? AND substr(uri, 1, ?) = ? ORDER BY uri LIMIT ?"
        };
        let mut tx = self.read().await?;
        let result = fetch_all(
            tx.conn(),
            sqlx::query(sql).bind(lower_bound).bind(prefix_len(prefix)).bind(prefix).bind(limit as i64 + 1),
            |row| rows::address(&col::<String>(row, "uri")?),
        )
        .await
        .map(|items| page_of(items, limit, |a| PageCursor(a.to_string())));
        tx.end(result).await
    }

    async fn delete(&self, address: &ActorAddress) -> StoreResult<DeleteResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = async {
            let c = tx.conn();
            if !Self::actor_exists(c, address.as_str()).await? {
                return Ok(DeleteResult::Absent);
            }
            let purged = self.purge_mailbox(c, address, now).await?;
            execute(c, sqlx::query("DELETE FROM actors WHERE uri = ?").bind(address.as_str())).await?;
            Ok(DeleteResult::Deleted { purged })
        }
        .await;
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
            Ok(EnqueueResult::Enqueued(self.insert(c, message, now).await?))
        }
        .await;
        tx.end(result).await
    }

    async fn claim_due(
        &self,
        types: &BTreeSet<UriTemplate>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        if limit == 0 || types.is_empty() {
            return Ok(Vec::new());
        }
        if ttl.is_zero() {
            return Err(StoreError::Invalid("ttl must be positive".into()));
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
            let updated = execute(
                tx.conn(),
                sqlx::query(
                    "UPDATE actors SET lease_until = ? WHERE uri = ? AND epoch = ? AND lease_owner IS NOT NULL",
                )
                .bind(micros(lease_until))
                .bind(address.as_str())
                .bind(epoch),
            )
            .await?;
            Ok(if updated == 1 { RenewResult::Renewed(lease_until) } else { RenewResult::LeaseLost })
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
                 WHERE uri = ? AND epoch = ? AND lease_owner IS NOT NULL",
            )
            .bind(address.as_str())
            .bind(epoch),
        )
        .await
        .map(|_| ());
        tx.end(result).await
    }

    async fn next_due_at(&self, types: &BTreeSet<UriTemplate>) -> StoreResult<Option<Timestamp>> {
        if types.is_empty() {
            return Ok(None);
        }
        let sql = format!(
            "SELECT min(CASE WHEN lease_owner IS NOT NULL AND lease_until > ? THEN max(next_due_at, lease_until)
                            ELSE next_due_at END) AS due
             FROM actors WHERE type IN {} AND next_due_at IS NOT NULL",
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
            sqlx::query(
                "SELECT * FROM messages WHERE actor_uri = ? AND (deliver_at > ? OR (deliver_at = ? AND seq > ?))
                 ORDER BY deliver_at, seq LIMIT ?",
            )
            .bind(address.as_str())
            .bind(after_at)
            .bind(after_at)
            .bind(after_seq)
            .bind(limit as i64 + 1),
            rows::message,
        )
        .await
        .map(|items| page_of(items, limit, support::mailbox_cursor));
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
            None => sqlx::query(
                "SELECT * FROM messages WHERE actor_uri >= ? AND substr(actor_uri, 1, ?) = ?
                 ORDER BY actor_uri, deliver_at, seq LIMIT ?",
            )
            .bind(prefix)
            .bind(prefix_len(prefix))
            .bind(prefix)
            .bind(limit as i64 + 1),
            Some((uri, at, seq)) => {
                // The lower bound lets the scan start in the mailbox index; substr() keeps the prefix filter exact.
                let lower_bound = if uri.as_str() >= prefix { uri.as_str() } else { prefix };
                sqlx::query(
                    "SELECT * FROM messages WHERE actor_uri >= ? AND substr(actor_uri, 1, ?) = ?
                       AND (actor_uri > ? OR (actor_uri = ? AND (deliver_at > ? OR (deliver_at = ? AND seq > ?))))
                     ORDER BY actor_uri, deliver_at, seq LIMIT ?",
                )
                .bind(lower_bound)
                .bind(prefix_len(prefix))
                .bind(prefix)
                .bind(uri.as_str())
                .bind(uri.as_str())
                .bind(*at)
                .bind(*at)
                .bind(*seq)
                .bind(limit as i64 + 1)
            }
        };
        let mut tx = self.read().await?;
        let result = fetch_all(tx.conn(), query, rows::message)
            .await
            .map(|items| page_of(items, limit, support::pending_cursor));
        tx.end(result).await
    }

    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = self.cancel_in(tx.conn(), id, now).await;
        tx.end(result).await
    }

    async fn outcome(&self, id: &MessageId) -> StoreResult<Option<OutcomeRecord>> {
        let mut tx = self.read().await?;
        let now = tx.now;
        let result = self.live_outcome(tx.conn(), id, now).await;
        tx.end(result).await
    }

    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        let mut tx = self.read().await?;
        let now = tx.now;
        let result = self.status_in(tx.conn(), id, now).await;
        tx.end(result).await
    }

    async fn failures(
        &self,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>> {
        check_limit(limit)?;
        let (after_at, after_id) =
            cursor.map(support::decode_failure_cursor).transpose()?.unwrap_or((i64::MIN, String::new()));
        let mut tx = self.read().await?;
        let now = tx.now;
        let result = fetch_all(
            tx.conn(),
            sqlx::query(
                "SELECT * FROM outcomes
                 WHERE status = ? AND finished_at >= ?
                   AND (CASE WHEN visible_until IS NULL THEN finished_at > ? ELSE visible_until > ? END)
                   AND substr(actor_uri, 1, ?) = ?
                   AND (finished_at > ? OR (finished_at = ? AND message_id > ?))
                 ORDER BY finished_at, message_id LIMIT ?",
            )
            .bind(OutcomeStatus::Failed.wire_name())
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
        .map(|items| page_of(items, limit, |o| support::failure_cursor(o.finished_at, &o.message_id)));
        tx.end(result).await
    }

    async fn chain_entries(&self, ids: &[MessageId]) -> StoreResult<Vec<ChainEntry>> {
        let mut tx = self.read().await?;
        let now = tx.now;
        let result = async {
            let mut seen = HashSet::new();
            let mut entries = Vec::new();
            for id in ids.iter().filter(|id| seen.insert(*id)) {
                if let Some(entry) = self.chain_entry_in(tx.conn(), id, now).await? {
                    entries.push(entry);
                }
            }
            Ok(entries)
        }
        .await;
        tx.end(result).await
    }

    async fn chain_children(&self, parents: &[MessageId], limit: usize) -> StoreResult<Vec<ChainEntry>> {
        check_limit(limit)?;
        let mut seen = HashSet::new();
        let wanted: Vec<&str> = parents.iter().map(MessageId::as_str).filter(|id| seen.insert(*id)).collect();
        if wanted.is_empty() {
            self.check_open()?;
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT id FROM messages WHERE caused_by IN {in_list}
             UNION ALL
             SELECT message_id FROM outcomes WHERE caused_by IN {in_list} AND (finished_at > ? OR visible_until > ?)
             LIMIT ?",
            in_list = placeholders(wanted.len())
        );
        let mut tx = self.read().await?;
        let now = tx.now;
        let result = async {
            let mut query = sqlx::query(AssertSqlSafe(sql));
            for id in wanted.iter().chain(wanted.iter()) {
                query = query.bind(*id);
            }
            let query = query.bind(self.cutoff(now)).bind(micros(now)).bind(limit as i64);
            let ids = fetch_all(tx.conn(), query, |row| {
                rows::message_id(row.try_get(0).map_err(|e| StoreError::Corrupt(e.to_string()))?)
            })
            .await?;
            let mut entries = Vec::with_capacity(ids.len());
            for id in &ids {
                if let Some(entry) = self.chain_entry_in(tx.conn(), id, now).await? {
                    entries.push(entry);
                }
            }
            Ok(entries)
        }
        .await;
        tx.end(result).await
    }

    async fn sweep_expired(&self) -> StoreResult<u64> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = async {
            // Hidden records lose their content (the minimal record stays for dedup and chains).
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

    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult> {
        let mut tx = self.write().await?;
        let now = tx.now;
        let result = self.commit_in(tx.conn(), commit, now).await;
        tx.end(result).await
    }

    async fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
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
