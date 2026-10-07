//! [`PostgresStore`]: the multi-node store on PostgreSQL.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use cicada::address::{ActorAddress, UriTemplate};
use cicada::retention::Retention;
use cicada::retry::RetryPolicy;
use cicada::store::support::{self, check_limit};
use cicada::store::{
    ActorCreate, ActorRecord, CancelResult, ChainEntry, Claim, CommitRequest, CommitResult, CreateResult,
    DeleteResult, EnqueueResult, Message, MessageId, MessageStatus, NodeId, OutcomeRecord, OutcomeStatus,
    OutgoingMessage, Page, PageCursor, RegisterResult, RenewResult, StateChange, Store, StoreError, StoreResult,
    TurnOutcome, TypeDef,
};
use cicada::time::{self, Timestamp};
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::Value;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row};
use tokio::sync::broadcast::error::RecvError;

use crate::config::PostgresStoreConfig;
use crate::connection::PostgresConnection;
use crate::error::{is_transient, map_error};
use crate::listener::NotificationListener;
use crate::schema;

/// Runs `body` (a block that evaluates to `TxResult<T>`, with `tx: &mut Tx` in scope) as one transaction and sends the
/// collected wake-ups with it; evaluates to `StoreResult<T>`. Re-runs the whole body when PostgreSQL aborted it for a
/// deadlock or serialization failure. A macro rather than a function taking a closure: futures of closures that borrow
/// their argument cannot be proven `Send` for every lifetime, which `async_trait` requires.
macro_rules! transaction {
    ($store:expr, $read_only:expr, |$tx:ident| -> $t:ty $body:block) => {{
        let store: &PostgresStore = $store;
        store.check_open()?;
        let mut attempt = 0u32;
        loop {
            let result: Result<$t, TxError> = async {
                let mut owned = store.begin($read_only).await?;
                let outcome: TxResult<$t> = store
                    .timed(async {
                        let $tx: &mut Tx = &mut owned;
                        $body
                    })
                    .await;
                let ended = store.timed(store.end(&mut owned, outcome)).await;
                if owned.open {
                    // Cut off mid-transaction or unable to roll back: never hand this connection out again.
                    owned.c.close_on_drop();
                }
                ended
            }
            .await;
            match result {
                Ok(value) => break Ok(value),
                Err(e) if store.retry(&e, &mut attempt).await => continue,
                Err(e) => break Err(e.into_store_error(store.socket_timeout)),
            }
        }
    }};
}

/// [`Store`] on PostgreSQL: the multi-node reference backend. Any number of nodes (processes, hosts, also Kotlin
/// nodes) may open a store on the same database and schema; the database is their only coordination.
///
/// **Transactions.** Every operation is one transaction at `READ COMMITTED` on a pooled connection. A transaction that
/// writes an instance or its mailbox first locks the instance's `actors` row (`SELECT … FOR UPDATE`), so all writes to
/// one instance are serialised (S3); `commit` locks the committing instance together with every instance it sends to
/// or creates, in URI order, which avoids deadlocks between commits. `claim_due` locks candidates with
/// `FOR UPDATE SKIP LOCKED`, so concurrent claims from many nodes never wait for each other and never share an
/// instance (S4). Transactions PostgreSQL aborts for a deadlock or serialization failure are re-run
/// ([`PostgresStoreConfig::transaction_retries`]).
///
/// **Clock (S9).** The store clock is the database server's `clock_timestamp()`; the local clock is never used for
/// due-ness or leases. Each transaction reads it once, *after* taking its row locks, and uses that single value for
/// every statement (truncated to microseconds, the precision of `timestamptz`). `now()` (the transaction start time)
/// is deliberately not used: a transaction that waited for a row lock would act on a time from before the wait.
///
/// **Sequences.** `seq` and epochs come from the store-wide sequences `message_seq` and `epoch_seq`, which are durable
/// and never reused, so epochs stay monotonic across delete/re-create and restarts (S1, S4).
///
/// **Dedup (S7).** Message ids are the primary key of `messages`; an id with a retained outcome is a duplicate too.
/// The outcome check runs after inserting the message, so a concurrent commit that moves the same id from `messages`
/// to `outcomes` cannot slip through.
///
/// **Wake-ups.** The store `NOTIFY`s the channel [`PostgresStoreConfig::effective_channel`] whenever a message becomes
/// claimable (enqueue, sends of a commit, a commit leaving messages behind, release, cancel of an expired turn) with
/// the instance's type template as payload. Notifications are transactional, so they are only delivered when the
/// write committed. [`Store::wake_ups`] listens on one dedicated connection, started on first use and re-connected
/// with back-off when it breaks.
///
/// **JSON.** State, payloads, headers, contracts and envelopes are `jsonb`, which normalises objects (key order,
/// duplicate keys) and rejects the escape `\u0000` in strings.
///
/// **Errors.** Values PostgreSQL refuses (SQLSTATE classes `22` and `54`, e.g. a NUL character or an id too long for
/// its index) are [`StoreError::Invalid`]; stored data that cannot be decoded is [`StoreError::Corrupt`]; everything
/// else, including a closed store, is [`StoreError::Unavailable`].
///
/// The store owns its pool and listener; [`Store::close`] releases them.
pub struct PostgresStore {
    pool: PgPool,
    retention: Duration,
    retries: u32,
    socket_timeout: Duration,
    channel: String,
    listener: NotificationListener,
    closed: AtomicBool,
}

impl std::fmt::Debug for PostgresStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresStore").field("channel", &self.channel).finish_non_exhaustive()
    }
}

impl PostgresStore {
    /// Opens a store on the database at `url`, creating or migrating its schema. `url` is a libpq-style URL with
    /// credentials as usually found in `DATABASE_URL` (`postgres://actors:secret@db:5432/actors?sslmode=require`) or
    /// a JDBC URL (`jdbc:postgresql://db:5432/actors`); see [`PostgresConnection`].
    ///
    /// Fails with [`StoreError::Invalid`] for invalid settings or a URL that is not a PostgreSQL URL, or a database
    /// migrated by a newer version, and with [`StoreError::Unavailable`] when the database cannot be reached.
    pub async fn connect(url: &str, config: PostgresStoreConfig) -> StoreResult<Self> {
        Self::connect_as(url, None, None, config).await
    }

    /// Like [`Self::connect`], connecting as `user` with `password`; when given, they win over the credentials in
    /// the URL.
    pub async fn connect_as(
        url: &str,
        user: Option<&str>,
        password: Option<&str>,
        config: PostgresStoreConfig,
    ) -> StoreResult<Self> {
        config.validate()?;
        let base = PostgresConnection::parse(url, user, password)?.connect_options()?;
        let mut options = base.clone();
        if options.get_application_name().is_none() {
            options = options.application_name("cicada");
        }
        // The search path is a startup parameter of every connection, so it holds in every transaction.
        let options = options.options([("search_path", config.schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(config.max_pool_size)
            .min_connections(config.min_idle)
            .acquire_timeout(config.connection_timeout)
            .connect_lazy_with(options);
        let migrated = async {
            let mut c = pool.acquire().await.map_err(map_error)?;
            schema::migrate(&mut c, &config.schema).await
        };
        if let Err(e) = migrated.await {
            pool.close().await;
            return Err(e);
        }
        let channel = config.effective_channel();
        let listener = NotificationListener::new(
            channel.clone(),
            base.application_name("cicada-listener"),
            config.connection_timeout,
            config.listener_reconnect_delay,
        );
        Ok(Self {
            pool,
            retention: config.retention,
            retries: config.transaction_retries,
            socket_timeout: config.socket_timeout,
            channel,
            listener,
            closed: AtomicBool::new(false),
        })
    }

    /// True while the wake-up listener has `LISTEN` active. For tests and health checks.
    pub fn is_listening(&self) -> bool {
        self.listener.is_listening()
    }

    // ---- execution ----

    fn check_open(&self) -> StoreResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(StoreError::unavailable("Store is closed"));
        }
        Ok(())
    }

    /// A transaction at `READ COMMITTED` on a pooled connection.
    async fn begin(&self, read_only: bool) -> Result<Tx, TxError> {
        let c = self.pool.acquire().await?;
        let mut tx = Tx { c, now: None, notify: Vec::new(), retention: self.retention, open: true };
        let begin = if read_only {
            "BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY"
        } else {
            "BEGIN ISOLATION LEVEL READ COMMITTED"
        };
        self.timed(async { Ok(sqlx::raw_sql(begin).execute(&mut *tx.c).await?) }).await?;
        Ok(tx)
    }

    /// `work`, cut off after the socket timeout.
    async fn timed<T>(&self, work: impl Future<Output = Result<T, TxError>>) -> Result<T, TxError> {
        if self.socket_timeout.is_zero() {
            return work.await;
        }
        tokio::time::timeout(self.socket_timeout, work).await.unwrap_or(Err(TxError::TimedOut))
    }

    /// Ends `tx` as `outcome` asks: commit (sending the collected wake-ups with it) or roll back.
    async fn end<T>(&self, tx: &mut Tx, outcome: TxResult<T>) -> Result<T, TxError> {
        let (result, commit) = match outcome {
            Err(TxError::TimedOut) => return Err(TxError::TimedOut), // the connection is busy: it is closed instead
            Ok(Done::Commit(result)) => (Ok(result), true),
            Ok(Done::Rollback(result)) => (Ok(result), false),
            Err(e) => (Err(e), false),
        };
        if commit {
            for template in std::mem::take(&mut tx.notify) {
                sqlx::query("SELECT pg_notify($1, $2)").bind(&self.channel).bind(template).execute(&mut *tx.c).await?;
            }
            sqlx::raw_sql("COMMIT").execute(&mut *tx.c).await?;
        } else {
            sqlx::raw_sql("ROLLBACK").execute(&mut *tx.c).await?;
        }
        tx.open = false;
        result
    }

    /// Whether a failed attempt is re-run: PostgreSQL aborted it for a deadlock or serialization failure, and retries
    /// are left. Waits a little before the next attempt.
    async fn retry(&self, error: &TxError, attempt: &mut u32) -> bool {
        let TxError::Sql(e) = error else { return false };
        if !is_transient(e) || *attempt >= self.retries {
            return false;
        }
        *attempt += 1;
        tokio::time::sleep(Duration::from_millis(1 + random_below(5 * *attempt as u64))).await;
        true
    }

    // ---- type registry ----

    async fn register_in(tx: &mut Tx, def: &TypeDef) -> Result<RegisterResult, TxError> {
        let same = |existing: TypeDef| {
            if existing.contract_hash == def.contract_hash {
                RegisterResult::Registered(existing)
            } else {
                RegisterResult::Conflict { existing, reason: support::hash_conflict_reason(&def.template) }
            }
        };
        // The common case, every node re-registering its types at start, needs no lock.
        if let Some(existing) = type_def_in(tx, &def.template).await? {
            return Ok(same(existing));
        }
        // Overlap is a relation between rows, which no unique index expresses: registrations are serialised by a lock
        // that conflicts with itself but not with reads. READ COMMITTED: statements after the lock see every
        // registration committed before it was granted.
        sqlx::query("LOCK TABLE actor_types IN SHARE ROW EXCLUSIVE MODE").execute(&mut *tx.c).await?;
        if let Some(existing) = type_def_in(tx, &def.template).await? {
            return Ok(same(existing));
        }
        let registered = list_types_in(tx).await?;
        if let Some(overlapping) = support::overlapping(&registered, &def.template) {
            return Ok(RegisterResult::Conflict {
                existing: overlapping.clone(),
                reason: support::overlap_reason(&def.template, &overlapping.template),
            });
        }
        sqlx::query("INSERT INTO actor_types (template, shape, contract, contract_hash) VALUES ($1, $2, $3, $4)")
            .bind(def.template.as_str())
            .bind(def.template.shape())
            .bind(&def.contract)
            .bind(&def.contract_hash)
            .execute(&mut *tx.c)
            .await?;
        Ok(RegisterResult::Registered(def.clone()))
    }
}

// ---- transactions ----

/// One transaction: its connection, its clock reading and the wake-ups to send at commit.
struct Tx {
    c: PoolConnection<Postgres>,
    now: Option<Timestamp>,
    notify: Vec<String>,
    retention: Duration,
    /// A transaction is open on `c` that was neither committed nor rolled back yet.
    open: bool,
}

impl Tx {
    /// Store time for this transaction: `clock_timestamp()`, read on first call and fixed afterwards. Writers call it
    /// after taking their row locks.
    async fn now(&mut self) -> Result<Timestamp, TxError> {
        if let Some(now) = self.now {
            return Ok(now);
        }
        let now: Timestamp = sqlx::query_scalar("SELECT clock_timestamp()").fetch_one(&mut *self.c).await?;
        let now = time::truncate(now);
        self.now = Some(now);
        Ok(now)
    }

    /// Retention cutoff: outcomes that finished at or before it are expired.
    async fn cutoff(&mut self) -> Result<Timestamp, TxError> {
        Ok(time::sub(self.now().await?, self.retention))
    }

    /// Sends a wake-up for `template` when the transaction commits.
    fn notify(&mut self, template: String) {
        if !self.notify.contains(&template) {
            self.notify.push(template);
        }
    }
}

/// How a transaction ends: committed, or rolled back while still returning a result.
enum Done<T> {
    Commit(T),
    Rollback(T),
}

/// An error inside a transaction: from the database (maybe transient), or decided by the store.
enum TxError {
    Sql(sqlx::Error),
    Store(StoreError),
    /// The socket timeout elapsed.
    TimedOut,
}

impl TxError {
    fn into_store_error(self, socket_timeout: Duration) -> StoreError {
        match self {
            TxError::Sql(e) => map_error(e),
            TxError::Store(e) => e,
            TxError::TimedOut => {
                StoreError::unavailable(format!("A transaction took longer than the socket timeout of {socket_timeout:?}"))
            }
        }
    }
}

impl From<sqlx::Error> for TxError {
    fn from(e: sqlx::Error) -> Self {
        TxError::Sql(e)
    }
}

impl From<StoreError> for TxError {
    fn from(e: StoreError) -> Self {
        TxError::Store(e)
    }
}

type TxResult<T> = Result<Done<T>, TxError>;

fn invalid(message: impl Into<String>) -> TxError {
    TxError::Store(StoreError::Invalid(message.into()))
}

fn corrupt(message: impl Into<String>) -> TxError {
    TxError::Store(StoreError::Corrupt(message.into()))
}

/// A pseudo-random number in `0..bound` (for retry jitter).
fn random_below(bound: u64) -> u64 {
    if bound == 0 {
        return 0;
    }
    RandomState::new().build_hasher().finish() % bound
}

/// An exclusive upper bound for strings starting with `prefix` under the `"C"` collation, so prefix filters can use a
/// B-tree range. Exact for canonical URIs, which are ASCII; `starts_with` keeps the filter exact anyway.
fn upper_bound(prefix: &str) -> String {
    format!("{prefix}{}", char::MAX)
}

// ---- SQL fragments ----
// `VISIBLE`: the outcome row is visible to reads (parameters: cutoff, now).
// `RETAINED`: the outcome row is within the dedup window or still visible (parameters: cutoff, now).

macro_rules! visible {
    ($cutoff:literal, $now:literal) => {
        concat!("(CASE WHEN visible_until IS NULL THEN finished_at > $", $cutoff, " ELSE visible_until > $", $now, " END)")
    };
}

macro_rules! retained {
    ($cutoff:literal, $now:literal) => {
        concat!("(finished_at > $", $cutoff, " OR visible_until > $", $now, ")")
    };
}

// ---- mapping ----

fn address(text: String) -> Result<ActorAddress, TxError> {
    ActorAddress::parse(&text).map_err(|e| corrupt(format!("stored address '{text}' is invalid: {e}")))
}

fn template(text: String) -> Result<UriTemplate, TxError> {
    UriTemplate::parse(&text).map_err(|e| corrupt(format!("stored template '{text}' is invalid: {e}")))
}

fn message_id(text: String) -> Result<MessageId, TxError> {
    MessageId::new(text).map_err(corrupt)
}

fn type_def(row: &PgRow) -> Result<TypeDef, TxError> {
    Ok(TypeDef {
        template: template(row.try_get("template")?)?,
        contract: row.try_get("contract")?,
        contract_hash: row.try_get("contract_hash")?,
    })
}

fn message(row: &PgRow) -> Result<Message, TxError> {
    let id: String = row.try_get("id")?;
    let source = format!("message {id}");
    Ok(Message {
        to: address(row.try_get("actor_uri")?)?,
        from: row.try_get::<Option<String>, _>("sender")?.map(address).transpose()?,
        message_type: row.try_get("type")?,
        payload: row.try_get("payload")?,
        deliver_at: row.try_get("deliver_at")?,
        correlation_id: row.try_get("correlation_id")?,
        headers: support::headers_from_json(&row.try_get::<Value, _>("headers")?)?,
        seq: row.try_get("seq")?,
        attempt: row.try_get::<i32, _>("attempt")? as u32,
        failures: row.try_get::<i32, _>("failures")? as u32,
        last_error: row.try_get("last_error")?,
        // Tolerant: a policy this version cannot decode must not break claims; it reads as no policy.
        retry: row.try_get::<Option<Value>, _>("retry_policy")?.and_then(|v| RetryPolicy::from_stored(&v, &source)),
        caused_by: row.try_get::<Option<String>, _>("caused_by")?.map(message_id).transpose()?,
        retention: row
            .try_get::<Option<Value>, _>("retention_policy")?
            .and_then(|v| Retention::from_stored(&v, &source)),
        id: message_id(id)?,
    })
}

fn outcome(row: &PgRow) -> Result<OutcomeRecord, TxError> {
    let status: String = row.try_get("status")?;
    Ok(OutcomeRecord {
        message_id: message_id(row.try_get("message_id")?)?,
        actor: address(row.try_get("actor_uri")?)?,
        status: OutcomeStatus::from_wire_name(&status).ok_or_else(|| corrupt(format!("unknown outcome status '{status}'")))?,
        error: row.try_get("error")?,
        finished_at: row.try_get("finished_at")?,
        envelope: support::envelope_from_json(&row.try_get::<Value, _>("envelope")?)?,
    })
}

fn actor_record(row: &PgRow) -> Result<ActorRecord, TxError> {
    Ok(ActorRecord {
        address: address(row.try_get("uri")?)?,
        template: template(row.try_get("type")?)?,
        state: row.try_get("state")?,
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
    })
}

// ---- statements shared by several operations ----

async fn type_def_in(tx: &mut Tx, t: &UriTemplate) -> Result<Option<TypeDef>, TxError> {
    let row = sqlx::query("SELECT template, contract, contract_hash FROM actor_types WHERE template = $1")
        .bind(t.as_str())
        .fetch_optional(&mut *tx.c)
        .await?;
    row.as_ref().map(type_def).transpose()
}

async fn list_types_in(tx: &mut Tx) -> Result<Vec<TypeDef>, TxError> {
    let rows = sqlx::query("SELECT template, contract, contract_hash FROM actor_types ORDER BY template")
        .fetch_all(&mut *tx.c)
        .await?;
    rows.iter().map(type_def).collect()
}

async fn get_in(tx: &mut Tx, address: &str) -> Result<Option<ActorRecord>, TxError> {
    let row = sqlx::query("SELECT uri, type, state, version, created_at FROM actors WHERE uri = $1")
        .bind(address)
        .fetch_optional(&mut *tx.c)
        .await?;
    row.as_ref().map(actor_record).transpose()
}

/// Locks the instance's row for the rest of the transaction and returns its type, or `None` when absent.
async fn lock_actor(tx: &mut Tx, uri: &str) -> Result<Option<String>, TxError> {
    Ok(sqlx::query_scalar("SELECT type FROM actors WHERE uri = $1 FOR UPDATE").bind(uri).fetch_optional(&mut *tx.c).await?)
}

async fn create_in(tx: &mut Tx, address: &ActorAddress, t: &UriTemplate, state: &Option<Value>) -> Result<CreateResult, TxError> {
    loop {
        let now = tx.now().await?;
        let inserted = sqlx::query(
            "INSERT INTO actors (uri, type, state, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (uri) DO NOTHING",
        )
        .bind(address.as_str())
        .bind(t.as_str())
        .bind(state)
        .bind(now)
        .execute(&mut *tx.c)
        .await?;
        if inserted.rows_affected() == 1 {
            return Ok(CreateResult::Created);
        }
        // The conflicting row is committed now (READ COMMITTED: the next statement sees it) unless it was deleted again
        // in the meantime; then try again.
        if let Some(existing) = get_in(tx, address.as_str()).await? {
            return Ok(CreateResult::Exists(existing));
        }
    }
}

/// Creates `create`'s instance unless it exists (idempotent) and locks its row; returns its type. Runs before the
/// transaction reads its clock, so `created_at` is taken from `clock_timestamp()` directly.
async fn create_and_lock(tx: &mut Tx, create: &ActorCreate) -> Result<String, TxError> {
    loop {
        sqlx::query(
            "INSERT INTO actors (uri, type, state, created_at) VALUES ($1, $2, $3, clock_timestamp()) \
             ON CONFLICT (uri) DO NOTHING",
        )
        .bind(create.address.as_str())
        .bind(create.template.as_str())
        .bind(&create.state)
        .execute(&mut *tx.c)
        .await?;
        // Our own row, or the conflicting one once its transaction committed; absent only if it was deleted again.
        if let Some(t) = lock_actor(tx, create.address.as_str()).await? {
            return Ok(t);
        }
    }
}

async fn is_id_in_use(tx: &mut Tx, id: &MessageId) -> Result<bool, TxError> {
    let pending: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM messages WHERE id = $1").bind(id.as_str()).fetch_optional(&mut *tx.c).await?;
    if pending.is_some() {
        return Ok(true);
    }
    let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
    let finished: Option<i32> = sqlx::query_scalar(concat!("SELECT 1 FROM outcomes WHERE message_id = $1 AND ", retained!(2, 3)))
        .bind(id.as_str())
        .bind(cutoff)
        .bind(now)
        .fetch_optional(&mut *tx.c)
        .await?;
    Ok(finished.is_some())
}

/// Inserts `message` into the mailbox of its target, whose row the caller has locked. Returns `None` when the id is in
/// use; the caller must then roll back, because the message row may already be written.
async fn insert(tx: &mut Tx, message: &OutgoingMessage) -> Result<Option<Message>, TxError> {
    let now = tx.now().await?;
    let deliver_at = message.delivery.resolve(now).map_err(|e| invalid(e.0))?;
    let seq: Option<i64> = sqlx::query_scalar(
        "INSERT INTO messages
            (id, actor_uri, seq, type, payload, sender, correlation_id, headers, deliver_at, attempt, retry_policy,
             caused_by, retention_policy)
         VALUES ($1, $2, nextval('message_seq'), $3, $4, $5, $6, $7, $8, 0, $9, $10, $11)
         ON CONFLICT (id) DO NOTHING
         RETURNING seq",
    )
    .bind(message.id.as_str())
    .bind(message.to.as_str())
    .bind(&message.message_type)
    .bind(&message.payload)
    .bind(message.from.as_ref().map(|a| a.as_str()))
    .bind(&message.correlation_id)
    .bind(support::headers_to_json(&message.headers))
    .bind(deliver_at)
    .bind(message.retry.as_ref().map(RetryPolicy::to_json))
    .bind(message.caused_by.as_ref().map(|id| id.as_str()))
    .bind(message.retention.as_ref().map(Retention::to_json))
    .fetch_optional(&mut *tx.c)
    .await?;
    let Some(seq) = seq else { return Ok(None) };
    // Checked after the insert: a concurrent commit or cancel that moved this id to `outcomes` has finished by now.
    let cutoff = tx.cutoff().await?;
    let in_use: Option<Option<bool>> =
        sqlx::query_scalar(concat!("SELECT ", retained!(1, 2), " FROM outcomes WHERE message_id = $3 FOR UPDATE"))
            .bind(cutoff)
            .bind(now)
            .bind(message.id.as_str())
            .fetch_optional(&mut *tx.c)
            .await?;
    if let Some(in_use) = in_use {
        if in_use == Some(true) {
            return Ok(None);
        }
        // expired, not yet swept
        sqlx::query("DELETE FROM outcomes WHERE message_id = $1").bind(message.id.as_str()).execute(&mut *tx.c).await?;
    }
    sqlx::query("UPDATE actors SET next_due_at = LEAST(next_due_at, $1) WHERE uri = $2")
        .bind(deliver_at)
        .bind(message.to.as_str())
        .execute(&mut *tx.c)
        .await?;
    Ok(Some(Message {
        id: message.id.clone(),
        to: message.to.clone(),
        from: message.from.clone(),
        message_type: message.message_type.clone(),
        payload: message.payload.clone(),
        deliver_at,
        correlation_id: message.correlation_id.clone(),
        headers: message.headers.clone(),
        seq,
        attempt: 0,
        failures: 0,
        last_error: None,
        retry: message.retry.clone(),
        caused_by: message.caused_by.clone(),
        retention: message.retention.clone(),
    }))
}

async fn refresh_next_due(tx: &mut Tx, uri: &str) -> Result<(), TxError> {
    sqlx::query("UPDATE actors SET next_due_at = (SELECT min(deliver_at) FROM messages WHERE actor_uri = $1) WHERE uri = $1")
        .bind(uri)
        .execute(&mut *tx.c)
        .await?;
    Ok(())
}

async fn finish(tx: &mut Tx, message: &Message, status: OutcomeStatus, error: Option<&str>) -> Result<OutcomeRecord, TxError> {
    let now = tx.now().await?;
    let cutoff = tx.cutoff().await?;
    sqlx::query("DELETE FROM messages WHERE id = $1").bind(message.id.as_str()).execute(&mut *tx.c).await?;
    let removed = support::removed_at_once(message, status);
    let keep_last = support::keep_last(message, status);
    let envelope = if removed { support::envelope_to_json(&support::stripped(message)) } else { support::envelope_to_json(message) };
    sqlx::query(
        "INSERT INTO outcomes
            (message_id, actor_uri, status, error, finished_at, envelope, caused_by, visible_until, keep_last, keep_seq,
             stripped)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, CASE WHEN $9 THEN nextval('message_seq') END, $10)
         ON CONFLICT (message_id) DO UPDATE SET actor_uri = excluded.actor_uri, status = excluded.status,
            error = excluded.error, finished_at = excluded.finished_at, envelope = excluded.envelope,
            caused_by = excluded.caused_by, visible_until = excluded.visible_until, keep_last = excluded.keep_last,
            keep_seq = excluded.keep_seq, stripped = excluded.stripped",
    )
    .bind(message.id.as_str())
    .bind(message.to.as_str())
    .bind(status.wire_name())
    .bind(if removed { None } else { error })
    .bind(now)
    .bind(envelope)
    .bind(message.caused_by.as_ref().map(|id| id.as_str()))
    .bind(support::visible_until(message, status, now))
    .bind(keep_last.is_some())
    .bind(removed)
    .execute(&mut *tx.c)
    .await?;
    if let Some(count) = keep_last {
        // Hide this instance's visible keep-last outcomes of this status beyond the newest `count`. The caller holds
        // the instance's row lock, so trims of one instance do not interleave.
        sqlx::query(concat!(
            "UPDATE outcomes SET visible_until = $1
             WHERE message_id IN (
                 SELECT message_id FROM outcomes
                 WHERE actor_uri = $2 AND status = $3 AND keep_last AND ",
            visible!(4, 1),
            " ORDER BY keep_seq DESC OFFSET $5
             )"
        ))
        .bind(now)
        .bind(message.to.as_str())
        .bind(status.wire_name())
        .bind(cutoff)
        .bind(count as i64)
        .execute(&mut *tx.c)
        .await?;
    }
    Ok(OutcomeRecord {
        message_id: message.id.clone(),
        actor: message.to.clone(),
        status,
        error: error.map(str::to_owned),
        finished_at: now,
        envelope: message.clone(),
    })
}

async fn purge_mailbox(tx: &mut Tx, uri: &str) -> Result<usize, TxError> {
    let rows =
        sqlx::query("SELECT * FROM messages WHERE actor_uri = $1 ORDER BY deliver_at, seq").bind(uri).fetch_all(&mut *tx.c).await?;
    let pending = rows.iter().map(message).collect::<Result<Vec<_>, _>>()?;
    for m in &pending {
        finish(tx, m, OutcomeStatus::Purged, None).await?;
    }
    Ok(pending.len())
}

async fn live_outcome(tx: &mut Tx, id: &MessageId) -> Result<Option<OutcomeRecord>, TxError> {
    let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
    let row = sqlx::query(concat!("SELECT * FROM outcomes WHERE message_id = $1 AND ", visible!(2, 3)))
        .bind(id.as_str())
        .bind(cutoff)
        .bind(now)
        .fetch_optional(&mut *tx.c)
        .await?;
    row.as_ref().map(outcome).transpose()
}

async fn status_in(tx: &mut Tx, id: &MessageId) -> Result<Option<MessageStatus>, TxError> {
    let now = tx.now().await?;
    let row = sqlx::query(
        "SELECT m.*, a.lease_owner, a.lease_until FROM messages m
         LEFT JOIN actors a ON a.uri = m.actor_uri AND a.lease_message = m.id AND a.lease_owner IS NOT NULL
             AND a.lease_until > $1
         WHERE m.id = $2",
    )
    .bind(now)
    .bind(id.as_str())
    .fetch_optional(&mut *tx.c)
    .await?;
    if let Some(row) = row {
        let message = message(&row)?;
        let owner: Option<String> = row.try_get("lease_owner")?;
        let until: Option<Timestamp> = row.try_get("lease_until")?;
        return Ok(Some(match (owner, until) {
            (Some(owner), Some(lease_until)) => {
                MessageStatus::Running { message, node: NodeId::new(owner).map_err(corrupt)?, lease_until }
            }
            _ => MessageStatus::Pending(message),
        }));
    }
    Ok(live_outcome(tx, id).await?.map(MessageStatus::Finished))
}

async fn chain_entry_in(tx: &mut Tx, id: &MessageId) -> Result<Option<ChainEntry>, TxError> {
    if let Some(status) = status_in(tx, id).await? {
        return Ok(Some(ChainEntry::Live(status)));
    }
    let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
    let row = sqlx::query(concat!("SELECT * FROM outcomes WHERE message_id = $1 AND ", retained!(2, 3)))
        .bind(id.as_str())
        .bind(cutoff)
        .bind(now)
        .fetch_optional(&mut *tx.c)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let removed = outcome(&row)?;
    Ok(Some(ChainEntry::Removed {
        status: removed.status,
        finished_at: removed.finished_at,
        envelope: support::stripped(&removed.envelope),
    }))
}

fn page_of<T>(mut rows: Vec<T>, limit: usize, cursor: impl Fn(&T) -> PageCursor) -> Page<T> {
    if rows.len() > limit {
        rows.truncate(limit);
        let next = rows.last().map(cursor);
        Page::new(rows, next)
    } else {
        Page::new(rows, None)
    }
}

fn commit<T>(value: T) -> TxResult<T> {
    Ok(Done::Commit(value))
}

#[async_trait]
impl Store for PostgresStore {
    fn retention(&self) -> Duration {
        self.retention
    }

    async fn now(&self) -> StoreResult<Timestamp> {
        transaction!(self, true, |tx| -> Timestamp { commit(tx.now().await?) })
    }

    // ---- type registry ----

    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult> {
        transaction!(self, false, |tx| -> RegisterResult { commit(Self::register_in(tx, &def).await?) })
    }

    async fn type_def(&self, template: &UriTemplate) -> StoreResult<Option<TypeDef>> {
        transaction!(self, true, |tx| -> Option<TypeDef> { commit(type_def_in(tx, template).await?) })
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        transaction!(self, true, |tx| -> Vec<TypeDef> { commit(list_types_in(tx).await?) })
    }

    // ---- state store ----

    async fn create(&self, address: &ActorAddress, template: &UriTemplate, state: Option<Value>) -> StoreResult<CreateResult> {
        transaction!(self, false, |tx| -> CreateResult { commit(create_in(tx, address, template, &state).await?) })
    }

    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        transaction!(self, true, |tx| -> Option<ActorRecord> { commit(get_in(tx, address.as_str()).await?) })
    }

    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        check_limit(limit)?;
        let lower_bound = cursor.map(|c| c.0.as_str()).filter(|c| *c >= prefix).unwrap_or(prefix);
        let inclusive = cursor.is_none_or(|c| c.0.as_str() < prefix);
        let sql = if inclusive {
            "SELECT uri FROM actors WHERE uri >= $1 AND uri < $2 AND starts_with(uri, $3) ORDER BY uri LIMIT $4"
        } else {
            "SELECT uri FROM actors WHERE uri > $1 AND uri < $2 AND starts_with(uri, $3) ORDER BY uri LIMIT $4"
        };
        transaction!(self, true, |tx| -> Page<ActorAddress> {
            let rows: Vec<String> = sqlx::query_scalar(sql)
                .bind(lower_bound)
                .bind(upper_bound(prefix))
                .bind(prefix)
                .bind(limit as i64 + 1)
                .fetch_all(&mut *tx.c)
                .await?;
            let addresses = rows.into_iter().map(address).collect::<Result<Vec<_>, _>>()?;
            commit(page_of(addresses, limit, |a| PageCursor(a.to_string())))
        })
    }

    async fn delete(&self, address: &ActorAddress) -> StoreResult<DeleteResult> {
        transaction!(self, false, |tx| -> DeleteResult {
            if lock_actor(tx, address.as_str()).await?.is_none() {
                return commit(DeleteResult::Absent);
            }
            let purged = purge_mailbox(tx, address.as_str()).await?;
            sqlx::query("DELETE FROM actors WHERE uri = $1").bind(address.as_str()).execute(&mut *tx.c).await?;
            commit(DeleteResult::Deleted { purged })
        })
    }

    // ---- mailbox ----

    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult> {
        transaction!(self, false, |tx| -> EnqueueResult {
            let Some(t) = lock_actor(tx, message.to.as_str()).await? else {
                return commit(if is_id_in_use(tx, &message.id).await? {
                    EnqueueResult::Duplicate(message.id.clone())
                } else {
                    EnqueueResult::NoSuchInstance
                });
            };
            let Some(stored) = insert(tx, &message).await? else {
                return Ok(Done::Rollback(EnqueueResult::Duplicate(message.id.clone())));
            };
            tx.notify(t);
            commit(EnqueueResult::Enqueued(stored))
        })
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
        let names: Vec<String> = types.iter().map(|t| t.as_str().to_owned()).collect();
        transaction!(self, false, |tx| -> Vec<Claim> {
            // SKIP LOCKED never waits, so reading the clock first is safe here.
            let now = tx.now().await?;
            let candidates = sqlx::query(
                "SELECT uri, type, state, version FROM actors
                 WHERE next_due_at <= $1 AND type = ANY ($2) AND (lease_owner IS NULL OR lease_until <= $1)
                 ORDER BY next_due_at
                 LIMIT $3
                 FOR UPDATE SKIP LOCKED",
            )
            .bind(now)
            .bind(&names)
            .bind(limit as i64)
            .fetch_all(&mut *tx.c)
            .await?;
            let lease_until = time::truncate(time::add(now, ttl).map_err(|e| invalid(e.0))?);
            let mut claims = Vec::with_capacity(candidates.len());
            for candidate in &candidates {
                let uri: String = candidate.try_get("uri")?;
                let head = sqlx::query("SELECT * FROM messages WHERE actor_uri = $1 ORDER BY deliver_at, seq LIMIT 1")
                    .bind(&uri)
                    .fetch_optional(&mut *tx.c)
                    .await?;
                let head = head.as_ref().map(message).transpose()?;
                let Some(head) = head.filter(|h| h.deliver_at <= now) else {
                    return Err(corrupt(format!("next_due_at out of sync for {uri}")));
                };
                let claimed = Message { attempt: head.attempt + 1, ..head };
                sqlx::query("UPDATE messages SET attempt = $1 WHERE id = $2")
                    .bind(claimed.attempt as i32)
                    .bind(claimed.id.as_str())
                    .execute(&mut *tx.c)
                    .await?;
                let epoch: i64 = sqlx::query_scalar(
                    "UPDATE actors SET epoch = nextval('epoch_seq'), lease_owner = $1, lease_until = $2, lease_message = $3
                     WHERE uri = $4 RETURNING epoch",
                )
                .bind(node.as_str())
                .bind(lease_until)
                .bind(claimed.id.as_str())
                .bind(&uri)
                .fetch_one(&mut *tx.c)
                .await?;
                claims.push(Claim {
                    actor: address(uri)?,
                    template: template(candidate.try_get("type")?)?,
                    epoch,
                    message: claimed,
                    state: candidate.try_get("state")?,
                    version: candidate.try_get("version")?,
                    claimed_at: now,
                    lease_until,
                });
            }
            commit(claims)
        })
    }

    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult> {
        transaction!(self, false, |tx| -> RenewResult {
            let held: Option<i32> =
                sqlx::query_scalar("SELECT 1 FROM actors WHERE uri = $1 AND epoch = $2 AND lease_owner IS NOT NULL FOR UPDATE")
                    .bind(address.as_str())
                    .bind(epoch)
                    .fetch_optional(&mut *tx.c)
                    .await?;
            if held.is_none() {
                return commit(RenewResult::LeaseLost);
            }
            let lease_until = time::truncate(time::add(tx.now().await?, ttl).map_err(|e| invalid(e.0))?);
            sqlx::query("UPDATE actors SET lease_until = $1 WHERE uri = $2")
                .bind(lease_until)
                .bind(address.as_str())
                .execute(&mut *tx.c)
                .await?;
            commit(RenewResult::Renewed(lease_until))
        })
    }

    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        transaction!(self, false, |tx| -> () {
            let t: Option<String> = sqlx::query_scalar(
                "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL
                 WHERE uri = $1 AND epoch = $2 AND lease_owner IS NOT NULL
                 RETURNING type",
            )
            .bind(address.as_str())
            .bind(epoch)
            .fetch_optional(&mut *tx.c)
            .await?;
            if let Some(t) = t {
                tx.notify(t);
            }
            commit(())
        })
    }

    async fn next_due_at(&self, types: &BTreeSet<UriTemplate>) -> StoreResult<Option<Timestamp>> {
        if types.is_empty() {
            return Ok(None);
        }
        let names: Vec<String> = types.iter().map(|t| t.as_str().to_owned()).collect();
        transaction!(self, true, |tx| -> Option<Timestamp> {
            let now = tx.now().await?;
            let at: Option<Timestamp> = sqlx::query_scalar(
                "SELECT min(CASE WHEN lease_owner IS NOT NULL AND lease_until > $1 THEN greatest(next_due_at, lease_until)
                                 ELSE next_due_at END)
                 FROM actors WHERE next_due_at IS NOT NULL AND type = ANY ($2)",
            )
            .bind(now)
            .bind(&names)
            .fetch_one(&mut *tx.c)
            .await?;
            commit(at)
        })
    }

    async fn pending(&self, address: &ActorAddress, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<Message>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_mailbox_cursor).transpose()?;
        transaction!(self, true, |tx| -> Page<Message> {
            let rows = match after {
                None => {
                    sqlx::query("SELECT * FROM messages WHERE actor_uri = $1 ORDER BY deliver_at, seq LIMIT $2")
                        .bind(address.as_str())
                        .bind(limit as i64 + 1)
                        .fetch_all(&mut *tx.c)
                        .await?
                }
                Some((at, seq)) => {
                    sqlx::query(
                        "SELECT * FROM messages WHERE actor_uri = $1 AND (deliver_at, seq) > ($2, $3)
                         ORDER BY deliver_at, seq LIMIT $4",
                    )
                    .bind(address.as_str())
                    .bind(time::from_micros(at))
                    .bind(seq)
                    .bind(limit as i64 + 1)
                    .fetch_all(&mut *tx.c)
                    .await?
                }
            };
            let messages = rows.iter().map(message).collect::<Result<Vec<_>, _>>()?;
            commit(page_of(messages, limit, support::mailbox_cursor))
        })
    }

    async fn list_pending(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<Message>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_pending_cursor).transpose()?;
        transaction!(self, true, |tx| -> Page<Message> {
            // A range on actor_uri walks the mailbox index (actor_uri, deliver_at, seq) in order; no sort.
            let rows = match &after {
                None => {
                    sqlx::query(
                        "SELECT * FROM messages
                         WHERE actor_uri >= $1 AND actor_uri < $2 AND starts_with(actor_uri, $3)
                         ORDER BY actor_uri, deliver_at, seq LIMIT $4",
                    )
                    .bind(prefix)
                    .bind(upper_bound(prefix))
                    .bind(prefix)
                    .bind(limit as i64 + 1)
                    .fetch_all(&mut *tx.c)
                    .await?
                }
                Some((uri, at, seq)) => {
                    sqlx::query(
                        "SELECT * FROM messages
                         WHERE actor_uri >= $1 AND actor_uri < $2 AND starts_with(actor_uri, $3)
                           AND (actor_uri, deliver_at, seq) > ($4, $5, $6)
                         ORDER BY actor_uri, deliver_at, seq LIMIT $7",
                    )
                    .bind(prefix)
                    .bind(upper_bound(prefix))
                    .bind(prefix)
                    .bind(uri)
                    .bind(time::from_micros(*at))
                    .bind(*seq)
                    .bind(limit as i64 + 1)
                    .fetch_all(&mut *tx.c)
                    .await?
                }
            };
            let messages = rows.iter().map(message).collect::<Result<Vec<_>, _>>()?;
            commit(page_of(messages, limit, support::pending_cursor))
        })
    }

    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        transaction!(self, false, |tx| -> CancelResult {
            let uri: Option<String> =
                sqlx::query_scalar("SELECT actor_uri FROM messages WHERE id = $1").bind(id.as_str()).fetch_optional(&mut *tx.c).await?;
            let t = match &uri {
                Some(uri) => lock_actor(tx, uri).await?,
                None => None,
            };
            // Read again under the instance lock: the message may have left the mailbox in the meantime.
            let row = match t {
                Some(_) => sqlx::query("SELECT * FROM messages WHERE id = $1").bind(id.as_str()).fetch_optional(&mut *tx.c).await?,
                None => None,
            };
            let (Some(t), Some(row)) = (t, row) else {
                return commit(match live_outcome(tx, id).await? {
                    Some(o) => CancelResult::AlreadyFinished(o),
                    None => CancelResult::NotFound,
                });
            };
            let message = message(&row)?;
            let lease: Option<Option<Timestamp>> = sqlx::query_scalar(
                "SELECT lease_until FROM actors WHERE uri = $1 AND lease_message = $2 AND lease_owner IS NOT NULL",
            )
            .bind(message.to.as_str())
            .bind(id.as_str())
            .fetch_optional(&mut *tx.c)
            .await?;
            if let Some(lease_until) = lease {
                let now = tx.now().await?;
                if lease_until.is_some_and(|until| until > now) {
                    return commit(CancelResult::Running);
                }
                // Void the expired lease so that the stale turn cannot commit.
                sqlx::query("UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE uri = $1")
                    .bind(message.to.as_str())
                    .execute(&mut *tx.c)
                    .await?;
            }
            let outcome = finish(tx, &message, OutcomeStatus::Cancelled, None).await?;
            refresh_next_due(tx, message.to.as_str()).await?;
            if lease.is_some() {
                tx.notify(t);
            }
            commit(CancelResult::Cancelled(outcome))
        })
    }

    async fn outcome(&self, id: &MessageId) -> StoreResult<Option<OutcomeRecord>> {
        transaction!(self, true, |tx| -> Option<OutcomeRecord> { commit(live_outcome(tx, id).await?) })
    }

    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        transaction!(self, true, |tx| -> Option<MessageStatus> { commit(status_in(tx, id).await?) })
    }

    async fn failures(
        &self,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_failure_cursor).transpose()?;
        let since = time::truncate(since);
        transaction!(self, true, |tx| -> Page<OutcomeRecord> {
            let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
            let base = concat!(
                "SELECT * FROM outcomes
                 WHERE status = $1 AND finished_at >= $2 AND ",
                visible!(3, 4),
                " AND actor_uri >= $5 AND actor_uri < $6 AND starts_with(actor_uri, $7)"
            );
            let query = match &after {
                None => sqlx::query(AssertSqlSafe(format!("{base} ORDER BY finished_at, message_id LIMIT $8"))),
                Some(_) => sqlx::query(AssertSqlSafe(format!(
                    "{base} AND (finished_at, message_id) > ($8, $9) ORDER BY finished_at, message_id LIMIT $10"
                ))),
            };
            let mut query = query
                .bind(OutcomeStatus::Failed.wire_name())
                .bind(since)
                .bind(cutoff)
                .bind(now)
                .bind(prefix)
                .bind(upper_bound(prefix))
                .bind(prefix);
            if let Some((at, id)) = &after {
                query = query.bind(time::from_micros(*at)).bind(id.clone());
            }
            let rows = query.bind(limit as i64 + 1).fetch_all(&mut *tx.c).await?;
            let outcomes = rows.iter().map(outcome).collect::<Result<Vec<_>, _>>()?;
            commit(page_of(outcomes, limit, |o| support::failure_cursor(o.finished_at, &o.message_id)))
        })
    }

    async fn chain_entries(&self, ids: &[MessageId]) -> StoreResult<Vec<ChainEntry>> {
        let mut seen = HashSet::new();
        let ids: Vec<&MessageId> = ids.iter().filter(|id| seen.insert(*id)).collect();
        transaction!(self, true, |tx| -> Vec<ChainEntry> {
            let mut entries = Vec::with_capacity(ids.len());
            for id in &ids {
                if let Some(entry) = chain_entry_in(tx, id).await? {
                    entries.push(entry);
                }
            }
            commit(entries)
        })
    }

    async fn chain_children(&self, parents: &[MessageId], limit: usize) -> StoreResult<Vec<ChainEntry>> {
        check_limit(limit)?;
        let mut seen = HashSet::new();
        let wanted: Vec<String> = parents.iter().filter(|p| seen.insert(*p)).map(|p| p.as_str().to_owned()).collect();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        transaction!(self, true, |tx| -> Vec<ChainEntry> {
            let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
            let ids: Vec<String> = sqlx::query_scalar(concat!(
                "SELECT id FROM messages WHERE caused_by = ANY ($1)
                 UNION ALL
                 SELECT message_id FROM outcomes WHERE caused_by = ANY ($1) AND ",
                retained!(2, 3),
                " LIMIT $4"
            ))
            .bind(&wanted)
            .bind(cutoff)
            .bind(now)
            .bind(limit as i64)
            .fetch_all(&mut *tx.c)
            .await?;
            let mut entries = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(entry) = chain_entry_in(tx, &message_id(id)?).await? {
                    entries.push(entry);
                }
            }
            commit(entries)
        })
    }

    async fn sweep_expired(&self) -> StoreResult<u64> {
        transaction!(self, false, |tx| -> u64 {
            let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
            // Hidden records lose their content (the minimal record stays for dedup and chains).
            sqlx::query(
                "UPDATE outcomes SET error = NULL, stripped = true,
                     envelope = envelope || jsonb_build_object('payload', 'null'::jsonb, 'headers', '{}'::jsonb,
                                                               'correlationId', NULL, 'lastError', NULL)
                 WHERE NOT stripped AND visible_until IS NOT NULL AND visible_until <= $1",
            )
            .bind(now)
            .execute(&mut *tx.c)
            .await?;
            let removed = sqlx::query("DELETE FROM outcomes WHERE finished_at <= $1 AND (visible_until IS NULL OR visible_until <= $2)")
                .bind(cutoff)
                .bind(now)
                .execute(&mut *tx.c)
                .await?;
            commit(removed.rows_affected())
        })
    }

    // ---- commit ----

    async fn commit(&self, request: CommitRequest) -> StoreResult<CommitResult> {
        let r = &request;
        transaction!(self, false, |tx| -> CommitResult {
            if r.cancels.contains(&r.message_id) {
                return Err(invalid(format!("A turn cannot cancel its own message {}", r.message_id)));
            }
            // Lock every instance this commit touches, in URI order, so that concurrent commits cannot deadlock.
            let touched: BTreeSet<&str> = std::iter::once(r.address.as_str())
                .chain(r.creates.iter().map(|c| c.address.as_str()))
                .chain(r.sends.iter().map(|s| s.to.as_str()))
                .collect();
            let touched: Vec<&str> = touched.into_iter().collect();
            let rows: Vec<(String, String)> =
                sqlx::query_as("SELECT uri, type FROM actors WHERE uri = ANY ($1) ORDER BY uri FOR UPDATE")
                    .bind(&touched)
                    .fetch_all(&mut *tx.c)
                    .await?;
            let mut types: HashMap<String, String> = rows.into_iter().collect();
            let claimed: Option<Option<String>> = sqlx::query_scalar(
                "SELECT lease_message FROM actors WHERE uri = $1 AND epoch = $2 AND lease_owner IS NOT NULL",
            )
            .bind(r.address.as_str())
            .bind(r.epoch)
            .fetch_optional(&mut *tx.c)
            .await?;
            let Some(Some(claimed)) = claimed else { return commit(CommitResult::LeaseLost) };
            if claimed != r.message_id.as_str() {
                return Err(invalid(format!(
                    "Message {} is not the message claimed under epoch {}",
                    r.message_id, r.epoch
                )));
            }

            // 3. creates, first of all writes: an instance that did not exist when locking is inserted and locked now,
            // *before* the clock is read. A concurrent transaction creating (and enqueueing to) the same instance is
            // thus waited for here, so the clock reading below follows it and our sends cannot sort before its
            // messages.
            let mut creates: Vec<&ActorCreate> = r.creates.iter().collect();
            creates.sort_by(|a, b| a.address.as_str().cmp(b.address.as_str()));
            for create in creates {
                if !types.contains_key(create.address.as_str()) {
                    let t = create_and_lock(tx, create).await?;
                    types.insert(create.address.as_str().to_owned(), t);
                }
            }

            // Validate before applying the rest (S2); a rejection rolls back the creates and leaves the lease as it is.
            let mut send_ids = HashSet::new();
            for send in &r.sends {
                if !types.contains_key(send.to.as_str()) {
                    return Ok(Done::Rollback(CommitResult::Rejected(format!("Send to absent instance {}", send.to))));
                }
                if !send_ids.insert(&send.id) || send.id == r.message_id || is_id_in_use(tx, &send.id).await? {
                    return Ok(Done::Rollback(CommitResult::Rejected(format!("Message id {} is already in use", send.id))));
                }
            }

            // 1. outcome, or back to pending
            let row = sqlx::query("SELECT * FROM messages WHERE id = $1")
                .bind(r.message_id.as_str())
                .fetch_optional(&mut *tx.c)
                .await?
                .ok_or_else(|| corrupt(format!("the claimed message {} is not pending", r.message_id)))?;
            let claimed_message = message(&row)?;
            match &r.outcome {
                TurnOutcome::Rescheduled { delivery, error } => {
                    let deliver_at = delivery.resolve(tx.now().await?).map_err(|e| invalid(e.0))?;
                    sqlx::query(
                        "UPDATE messages SET deliver_at = $1, seq = nextval('message_seq'), failures = failures + 1,
                             last_error = $2
                         WHERE id = $3",
                    )
                    .bind(deliver_at)
                    .bind(error)
                    .bind(r.message_id.as_str())
                    .execute(&mut *tx.c)
                    .await?;
                }
                outcome => {
                    let status = outcome.status().expect("a final outcome has a status");
                    finish(tx, &claimed_message, status, outcome.error()).await?;
                }
            }
            // 2. cancels: only this instance's pending messages, whose row lock we hold (next_due_at is refreshed below)
            let mut cancelled = Vec::new();
            let mut seen = HashSet::new();
            for id in r.cancels.iter().filter(|id| seen.insert(*id)) {
                let row = sqlx::query("SELECT * FROM messages WHERE id = $1 AND actor_uri = $2")
                    .bind(id.as_str())
                    .bind(r.address.as_str())
                    .fetch_optional(&mut *tx.c)
                    .await?;
                if let Some(row) = row {
                    finish(tx, &message(&row)?, OutcomeStatus::Cancelled, None).await?;
                    cancelled.push(id.clone());
                }
            }
            // 4. sends
            let mut sent = Vec::with_capacity(r.sends.len());
            for send in &r.sends {
                // A concurrent enqueue may have taken the id since the check above: undo everything.
                let Some(stored) = insert(tx, send).await? else {
                    return Ok(Done::Rollback(CommitResult::Rejected(format!("Message id {} is already in use", send.id))));
                };
                tx.notify(types[send.to.as_str()].clone());
                sent.push(stored);
            }
            // 5. state, 6. release
            match &r.state {
                StateChange::Unchanged => {}
                StateChange::NewState(state) => {
                    sqlx::query("UPDATE actors SET state = $1, version = version + 1 WHERE uri = $2")
                        .bind(state)
                        .bind(r.address.as_str())
                        .execute(&mut *tx.c)
                        .await?;
                }
                StateChange::Stop => {
                    purge_mailbox(tx, r.address.as_str()).await?;
                    let version: i64 = sqlx::query_scalar("DELETE FROM actors WHERE uri = $1 RETURNING version")
                        .bind(r.address.as_str())
                        .fetch_one(&mut *tx.c)
                        .await?;
                    return commit(CommitResult::Committed { version, sent, cancelled });
                }
            }
            refresh_next_due(tx, r.address.as_str()).await?;
            let (version, has_more): (i64, bool) = sqlx::query_as(
                "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE uri = $1
                 RETURNING version, next_due_at IS NOT NULL",
            )
            .bind(r.address.as_str())
            .fetch_one(&mut *tx.c)
            .await?;
            if has_more {
                tx.notify(types[r.address.as_str()].clone());
            }
            commit(CommitResult::Committed { version, sent, cancelled })
        })
    }

    // ---- wake-ups ----

    /// Yields when a `NOTIFY` for one of `types` arrives on this store's channel (from any node, including this one),
    /// and after the listener reconnected. The first call starts the shared listener connection.
    fn wake_ups(&self, types: &BTreeSet<UriTemplate>) -> Option<BoxStream<'static, ()>> {
        let names: Arc<HashSet<String>> = Arc::new(types.iter().map(|t| t.as_str().to_owned()).collect());
        let receiver = self.listener.subscribe();
        self.listener.start();
        Some(
            futures::stream::unfold(receiver, move |mut receiver| {
                let names = names.clone();
                async move {
                    loop {
                        match receiver.recv().await {
                            Ok(payload) if payload.is_empty() || names.contains(&payload) => return Some(((), receiver)),
                            Ok(_) => continue,
                            // Missed some: one of them may have been ours.
                            Err(RecvError::Lagged(_)) => return Some(((), receiver)),
                            Err(RecvError::Closed) => return None,
                        }
                    }
                }
            })
            .boxed(),
        )
    }

    /// Closes the listener and the connection pool. Idempotent.
    async fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.listener.stop().await;
        self.pool.close().await;
    }
}
