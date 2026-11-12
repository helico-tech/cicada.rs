//! [`PostgresStore`]: the multi-node store on PostgreSQL.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::address::{ActorAddress, TypeName};
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::store::support::{self, Notice, Removal, check_limit};
use crate::store::{
    ActorCreate, ActorRecord, Backend, CancelResult, Claim, CommitRequest, CommitResult, CreateResult, DeleteResult,
    DeliverNowResult, EnqueueResult, Message, MessageId, MessageStatus, NodeId, NodeRecord, OutcomeRecord,
    OutcomeStatus, OutgoingMessage, Page, PageCursor, RegisterResult, RemoveTypeResult, RenewResult, RunningTurn,
    SetWatchResult, StateChange, StoreError, StoreResult, StoreStats, TurnOutcome, TypeDef,
};
use crate::time::{self, Timestamp};
use crate::watch::{self, FailureCause, Watch, WatchTarget};
use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::Value;
use sqlx::pool::PoolConnection;
use sqlx::postgres::PgConnectOptions;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row};
use tokio::sync::broadcast::error::RecvError;

use crate::store::postgres::config::PostgresOptions;
use crate::store::postgres::error::{is_transient, map_error};
use crate::store::postgres::listener::NotificationListener;
use crate::store::postgres::schema;

/// How many times a transaction is re-run after PostgreSQL aborted it for a deadlock or a serialization failure (both
/// are transient under concurrent commits from several nodes), or after it found a watch set while it wrote an event,
/// before the error is reported.
const TRANSACTION_RETRIES: u32 = 8;
/// Number of idle connections the pool keeps open.
const MIN_IDLE: u32 = 1;
/// How long an operation waits for a pooled connection before failing.
pub(crate) const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

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

/// [`Backend`] on PostgreSQL: the multi-node reference backend. Any number of nodes (processes, hosts) may open a store on the same database and schema; the database is their only coordination.
///
/// **Transactions.** Every operation is one transaction at `READ COMMITTED` on a pooled connection. A transaction that
/// writes an instance or its mailbox first locks the instance's `actors` row (`SELECT … FOR UPDATE`), so all writes to
/// one instance are serialised (S3); `commit` locks the committing instance together with every instance it sends to,
/// creates, pauses, resumes, purges or deletes, in address order, which avoids deadlocks between commits. A commit
/// that deletes and creates one address restarts it in its row (a new life), so a transaction waiting for that row
/// finds the actor; for an address that is absent, the row it inserts to hold the lock is the create. `claim_due`
/// locks candidates with `FOR UPDATE SKIP LOCKED`, so concurrent claims from many nodes never wait for each other and
/// never share an instance (S4). Transactions PostgreSQL aborts for a deadlock or serialization failure are re-run,
/// up to 8 times.
///
/// **Trees.** A removal (`delete`, or a commit that stops or deletes) locks the instance, then its children, then
/// theirs, each level with its own statement. A child is created only in a commit that holds its parent's row lock, so
/// once a level is locked no commit can add a child below it, and the next statement sees every child committed
/// before. One recursive query would read a single snapshot and could miss a child that a commit it waited for added.
/// These locks are not in address order, so a removal can deadlock with a commit that sends up or down the tree; the
/// re-run covers that.
///
/// **Watches.** A watch changes only under the lock of its watcher's row: the watcher's own commit holds it, and a
/// change from outside takes it. A commit that ends a message `failed`, stops or deletes, and a delete, first read the
/// watchers of the actors they name and lock those rows in the same statement as the other rows they touch, in address
/// order; the watchers of the descendants of a removal are locked after the trees, in address order among themselves.
/// Then they read the watches again: a watcher whose row exists and is not locked set its watch in between, and the
/// transaction is run again; one whose row does not exist is skipped and its watches are removed. All of it comes
/// before the clock is read. A notification wakes the nodes of its watcher's type as a send does.
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
/// **Wake-ups.** The store `NOTIFY`s the channel [`PostgresOptions::effective_channel`] whenever a message becomes
/// claimable (enqueue, sends of a commit, a commit leaving messages behind, release, cancel of an expired turn,
/// deliver-now, resume of a paused actor, also by a commit) with
/// the instance's type name as payload. Notifications are transactional, so they are only delivered when the
/// write committed. [`Backend::wake_ups`] listens on one dedicated connection, started on first use and re-connected
/// with back-off when it breaks.
///
/// **JSON.** State, payloads, headers, contracts and envelopes are `jsonb`, which normalises objects (key order,
/// duplicate keys) and rejects the escape `\u0000` in strings.
///
/// **Errors.** Values PostgreSQL refuses (SQLSTATE classes `22` and `54`, e.g. a NUL character or an id too long for
/// its index) are [`StoreError::Invalid`]; stored data that cannot be decoded is [`StoreError::Corrupt`]; everything
/// else, including a closed store, is [`StoreError::Unavailable`].
///
/// The store owns its pool and listener; [`Backend::close`] releases them.
pub struct PostgresStore {
    pool: PgPool,
    retention: Duration,
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
    /// credentials as usually found in `DATABASE_URL` (`postgres://actors:secret@db:5432/actors?sslmode=require`, or
    /// `postgresql://…`), parsed by sqlx ([`PgConnectOptions`]): special characters in the user and password are
    /// percent-encoded, and only the query parameters sqlx knows have an effect (`sslmode`, `sslrootcert`,
    /// `application_name`, `options`, …).
    ///
    /// Fails with [`StoreError::Invalid`] for invalid settings or a URL that is not a PostgreSQL URL (the message never
    /// repeats the URL, which can hold a password), a database migrated by a newer version, or data that a migration's
    /// constraint refuses (the schema stays at its version), and with [`StoreError::Unavailable`] when the database
    /// cannot be reached.
    pub async fn connect(url: &str, config: PostgresOptions) -> StoreResult<Self> {
        Self::connect_with(connect_options(url)?, config).await
    }

    /// Opens a store with the connect options of the URL. The store sets `search_path` (from `config.schema`) and,
    /// when the URL sets none, the application name.
    async fn connect_with(base: PgConnectOptions, config: PostgresOptions) -> StoreResult<Self> {
        config.validate()?;
        let mut options = base.clone();
        if options.get_application_name().is_none() {
            options = options.application_name("cicada");
        }
        // The search path is a startup parameter of every connection, so it holds in every transaction.
        let options = options.options([("search_path", config.schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(config.max_pool_size)
            .min_connections(MIN_IDLE)
            .acquire_timeout(CONNECTION_TIMEOUT)
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
        let listener = NotificationListener::new(channel.clone(), base.application_name("cicada-listener"));
        Ok(Self {
            pool,
            retention: config.retention,
            socket_timeout: config.socket_timeout,
            channel,
            listener,
            closed: AtomicBool::new(false),
        })
    }

    /// True while the wake-up listener has `LISTEN` active. For tests and health checks.
    #[cfg(test)]
    pub(crate) fn is_listening(&self) -> bool {
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
            for type_name in std::mem::take(&mut tx.notify) {
                sqlx::query("SELECT pg_notify($1, $2)").bind(&self.channel).bind(type_name).execute(&mut *tx.c).await?;
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
        let transient = match error {
            TxError::Sql(e) => is_transient(e),
            TxError::Again(_) => true,
            TxError::Store(_) | TxError::TimedOut => false,
        };
        if !transient || *attempt >= TRANSACTION_RETRIES {
            return false;
        }
        *attempt += 1;
        tokio::time::sleep(Duration::from_millis(1 + random_below(5 * *attempt as u64))).await;
        true
    }

    // ---- type registry ----

    async fn register_in(tx: &mut Tx, def: &TypeDef) -> Result<RegisterResult, TxError> {
        // The common case, every node re-registering its types at start, needs no write.
        if let Some(existing) = type_def_in(tx, &def.name).await? {
            return Ok(support::register_again(existing, def));
        }
        // The primary key decides a race between nodes: the loser inserts nothing and reads the winner's row.
        let inserted = sqlx::query(
            "INSERT INTO actor_types (name, contract, contract_hash) VALUES ($1, $2, $3) ON CONFLICT (name) DO NOTHING",
        )
        .bind(def.name.as_str())
        .bind(&def.contract)
        .bind(&def.contract_hash)
        .execute(&mut *tx.c)
        .await?;
        if inserted.rows_affected() == 1 {
            return Ok(RegisterResult::Registered(def.clone()));
        }
        // READ COMMITTED: the conflicting row is visible to the next statement once its transaction committed.
        let existing = type_def_in(tx, &def.name)
            .await?
            .ok_or_else(|| corrupt(format!("type {} vanished while registering", def.name)))?;
        Ok(support::register_again(existing, def))
    }
}

/// The sqlx connect options of the PostgreSQL URL `url`. The errors never repeat the URL or sqlx's own message about
/// it: either can hold a password.
fn connect_options(url: &str) -> StoreResult<PgConnectOptions> {
    let url = url.trim();
    if !url.starts_with("postgres://") && !url.starts_with("postgresql://") {
        return Err(StoreError::Invalid("Not a PostgreSQL URL: expected postgres://… or postgresql://…".into()));
    }
    PgConnectOptions::from_str(url).map_err(|_| {
        StoreError::Invalid(
            "Malformed PostgreSQL URL: expected postgres://user:password@host:port/database?parameters".into(),
        )
    })
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

    /// Sends a wake-up for the type `type_name` when the transaction commits.
    fn notify(&mut self, type_name: String) {
        if !self.notify.contains(&type_name) {
            self.notify.push(type_name);
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
    /// The transaction must run again, as after a deadlock: it found a watch that was set while it wrote an event.
    Again(String),
}

impl TxError {
    fn into_store_error(self, socket_timeout: Duration) -> StoreError {
        match self {
            TxError::Sql(e) => map_error(e),
            TxError::Store(e) => e,
            TxError::Again(why) => {
                StoreError::unavailable(format!("{why}; the transaction ran {} times", TRANSACTION_RETRIES + 1))
            }
            TxError::TimedOut => StoreError::unavailable(format!(
                "A transaction took longer than the socket timeout of {socket_timeout:?}"
            )),
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

/// `instant` as a lower bound PostgreSQL can hold: `timestamptz` starts in 4713 BC, so earlier instants (such as
/// `DateTime::MIN_UTC` for "since ever") become 0001-01-01, before which no store time lies.
fn lower_bound(instant: Timestamp) -> Timestamp {
    const EARLIEST: Timestamp = match chrono::DateTime::from_timestamp(-62_135_596_800, 0) {
        Some(t) => t,
        None => panic!("0001-01-01 is representable"),
    };
    instant.max(EARLIEST)
}

/// An exclusive upper bound for strings starting with `prefix` under the `"C"` collation, so prefix filters can use a
/// B-tree range. Exact for canonical addresses, which are ASCII; `starts_with` keeps the filter exact anyway.
fn upper_bound(prefix: &str) -> String {
    format!("{prefix}{}", char::MAX)
}

// ---- SQL fragments ----
// `VISIBLE`: the outcome row is visible to reads (parameters: cutoff, now).
// `RETAINED`: the outcome row is within the dedup window or still visible (parameters: cutoff, now).

macro_rules! visible {
    ($cutoff:literal, $now:literal) => {
        concat!(
            "(CASE WHEN visible_until IS NULL THEN finished_at > $",
            $cutoff,
            " ELSE visible_until > $",
            $now,
            " END)"
        )
    };
}

macro_rules! retained {
    ($cutoff:literal, $now:literal) => {
        concat!("(finished_at > $", $cutoff, " OR visible_until > $", $now, ")")
    };
}

// `MESSAGE_COLUMNS`: the columns of `messages` that [`message`] reads, each after the table prefix (`""` or `"m."`).
// `OUTCOME_COLUMNS`: the columns of `outcomes` that [`outcome`] reads. Statements name their columns and never read
// with `*`, so a statement that a connection has prepared keeps working when a migration adds a column (ADR 0025).

macro_rules! message_columns {
    () => {
        message_columns!("")
    };
    ($p:literal) => {
        concat!(
            $p,
            "id, ",
            $p,
            "actor_address, ",
            $p,
            "seq, ",
            $p,
            "type, ",
            $p,
            "payload, ",
            $p,
            "sender, ",
            $p,
            "correlation_id, ",
            $p,
            "headers, ",
            $p,
            "deliver_at, ",
            $p,
            "attempt, ",
            $p,
            "failures, ",
            $p,
            "lost_claims, ",
            $p,
            "last_error, ",
            $p,
            "retry_policy, ",
            $p,
            "caused_by, ",
            $p,
            "retention_policy, ",
            $p,
            "notification"
        )
    };
}

macro_rules! outcome_columns {
    () => {
        "message_id, actor_address, status, error, finished_at, envelope"
    };
}

// ---- mapping ----

fn address(text: String) -> Result<ActorAddress, TxError> {
    ActorAddress::parse(&text).map_err(|e| corrupt(format!("stored address '{text}' is invalid: {e}")))
}

fn type_name(text: String) -> Result<TypeName, TxError> {
    TypeName::parse(&text).map_err(|e| corrupt(format!("stored type name '{text}' is invalid: {e}")))
}

fn message_id(text: String) -> Result<MessageId, TxError> {
    MessageId::new(text).map_err(|e| corrupt(e.to_string()))
}

fn type_def(row: &PgRow) -> Result<TypeDef, TxError> {
    Ok(TypeDef {
        name: type_name(row.try_get("name")?)?,
        contract: row.try_get("contract")?,
        contract_hash: row.try_get("contract_hash")?,
    })
}

fn message(row: &PgRow) -> Result<Message, TxError> {
    let id: String = row.try_get("id")?;
    let source = format!("message {id}");
    Ok(Message {
        to: address(row.try_get("actor_address")?)?,
        from: row.try_get::<Option<String>, _>("sender")?.map(address).transpose()?,
        message_type: row.try_get("type")?,
        payload: row.try_get("payload")?,
        deliver_at: row.try_get("deliver_at")?,
        correlation_id: row.try_get("correlation_id")?,
        headers: support::headers_from_json(&row.try_get::<Value, _>("headers")?)?,
        seq: row.try_get("seq")?,
        attempt: row.try_get::<i32, _>("attempt")? as u32,
        failures: row.try_get::<i32, _>("failures")? as u32,
        lost_claims: row.try_get::<i32, _>("lost_claims")? as u32,
        last_error: row.try_get("last_error")?,
        notification: row.try_get("notification")?,
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
        actor: address(row.try_get("actor_address")?)?,
        status: OutcomeStatus::from_wire_name(&status)
            .ok_or_else(|| corrupt(format!("unknown outcome status '{status}'")))?,
        error: row.try_get("error")?,
        finished_at: row.try_get("finished_at")?,
        envelope: support::envelope_from_json(&row.try_get::<Value, _>("envelope")?)?,
    })
}

fn actor_record(row: &PgRow) -> Result<ActorRecord, TxError> {
    Ok(ActorRecord {
        address: address(row.try_get("address")?)?,
        state: row.try_get("state")?,
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
        paused: row.try_get("paused")?,
        parent: row.try_get::<Option<String>, _>("parent")?.map(address).transpose()?,
    })
}

fn node_id(text: String) -> Result<NodeId, TxError> {
    NodeId::new(text).map_err(|e| corrupt(e.to_string()))
}

fn node_record(row: &PgRow) -> Result<NodeRecord, TxError> {
    Ok(NodeRecord {
        id: node_id(row.try_get("id")?)?,
        hosted: support::hosted_from_json(&row.try_get::<Value, _>("hosted")?)?,
        started_at: row.try_get("started_at")?,
        last_seen: row.try_get("last_seen")?,
    })
}

/// A row of the columns of [`RUNNING_TURN`].
fn running_turn(row: &PgRow) -> Result<RunningTurn, TxError> {
    Ok(RunningTurn {
        actor: address(row.try_get("address")?)?,
        message_id: message_id(row.try_get("lease_message")?)?,
        node: node_id(row.try_get("lease_owner")?)?,
        lease_until: row.try_get("lease_until")?,
        cancel_requested: row.try_get("cancel_requested")?,
    })
}

// ---- statements shared by several operations ----

/// The columns of a running turn; `cancel_requested` is true only for the current epoch.
const RUNNING_TURN: &str =
    "address, lease_message, lease_owner, lease_until, cancel_epoch IS NOT DISTINCT FROM epoch AS cancel_requested";

async fn type_def_in(tx: &mut Tx, name: &TypeName) -> Result<Option<TypeDef>, TxError> {
    let row = sqlx::query("SELECT name, contract, contract_hash FROM actor_types WHERE name = $1")
        .bind(name.as_str())
        .fetch_optional(&mut *tx.c)
        .await?;
    row.as_ref().map(type_def).transpose()
}

async fn list_types_in(tx: &mut Tx) -> Result<Vec<TypeDef>, TxError> {
    let rows = sqlx::query("SELECT name, contract, contract_hash FROM actor_types ORDER BY name")
        .fetch_all(&mut *tx.c)
        .await?;
    rows.iter().map(type_def).collect()
}

async fn get_in(tx: &mut Tx, address: &str) -> Result<Option<ActorRecord>, TxError> {
    let row =
        sqlx::query("SELECT address, type, state, version, created_at, paused, parent FROM actors WHERE address = $1")
            .bind(address)
            .fetch_optional(&mut *tx.c)
            .await?;
    row.as_ref().map(actor_record).transpose()
}

/// Locks the instance's row for the rest of the transaction and returns its type, or `None` when absent.
async fn lock_actor(tx: &mut Tx, addr: &str) -> Result<Option<String>, TxError> {
    Ok(sqlx::query_scalar("SELECT type FROM actors WHERE address = $1 FOR UPDATE")
        .bind(addr)
        .fetch_optional(&mut *tx.c)
        .await?)
}

async fn create_in(tx: &mut Tx, address: &ActorAddress, state: &Value) -> Result<CreateResult, TxError> {
    // Holds the type's registration in place until this transaction ends, so that `remove_type` either sees the new
    // instance or runs before this create (then the registration is gone and this create is refused).
    let registered: Option<i32> = sqlx::query_scalar("SELECT 1 FROM actor_types WHERE name = $1 FOR SHARE")
        .bind(address.type_name().as_str())
        .fetch_optional(&mut *tx.c)
        .await?;
    if registered.is_none() {
        return Ok(CreateResult::NoSuchType);
    }
    loop {
        let now = tx.now().await?;
        let inserted = sqlx::query(
            "INSERT INTO actors (address, type, state, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (address) DO NOTHING",
        )
        .bind(address.as_str())
        .bind(address.type_name().as_str())
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

/// Creates `create`'s instance with `parent` unless it exists (idempotent) and locks its row; returns its type, its
/// parent, and whether the row is this transaction's own. Runs before the transaction reads its clock, so `created_at`
/// is taken from `clock_timestamp()` directly.
async fn create_and_lock(
    tx: &mut Tx,
    create: &ActorCreate,
    parent: Option<&str>,
) -> Result<(String, Option<String>, bool), TxError> {
    loop {
        let inserted = sqlx::query(
            "INSERT INTO actors (address, type, state, parent, created_at) VALUES ($1, $2, $3, $4, clock_timestamp()) \
             ON CONFLICT (address) DO NOTHING",
        )
        .bind(create.address.as_str())
        .bind(create.address.type_name().as_str())
        .bind(&create.state)
        .bind(parent)
        .execute(&mut *tx.c)
        .await?
        .rows_affected()
            == 1;
        // Our own row, or the conflicting one once its transaction committed; absent only if it was deleted again.
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT type, parent FROM actors WHERE address = $1 FOR UPDATE")
                .bind(create.address.as_str())
                .fetch_optional(&mut *tx.c)
                .await?;
        if let Some((t, parent)) = row {
            return Ok((t, parent, inserted));
        }
    }
}

/// Locks the descendants of `root`, whose row the caller has locked, level by level (see the type's docs), and returns
/// `root` and every descendant in address order. A level leaves out the rows locked already, so the walk ends on
/// parent links that form a cycle.
async fn lock_tree(tx: &mut Tx, root: &str) -> Result<Vec<String>, TxError> {
    let mut locked = vec![root.to_owned()];
    let mut level = locked.clone();
    while !level.is_empty() {
        level = sqlx::query_scalar(
            "SELECT address FROM actors WHERE parent = ANY ($1) AND address <> ALL ($2) ORDER BY address FOR UPDATE",
        )
        .bind(&level)
        .bind(&locked)
        .fetch_all(&mut *tx.c)
        .await?;
        locked.extend(level.iter().cloned());
    }
    locked.sort_unstable();
    Ok(locked)
}

/// Removes every instance of `tree`, whose rows the caller has locked, purges their mailboxes and removes the watches
/// they hold as watchers; returns how many messages were purged.
async fn remove_tree(tx: &mut Tx, tree: &[String]) -> Result<usize, TxError> {
    let mut purged = 0;
    for addr in tree {
        purged += purge_mailbox(tx, addr).await?;
        sqlx::query("DELETE FROM actors WHERE address = $1").bind(addr).execute(&mut *tx.c).await?;
    }
    sqlx::query("DELETE FROM watches WHERE watcher = ANY ($1)").bind(tree).execute(&mut *tx.c).await?;
    Ok(purged)
}

/// Locks the rows of `addresses` that exist, in address order, and returns their addresses.
async fn lock_rows(tx: &mut Tx, addresses: &[&str]) -> Result<Vec<String>, TxError> {
    Ok(sqlx::query_scalar("SELECT address FROM actors WHERE address = ANY ($1) ORDER BY address FOR UPDATE")
        .bind(addresses)
        .fetch_all(&mut *tx.c)
        .await?)
}

/// The watchers of the watches on `targets`, unlocked; the first reading of an event (see the type's docs).
async fn watchers_of(tx: &mut Tx, targets: &[String]) -> Result<Vec<String>, TxError> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar("SELECT DISTINCT watcher FROM watches WHERE target = ANY ($1)")
        .bind(targets)
        .fetch_all(&mut *tx.c)
        .await?)
}

/// Locks the watchers of the watches on `descendants` that `locked` does not hold yet, in address order, and adds
/// them to it: the watchers of a removed tree's descendants, after the tree.
async fn lock_watchers_of(tx: &mut Tx, descendants: &[String], locked: &mut HashSet<String>) -> Result<(), TxError> {
    let watchers = watchers_of(tx, descendants).await?;
    let new: Vec<&str> = watchers.iter().map(String::as_str).filter(|w| !locked.contains(*w)).collect();
    if !new.is_empty() {
        locked.extend(lock_rows(tx, &new).await?);
    }
    Ok(())
}

/// The watches of an event's targets, as `(target, watcher)`.
type EventWatches = Vec<(String, String)>;

/// Reads the watches on `targets` again, once every lock is held: a watcher that `locked` does not hold and whose row
/// exists set its watch in between, and the transaction runs again ([`TxError::Again`]); one whose row does not exist
/// is returned as gone.
async fn reread_watches(
    tx: &mut Tx,
    targets: &[String],
    locked: &HashSet<String>,
) -> Result<(EventWatches, HashSet<String>), TxError> {
    if targets.is_empty() {
        return Ok((Vec::new(), HashSet::new()));
    }
    let watches: EventWatches =
        sqlx::query_as("SELECT target, watcher FROM watches WHERE target = ANY ($1) ORDER BY target, watcher")
            .bind(targets)
            .fetch_all(&mut *tx.c)
            .await?;
    let mut gone = HashSet::new();
    for (_, watcher) in &watches {
        if locked.contains(watcher) || gone.contains(watcher) {
            continue;
        }
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM actors WHERE address = $1")
            .bind(watcher)
            .fetch_optional(&mut *tx.c)
            .await?;
        if exists.is_some() {
            return Err(TxError::Again(format!("The watcher {watcher} set a watch while an event was written")));
        }
        gone.insert(watcher.clone());
    }
    Ok((watches, gone))
}

/// The notices of an event from the watches of its targets: the `Failed` of `failed` (a message that ends `failed`,
/// with its error) for the watchers of its failure, and a `Stopped` of each actor of `removed` for the watchers of its
/// address. A watcher the event removes gets nothing; a gone one gets nothing and loses its watches.
async fn notices(
    tx: &mut Tx,
    watches: &EventWatches,
    gone: &HashSet<String>,
    failed: Option<(&Message, &str, FailureCause)>,
    removed: Option<(&Removal<'_>, &[String])>,
) -> Result<Vec<Notice>, TxError> {
    let mut candidates: Vec<(ActorAddress, &str, bool)> = Vec::new();
    if let Some((message, _, _)) = failed {
        let targets = watch::failure_targets(&message.to);
        let watchers: BTreeSet<&str> =
            watches.iter().filter(|(t, _)| targets.contains(t)).map(|(_, w)| w.as_str()).collect();
        candidates.extend(watchers.into_iter().map(|w| (message.to.clone(), w, true)));
    }
    if let Some((_, tree)) = removed {
        for actor in tree {
            let about = address(actor.clone())?;
            candidates
                .extend(watches.iter().filter(|(t, _)| t == actor).map(|(_, w)| (about.clone(), w.as_str(), false)));
        }
    }
    let tree = removed.map_or(&[][..], |(_, tree)| tree);
    let mut lost = Vec::new();
    let mut out = Vec::with_capacity(candidates.len());
    for (about, watcher, is_failure) in candidates {
        if tree.iter().any(|a| a == watcher) {
            continue;
        }
        if gone.contains(watcher) {
            lost.push(watcher.to_owned());
            continue;
        }
        let watcher = address(watcher.to_owned())?;
        out.push(match (failed, removed) {
            (Some((message, error, cause)), _) if is_failure => Notice::failed(message, error, cause, &watcher),
            (_, Some((removal, _))) => removal.notice(&about, &watcher),
            _ => unreachable!("a candidate comes from the failure or the removal"),
        });
    }
    if !lost.is_empty() {
        sqlx::query("DELETE FROM watches WHERE watcher = ANY ($1)").bind(&lost).execute(&mut *tx.c).await?;
    }
    Ok(out)
}

/// Writes the notifications of `notices`, named after `event`, into the watchers' mailboxes, whose rows the caller has
/// locked, and wakes the nodes of their types; one whose id is in use is skipped.
async fn notify(tx: &mut Tx, event: &MessageId, notices: Vec<Notice>) -> Result<(), TxError> {
    for message in support::notifications(event, notices) {
        if is_id_in_use(tx, &message.id).await? {
            tracing::warn!("Skipping the notification {} for {}: its id is in use", message.id, message.to);
            continue;
        }
        if insert(tx, &message, true).await?.is_none() {
            // Taken since the check: the re-run skips it.
            return Err(TxError::Again(format!("The id of the notification {} was taken", message.id)));
        }
        tx.notify(message.to.type_name().as_str().to_owned());
    }
    Ok(())
}

async fn is_id_in_use(tx: &mut Tx, id: &MessageId) -> Result<bool, TxError> {
    let pending: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM messages WHERE id = $1").bind(id.as_str()).fetch_optional(&mut *tx.c).await?;
    if pending.is_some() {
        return Ok(true);
    }
    let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
    let finished: Option<i32> =
        sqlx::query_scalar(concat!("SELECT 1 FROM outcomes WHERE message_id = $1 AND ", retained!(2, 3)))
            .bind(id.as_str())
            .bind(cutoff)
            .bind(now)
            .fetch_optional(&mut *tx.c)
            .await?;
    Ok(finished.is_some())
}

/// Inserts `message` into the mailbox of its target, whose row the caller has locked, marked as a notification when
/// `notification`. Returns `None` when the id is in use; the caller must then roll back, because the message row may
/// already be written.
async fn insert(tx: &mut Tx, message: &OutgoingMessage, notification: bool) -> Result<Option<Message>, TxError> {
    let now = tx.now().await?;
    let deliver_at = message.delivery.resolve(now).map_err(|e| invalid(e.0))?;
    let seq: Option<i64> = sqlx::query_scalar(
        "INSERT INTO messages
            (id, actor_address, seq, type, payload, sender, correlation_id, headers, deliver_at, attempt, retry_policy,
             caused_by, retention_policy, notification)
         VALUES ($1, $2, nextval('message_seq'), $3, $4, $5, $6, $7, $8, 0, $9, $10, $11, $12)
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
    .bind(notification)
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
    sqlx::query("UPDATE actors SET next_due_at = LEAST(next_due_at, $1) WHERE address = $2")
        .bind(deliver_at)
        .bind(message.to.as_str())
        .execute(&mut *tx.c)
        .await?;
    Ok(Some(message.clone().into_message(deliver_at, seq, notification)))
}

async fn refresh_next_due(tx: &mut Tx, addr: &str) -> Result<(), TxError> {
    sqlx::query(
        "UPDATE actors SET next_due_at = (SELECT min(deliver_at) FROM messages WHERE actor_address = $1) WHERE address = $1",
    )
    .bind(addr)
    .execute(&mut *tx.c)
    .await?;
    Ok(())
}

async fn finish(
    tx: &mut Tx,
    message: &Message,
    status: OutcomeStatus,
    error: Option<&str>,
) -> Result<OutcomeRecord, TxError> {
    let now = tx.now().await?;
    let cutoff = tx.cutoff().await?;
    sqlx::query("DELETE FROM messages WHERE id = $1").bind(message.id.as_str()).execute(&mut *tx.c).await?;
    let removed = support::removed_at_once(message, status);
    let keep_last = support::keep_last(message, status);
    let envelope = if removed {
        support::envelope_to_json(&support::stripped(message))
    } else {
        support::envelope_to_json(message)
    };
    sqlx::query(
        "INSERT INTO outcomes
            (message_id, actor_address, status, error, finished_at, envelope, caused_by, visible_until, keep_last,
             keep_seq, stripped)
         VALUES ($1, $2, $3, $4, $5, $6, $10, $7, $8, CASE WHEN $8 THEN nextval('message_seq') END, $9)
         ON CONFLICT (message_id) DO UPDATE SET actor_address = excluded.actor_address, status = excluded.status,
            error = excluded.error, finished_at = excluded.finished_at, envelope = excluded.envelope,
            caused_by = excluded.caused_by,
            visible_until = excluded.visible_until, keep_last = excluded.keep_last,
            keep_seq = excluded.keep_seq, stripped = excluded.stripped",
    )
    .bind(message.id.as_str())
    .bind(message.to.as_str())
    .bind(status.wire_name())
    .bind(if removed { None } else { error })
    .bind(now)
    .bind(envelope)
    .bind(support::visible_until(message, status, now))
    .bind(keep_last.is_some())
    .bind(removed)
    .bind(message.caused_by.as_ref().map(MessageId::as_str))
    .execute(&mut *tx.c)
    .await?;
    if let Some(count) = keep_last {
        // Hide this instance's visible keep-last outcomes of this status beyond the newest `count`. The caller holds
        // the instance's row lock, so trims of one instance do not interleave.
        sqlx::query(concat!(
            "UPDATE outcomes SET visible_until = $1
             WHERE message_id IN (
                 SELECT message_id FROM outcomes
                 WHERE actor_address = $2 AND status = $3 AND keep_last AND ",
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

async fn purge_mailbox(tx: &mut Tx, addr: &str) -> Result<usize, TxError> {
    let rows = sqlx::query(concat!(
        "SELECT ",
        message_columns!(),
        " FROM messages WHERE actor_address = $1 ORDER BY deliver_at, seq"
    ))
    .bind(addr)
    .fetch_all(&mut *tx.c)
    .await?;
    let pending = rows.iter().map(message).collect::<Result<Vec<_>, _>>()?;
    for m in &pending {
        finish(tx, m, OutcomeStatus::Purged, None).await?;
    }
    Ok(pending.len())
}

/// Locks the row of `addr` and gives `purged` to every pending message of it except the one being handled: `own`, the
/// message of the committing turn when its commit purges its own instance, or else the message of a valid lease.
/// Purging a message whose lease expired also voids that lease, so that the stale turn cannot commit. `None` when the
/// instance is absent.
async fn purge_in(tx: &mut Tx, addr: &str, own: Option<&str>) -> Result<Option<u64>, TxError> {
    let lease: Option<(Option<String>, Option<Timestamp>, Option<String>)> =
        sqlx::query_as("SELECT lease_owner, lease_until, lease_message FROM actors WHERE address = $1 FOR UPDATE")
            .bind(addr)
            .fetch_optional(&mut *tx.c)
            .await?;
    let Some((owner, until, lease_message)) = lease else {
        return Ok(None);
    };
    let now = tx.now().await?;
    let held = match own {
        Some(id) => Some(id.to_owned()),
        None if owner.is_some() && until.is_some_and(|until| until > now) => lease_message,
        None => None,
    };
    let rows = sqlx::query(concat!(
        "SELECT ",
        message_columns!(),
        " FROM messages WHERE actor_address = $1 ORDER BY deliver_at, seq"
    ))
    .bind(addr)
    .fetch_all(&mut *tx.c)
    .await?;
    let pending = rows.iter().map(message).collect::<Result<Vec<_>, _>>()?;
    let mut purged: u64 = 0;
    for m in pending.iter().filter(|m| held.as_deref() != Some(m.id.as_str())) {
        finish(tx, m, OutcomeStatus::Purged, None).await?;
        purged += 1;
    }
    if held.is_none() {
        // Void an expired lease whose message is gone, so that the stale turn cannot commit.
        sqlx::query(
            "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE address = $1",
        )
        .bind(addr)
        .execute(&mut *tx.c)
        .await?;
    }
    refresh_next_due(tx, addr).await?;
    Ok(Some(purged))
}

async fn live_outcome(tx: &mut Tx, id: &MessageId) -> Result<Option<OutcomeRecord>, TxError> {
    let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
    let row = sqlx::query(concat!(
        "SELECT ",
        outcome_columns!(),
        " FROM outcomes WHERE message_id = $1 AND ",
        visible!(2, 3)
    ))
    .bind(id.as_str())
    .bind(cutoff)
    .bind(now)
    .fetch_optional(&mut *tx.c)
    .await?;
    row.as_ref().map(outcome).transpose()
}

async fn status_in(tx: &mut Tx, id: &MessageId) -> Result<Option<MessageStatus>, TxError> {
    let now = tx.now().await?;
    let row = sqlx::query(concat!(
        "SELECT ",
        message_columns!("m."),
        ", a.lease_owner, a.lease_until FROM messages m
         LEFT JOIN actors a ON a.address = m.actor_address AND a.lease_message = m.id AND a.lease_owner IS NOT NULL
             AND a.lease_until > $1
         WHERE m.id = $2"
    ))
    .bind(now)
    .bind(id.as_str())
    .fetch_optional(&mut *tx.c)
    .await?;
    if let Some(row) = row {
        let message = message(&row)?;
        let owner: Option<String> = row.try_get("lease_owner")?;
        let until: Option<Timestamp> = row.try_get("lease_until")?;
        return Ok(Some(match (owner, until) {
            (Some(owner), Some(lease_until)) => MessageStatus::Running {
                message,
                node: NodeId::new(owner).map_err(|e| corrupt(e.to_string()))?,
                lease_until,
            },
            _ => MessageStatus::Pending(message),
        }));
    }
    Ok(live_outcome(tx, id).await?.map(MessageStatus::Finished))
}

fn commit<T>(value: T) -> TxResult<T> {
    Ok(Done::Commit(value))
}

#[async_trait]
impl Backend for PostgresStore {
    async fn now(&self) -> StoreResult<Timestamp> {
        transaction!(self, true, |tx| -> Timestamp { commit(tx.now().await?) })
    }

    // ---- type registry ----

    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult> {
        transaction!(self, false, |tx| -> RegisterResult { commit(Self::register_in(tx, &def).await?) })
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        transaction!(self, true, |tx| -> Vec<TypeDef> { commit(list_types_in(tx).await?) })
    }

    // ---- state store ----

    async fn remove_type(&self, name: &TypeName) -> StoreResult<RemoveTypeResult> {
        let prefix = format!("{name}/");
        transaction!(self, false, |tx| -> RemoveTypeResult {
            // The row lock makes this exclusive with `create`, which takes a share lock on it: a create that started
            // first is visible to the check below (a new statement, so a new snapshot), one that starts later waits and
            // then finds the type gone.
            let registered: Option<i32> = sqlx::query_scalar("SELECT 1 FROM actor_types WHERE name = $1 FOR UPDATE")
                .bind(name.as_str())
                .fetch_optional(&mut *tx.c)
                .await?;
            if registered.is_none() {
                return commit(RemoveTypeResult::Absent);
            }
            let used: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM actors WHERE address >= $1 AND address < $2 AND starts_with(address, $1) LIMIT 1",
            )
            .bind(&prefix)
            .bind(upper_bound(&prefix))
            .fetch_optional(&mut *tx.c)
            .await?;
            if used.is_some() {
                return commit(RemoveTypeResult::InUse);
            }
            sqlx::query("DELETE FROM actor_types WHERE name = $1").bind(name.as_str()).execute(&mut *tx.c).await?;
            commit(RemoveTypeResult::Removed)
        })
    }

    async fn create(&self, address: &ActorAddress, state: Value) -> StoreResult<CreateResult> {
        transaction!(self, false, |tx| -> CreateResult { commit(create_in(tx, address, &state).await?) })
    }

    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        transaction!(self, true, |tx| -> Option<ActorRecord> { commit(get_in(tx, address.as_str()).await?) })
    }

    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        check_limit(limit)?;
        let lower_bound = cursor.map(|c| c.0.as_str()).filter(|c| *c >= prefix).unwrap_or(prefix);
        let inclusive = cursor.is_none_or(|c| c.0.as_str() < prefix);
        let sql = if inclusive {
            "SELECT address FROM actors WHERE address >= $1 AND address < $2 AND starts_with(address, $3) ORDER BY address LIMIT $4"
        } else {
            "SELECT address FROM actors WHERE address > $1 AND address < $2 AND starts_with(address, $3) ORDER BY address LIMIT $4"
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
            commit(support::page_of(addresses, limit, |a| PageCursor(a.to_string())))
        })
    }

    async fn children(
        &self,
        parent: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorAddress>> {
        check_limit(limit)?;
        let after = cursor.map_or("", |c| c.0.as_str());
        transaction!(self, true, |tx| -> Page<ActorAddress> {
            let rows: Vec<String> = sqlx::query_scalar(
                "SELECT address FROM actors WHERE parent = $1 AND address > $2 ORDER BY address LIMIT $3",
            )
            .bind(parent.as_str())
            .bind(after)
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx.c)
            .await?;
            let addresses = rows.into_iter().map(address).collect::<Result<Vec<_>, _>>()?;
            commit(support::page_of(addresses, limit, |a| PageCursor(a.to_string())))
        })
    }

    async fn delete(&self, address: &ActorAddress, event: &MessageId) -> StoreResult<DeleteResult> {
        transaction!(self, false, |tx| -> DeleteResult {
            // The watchers of the actor are read first and locked with it, in address order (see the type's docs).
            let root = vec![address.to_string()];
            let watchers = watchers_of(tx, &root).await?;
            let rows: Vec<&str> =
                std::iter::once(address.as_str()).chain(watchers.iter().map(String::as_str)).collect();
            let mut locked: HashSet<String> = lock_rows(tx, &rows).await?.into_iter().collect();
            if !locked.contains(address.as_str()) {
                return commit(DeleteResult::Absent);
            }
            let tree = lock_tree(tx, address.as_str()).await?;
            locked.extend(tree.iter().cloned());
            let descendants: Vec<String> = tree.iter().filter(|a| *a != address.as_str()).cloned().collect();
            lock_watchers_of(tx, &descendants, &mut locked).await?;
            let (watches, gone) = reread_watches(tx, &tree, &locked).await?;
            let removal = Removal::delete(address);
            let notices = notices(tx, &watches, &gone, None, Some((&removal, &tree))).await?;
            let purged = remove_tree(tx, &tree).await?;
            notify(tx, event, notices).await?;
            commit(DeleteResult::Deleted { purged })
        })
    }

    // ---- watches ----

    async fn watch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<SetWatchResult> {
        transaction!(self, false, |tx| -> SetWatchResult {
            if lock_actor(tx, watcher.as_str()).await?.is_none() {
                return commit(SetWatchResult::NoSuchWatcher);
            }
            let inserted = sqlx::query(
                "INSERT INTO watches (target, watcher) VALUES ($1, $2) ON CONFLICT (target, watcher) DO NOTHING",
            )
            .bind(target.as_str())
            .bind(watcher.as_str())
            .execute(&mut *tx.c)
            .await?;
            commit(if inserted.rows_affected() == 1 { SetWatchResult::Created } else { SetWatchResult::Exists })
        })
    }

    async fn unwatch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<bool> {
        transaction!(self, false, |tx| -> bool {
            lock_actor(tx, watcher.as_str()).await?;
            let removed = sqlx::query("DELETE FROM watches WHERE target = $1 AND watcher = $2")
                .bind(target.as_str())
                .bind(watcher.as_str())
                .execute(&mut *tx.c)
                .await?;
            commit(removed.rows_affected() == 1)
        })
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
        let (after_target, after_watcher) = after.unzip();
        transaction!(self, true, |tx| -> Page<Watch> {
            let rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT target, watcher FROM watches
                 WHERE ($1::text IS NULL OR target = $1) AND ($2::text IS NULL OR watcher = $2)
                   AND ($3::text IS NULL OR (target, watcher) > ($3, $4))
                 ORDER BY target, watcher LIMIT $5",
            )
            .bind(target.map(WatchTarget::as_str))
            .bind(watcher.map(ActorAddress::as_str))
            .bind(after_target.as_deref())
            .bind(after_watcher.as_deref())
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx.c)
            .await?;
            let watches = rows.iter().map(|(t, w)| support::stored_watch(t, w)).collect::<Result<Vec<_>, _>>()?;
            commit(support::page_of(watches, limit, support::watch_cursor))
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
            let Some(stored) = insert(tx, &message, false).await? else {
                return Ok(Done::Rollback(EnqueueResult::Duplicate(message.id.clone())));
            };
            tx.notify(t);
            commit(EnqueueResult::Enqueued(stored))
        })
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
        let names: Vec<String> = types.iter().map(|t| t.as_str().to_owned()).collect();
        transaction!(self, false, |tx| -> Vec<Claim> {
            // SKIP LOCKED never waits, so reading the clock first is safe here.
            let now = tx.now().await?;
            let candidates = sqlx::query(
                "SELECT address, type, state, version, CASE WHEN lease_owner IS NOT NULL THEN lease_message END AS lost
                 FROM actors
                 WHERE next_due_at <= $1 AND NOT paused AND type = ANY ($2)
                   AND (lease_owner IS NULL OR lease_until <= $1)
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
                let addr: String = candidate.try_get("address")?;
                // A lease that ran out and that nothing cleared: its claim is lost (ADR 0025). The table holds only
                // pending messages, so a message that is gone counts nothing.
                if let Some(lost) = candidate.try_get::<Option<String>, _>("lost")? {
                    sqlx::query("UPDATE messages SET lost_claims = lost_claims + 1 WHERE id = $1")
                        .bind(lost)
                        .execute(&mut *tx.c)
                        .await?;
                }
                let head = sqlx::query(concat!(
                    "SELECT ",
                    message_columns!(),
                    " FROM messages WHERE actor_address = $1 ORDER BY deliver_at, seq LIMIT 1"
                ))
                .bind(&addr)
                .fetch_optional(&mut *tx.c)
                .await?;
                let head = head.as_ref().map(message).transpose()?;
                let Some(head) = head.filter(|h| h.deliver_at <= now) else {
                    return Err(corrupt(format!("next_due_at out of sync for {addr}")));
                };
                let claimed = Message { attempt: head.attempt + 1, ..head };
                sqlx::query("UPDATE messages SET attempt = $1 WHERE id = $2")
                    .bind(claimed.attempt as i32)
                    .bind(claimed.id.as_str())
                    .execute(&mut *tx.c)
                    .await?;
                let epoch: i64 = sqlx::query_scalar(
                    "UPDATE actors SET epoch = nextval('epoch_seq'), lease_owner = $1, lease_until = $2, lease_message = $3
                     WHERE address = $4 RETURNING epoch",
                )
                .bind(node.as_str())
                .bind(lease_until)
                .bind(claimed.id.as_str())
                .bind(&addr)
                .fetch_one(&mut *tx.c)
                .await?;
                claims.push(Claim {
                    actor: address(addr)?,
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
            let held: Option<Option<i64>> = sqlx::query_scalar(
                "SELECT cancel_epoch FROM actors WHERE address = $1 AND epoch = $2 AND lease_owner IS NOT NULL FOR UPDATE",
            )
            .bind(address.as_str())
            .bind(epoch)
            .fetch_optional(&mut *tx.c)
            .await?;
            let Some(cancel_epoch) = held else {
                return commit(RenewResult::LeaseLost);
            };
            let lease_until = time::truncate(time::add(tx.now().await?, ttl).map_err(|e| invalid(e.0))?);
            sqlx::query("UPDATE actors SET lease_until = $1 WHERE address = $2")
                .bind(lease_until)
                .bind(address.as_str())
                .execute(&mut *tx.c)
                .await?;
            commit(if cancel_epoch == Some(epoch) {
                RenewResult::CancelRequested(lease_until)
            } else {
                RenewResult::Renewed(lease_until)
            })
        })
    }

    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        transaction!(self, false, |tx| -> () {
            let t: Option<String> = sqlx::query_scalar(
                "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL
                 WHERE address = $1 AND epoch = $2 AND lease_owner IS NOT NULL
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

    async fn next_due_at(&self, types: &BTreeSet<TypeName>) -> StoreResult<Option<Timestamp>> {
        if types.is_empty() {
            return Ok(None);
        }
        let names: Vec<String> = types.iter().map(|t| t.as_str().to_owned()).collect();
        transaction!(self, true, |tx| -> Option<Timestamp> {
            let now = tx.now().await?;
            let at: Option<Timestamp> = sqlx::query_scalar(
                "SELECT min(CASE WHEN lease_owner IS NOT NULL AND lease_until > $1 THEN greatest(next_due_at, lease_until)
                                 ELSE next_due_at END)
                 FROM actors WHERE next_due_at IS NOT NULL AND NOT paused AND type = ANY ($2)",
            )
            .bind(now)
            .bind(&names)
            .fetch_one(&mut *tx.c)
            .await?;
            commit(at)
        })
    }

    async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_mailbox_cursor).transpose()?;
        transaction!(self, true, |tx| -> Page<Message> {
            let rows = match after {
                None => {
                    sqlx::query(concat!(
                        "SELECT ",
                        message_columns!(),
                        " FROM messages WHERE actor_address = $1 ORDER BY deliver_at, seq LIMIT $2"
                    ))
                    .bind(address.as_str())
                    .bind(limit as i64 + 1)
                    .fetch_all(&mut *tx.c)
                    .await?
                }
                Some((at, seq)) => {
                    sqlx::query(concat!(
                        "SELECT ",
                        message_columns!(),
                        " FROM messages WHERE actor_address = $1 AND (deliver_at, seq) > ($2, $3)
                         ORDER BY deliver_at, seq LIMIT $4"
                    ))
                    .bind(address.as_str())
                    .bind(lower_bound(time::from_micros(at)))
                    .bind(seq)
                    .bind(limit as i64 + 1)
                    .fetch_all(&mut *tx.c)
                    .await?
                }
            };
            let messages = rows.iter().map(message).collect::<Result<Vec<_>, _>>()?;
            commit(support::page_of(messages, limit, support::mailbox_cursor))
        })
    }

    async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_pending_cursor).transpose()?;
        transaction!(self, true, |tx| -> Page<Message> {
            // A range on actor_address walks the mailbox index (actor_address, deliver_at, seq) in order; no sort.
            let rows = match &after {
                None => {
                    sqlx::query(concat!(
                        "SELECT ",
                        message_columns!(),
                        " FROM messages
                         WHERE actor_address >= $1 AND actor_address < $2 AND starts_with(actor_address, $3)
                         ORDER BY actor_address, deliver_at, seq LIMIT $4"
                    ))
                    .bind(prefix)
                    .bind(upper_bound(prefix))
                    .bind(prefix)
                    .bind(limit as i64 + 1)
                    .fetch_all(&mut *tx.c)
                    .await?
                }
                Some((addr, at, seq)) => {
                    sqlx::query(concat!(
                        "SELECT ",
                        message_columns!(),
                        " FROM messages
                         WHERE actor_address >= $1 AND actor_address < $2 AND starts_with(actor_address, $3)
                           AND (actor_address, deliver_at, seq) > ($4, $5, $6)
                         ORDER BY actor_address, deliver_at, seq LIMIT $7"
                    ))
                    .bind(prefix)
                    .bind(upper_bound(prefix))
                    .bind(prefix)
                    .bind(addr)
                    .bind(lower_bound(time::from_micros(*at)))
                    .bind(*seq)
                    .bind(limit as i64 + 1)
                    .fetch_all(&mut *tx.c)
                    .await?
                }
            };
            let messages = rows.iter().map(message).collect::<Result<Vec<_>, _>>()?;
            commit(support::page_of(messages, limit, support::pending_cursor))
        })
    }

    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        transaction!(self, false, |tx| -> CancelResult {
            let addr: Option<String> = sqlx::query_scalar("SELECT actor_address FROM messages WHERE id = $1")
                .bind(id.as_str())
                .fetch_optional(&mut *tx.c)
                .await?;
            let t = match &addr {
                Some(addr) => lock_actor(tx, addr).await?,
                None => None,
            };
            // Read again under the instance lock: the message may have left the mailbox in the meantime.
            let row = match t {
                Some(_) => {
                    sqlx::query(concat!("SELECT ", message_columns!(), " FROM messages WHERE id = $1"))
                        .bind(id.as_str())
                        .fetch_optional(&mut *tx.c)
                        .await?
                }
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
                "SELECT lease_until FROM actors WHERE address = $1 AND lease_message = $2 AND lease_owner IS NOT NULL",
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
                sqlx::query(
                    "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE address = $1",
                )
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

    async fn deliver_now(&self, id: &MessageId) -> StoreResult<DeliverNowResult> {
        transaction!(self, false, |tx| -> DeliverNowResult {
            let addr: Option<String> = sqlx::query_scalar("SELECT actor_address FROM messages WHERE id = $1")
                .bind(id.as_str())
                .fetch_optional(&mut *tx.c)
                .await?;
            let t = match &addr {
                Some(addr) => lock_actor(tx, addr).await?,
                None => None,
            };
            // Read again under the instance lock: the message may have left the mailbox in the meantime.
            let row = match t {
                Some(_) => {
                    sqlx::query(concat!("SELECT ", message_columns!(), " FROM messages WHERE id = $1"))
                        .bind(id.as_str())
                        .fetch_optional(&mut *tx.c)
                        .await?
                }
                None => None,
            };
            let (Some(t), Some(row)) = (t, row) else {
                return commit(match live_outcome(tx, id).await? {
                    Some(o) => DeliverNowResult::AlreadyFinished(o),
                    None => DeliverNowResult::NotFound,
                });
            };
            let message = message(&row)?;
            let now = tx.now().await?;
            let running: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM actors WHERE address = $1 AND lease_message = $2 AND lease_owner IS NOT NULL
                   AND lease_until > $3",
            )
            .bind(message.to.as_str())
            .bind(id.as_str())
            .bind(now)
            .fetch_optional(&mut *tx.c)
            .await?;
            if running.is_some() {
                return commit(DeliverNowResult::Running);
            }
            let seq: i64 = sqlx::query_scalar(
                "UPDATE messages SET deliver_at = $1, seq = nextval('message_seq') WHERE id = $2 RETURNING seq",
            )
            .bind(now)
            .bind(id.as_str())
            .fetch_one(&mut *tx.c)
            .await?;
            refresh_next_due(tx, message.to.as_str()).await?;
            tx.notify(t);
            commit(DeliverNowResult::Delivered(Message { deliver_at: now, seq, ..message }))
        })
    }

    async fn set_paused(&self, address: &ActorAddress, paused: bool) -> StoreResult<bool> {
        transaction!(self, false, |tx| -> bool {
            let t: Option<String> =
                sqlx::query_scalar("UPDATE actors SET paused = $1 WHERE address = $2 RETURNING type")
                    .bind(paused)
                    .bind(address.as_str())
                    .fetch_optional(&mut *tx.c)
                    .await?;
            if let (Some(t), false) = (&t, paused) {
                tx.notify(t.clone());
            }
            commit(t.is_some())
        })
    }

    async fn purge(&self, address: &ActorAddress) -> StoreResult<Option<u64>> {
        transaction!(self, false, |tx| -> Option<u64> { commit(purge_in(tx, address.as_str(), None).await?) })
    }

    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        transaction!(self, true, |tx| -> Option<MessageStatus> { commit(status_in(tx, id).await?) })
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
        let after = cursor.map(support::decode_outcome_cursor).transpose()?;
        let since = lower_bound(time::truncate(since));
        transaction!(self, true, |tx| -> Page<OutcomeRecord> {
            let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
            // Without a status the condition is left out (a null status parameter would hide the partial index).
            let base = concat!(
                "SELECT ",
                outcome_columns!(),
                " FROM outcomes
                 WHERE finished_at >= $1 AND ",
                visible!(2, 3),
                " AND actor_address >= $4 AND actor_address < $5 AND starts_with(actor_address, $6)"
            );
            let status_filter = if status.is_some() { " AND status = $7" } else { "" };
            let n = 6 + usize::from(status.is_some());
            let sql = match &after {
                None => format!("{base}{status_filter} ORDER BY finished_at, message_id LIMIT ${}", n + 1),
                Some(_) => format!(
                    "{base}{status_filter} AND (finished_at, message_id) > (${}, ${}) \
                     ORDER BY finished_at, message_id LIMIT ${}",
                    n + 1,
                    n + 2,
                    n + 3
                ),
            };
            let mut query = sqlx::query(AssertSqlSafe(sql))
                .bind(since)
                .bind(cutoff)
                .bind(now)
                .bind(prefix)
                .bind(upper_bound(prefix))
                .bind(prefix);
            if let Some(status) = status {
                query = query.bind(status.wire_name());
            }
            if let Some((at, id)) = &after {
                query = query.bind(lower_bound(time::from_micros(*at))).bind(id.clone());
            }
            let rows = query.bind(limit as i64 + 1).fetch_all(&mut *tx.c).await?;
            let outcomes = rows.iter().map(outcome).collect::<Result<Vec<_>, _>>()?;
            commit(support::page_of(outcomes, limit, |o| support::outcome_cursor(o.finished_at, &o.message_id)))
        })
    }

    async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<MessageStatus>> {
        check_limit(limit)?;
        let after = cursor.map_or("", |c| c.0.as_str());
        transaction!(self, true, |tx| -> Page<MessageStatus> {
            let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
            // The ids first, from the pending messages and the visible outcomes; then each one's status.
            let ids: Vec<String> = sqlx::query_scalar(concat!(
                "SELECT id FROM (
                     SELECT id FROM messages WHERE caused_by = $1 AND id > $2
                     UNION
                     SELECT message_id FROM outcomes WHERE caused_by = $1 AND message_id > $2 AND ",
                visible!(3, 4),
                ") AS caused ORDER BY id LIMIT $5"
            ))
            .bind(id.as_str())
            .bind(after)
            .bind(cutoff)
            .bind(now)
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx.c)
            .await?;
            let ids = ids.into_iter().map(message_id).collect::<Result<Vec<_>, _>>()?;
            let page = support::page_of(ids, limit, |id| PageCursor(id.to_string()));
            let mut items = Vec::with_capacity(page.items.len());
            for id in &page.items {
                items.extend(status_in(tx, id).await?);
            }
            commit(Page::new(items, page.next))
        })
    }

    async fn stats(&self, prefix: &str) -> StoreResult<StoreStats> {
        transaction!(self, true, |tx| -> StoreStats {
            let now = tx.now().await?;
            let actors: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM actors WHERE address >= $1 AND address < $2 AND starts_with(address, $3)",
            )
            .bind(prefix)
            .bind(upper_bound(prefix))
            .bind(prefix)
            .fetch_one(&mut *tx.c)
            .await?;
            let (pending, oldest): (i64, Option<Timestamp>) = sqlx::query_as(
                "SELECT count(*), min(deliver_at) FROM messages
                 WHERE actor_address >= $1 AND actor_address < $2 AND starts_with(actor_address, $3)",
            )
            .bind(prefix)
            .bind(upper_bound(prefix))
            .bind(prefix)
            .fetch_one(&mut *tx.c)
            .await?;
            let oldest_due = oldest.map(time::truncate).filter(|at| *at <= now);
            commit(StoreStats { actors: actors as u64, pending: pending as u64, oldest_due })
        })
    }

    async fn sweep_expired(&self) -> StoreResult<u64> {
        transaction!(self, false, |tx| -> u64 {
            let (cutoff, now) = (tx.cutoff().await?, tx.now().await?);
            // Hidden records lose their content (the minimal record stays for dedup).
            sqlx::query(
                "UPDATE outcomes SET error = NULL, stripped = true,
                     envelope = envelope || jsonb_build_object('payload', 'null'::jsonb, 'headers', '{}'::jsonb,
                                                               'correlationId', NULL, 'lastError', NULL)
                 WHERE NOT stripped AND visible_until IS NOT NULL AND visible_until <= $1",
            )
            .bind(now)
            .execute(&mut *tx.c)
            .await?;
            let removed = sqlx::query(
                "DELETE FROM outcomes WHERE finished_at <= $1 AND (visible_until IS NULL OR visible_until <= $2)",
            )
            .bind(cutoff)
            .bind(now)
            .execute(&mut *tx.c)
            .await?;
            commit(removed.rows_affected())
        })
    }

    // ---- nodes and running turns ----

    async fn heartbeat_node(&self, id: &NodeId, hosted: &BTreeSet<TypeName>) -> StoreResult<()> {
        transaction!(self, false, |tx| -> () {
            let now = tx.now().await?;
            sqlx::query(
                "INSERT INTO nodes (id, hosted, started_at, last_seen) VALUES ($1, $2, $3, $3)
                 ON CONFLICT (id) DO UPDATE SET hosted = excluded.hosted, last_seen = excluded.last_seen",
            )
            .bind(id.as_str())
            .bind(support::hosted_to_json(hosted))
            .bind(now)
            .execute(&mut *tx.c)
            .await?;
            sqlx::query("DELETE FROM nodes WHERE last_seen < $1")
                .bind(time::sub(now, support::NODE_ROW_KEPT))
                .execute(&mut *tx.c)
                .await?;
            commit(())
        })
    }

    async fn list_nodes(&self) -> StoreResult<Vec<NodeRecord>> {
        transaction!(self, true, |tx| -> Vec<NodeRecord> {
            let rows = sqlx::query("SELECT id, hosted, started_at, last_seen FROM nodes ORDER BY id")
                .fetch_all(&mut *tx.c)
                .await?;
            commit(rows.iter().map(node_record).collect::<Result<Vec<_>, _>>()?)
        })
    }

    async fn remove_node(&self, id: &NodeId) -> StoreResult<()> {
        transaction!(self, false, |tx| -> () {
            sqlx::query("DELETE FROM nodes WHERE id = $1").bind(id.as_str()).execute(&mut *tx.c).await?;
            commit(())
        })
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
             WHERE lease_owner IS NOT NULL AND lease_until > $1 AND address >= $2 AND address < $3
               AND starts_with(address, $2) AND address > $4 AND ($5::text IS NULL OR lease_owner = $5)
             ORDER BY address LIMIT $6"
        );
        transaction!(self, true, |tx| -> Page<RunningTurn> {
            let now = tx.now().await?;
            let rows = sqlx::query(AssertSqlSafe(sql.clone()))
                .bind(now)
                .bind(prefix)
                .bind(upper_bound(prefix))
                .bind(after)
                .bind(node.map(NodeId::as_str))
                .bind(limit as i64 + 1)
                .fetch_all(&mut *tx.c)
                .await?;
            let turns = rows.iter().map(running_turn).collect::<Result<Vec<_>, _>>()?;
            commit(support::page_of(turns, limit, |t| PageCursor(t.actor.to_string())))
        })
    }

    async fn running_turn(&self, address: &ActorAddress) -> StoreResult<Option<RunningTurn>> {
        let sql = format!(
            "SELECT {RUNNING_TURN} FROM actors WHERE address = $1 AND lease_owner IS NOT NULL AND lease_until > $2"
        );
        transaction!(self, true, |tx| -> Option<RunningTurn> {
            let now = tx.now().await?;
            let row = sqlx::query(AssertSqlSafe(sql.clone()))
                .bind(address.as_str())
                .bind(now)
                .fetch_optional(&mut *tx.c)
                .await?;
            commit(row.as_ref().map(running_turn).transpose()?)
        })
    }

    async fn request_cancel(&self, address: &ActorAddress) -> StoreResult<bool> {
        transaction!(self, false, |tx| -> bool {
            if lock_actor(tx, address.as_str()).await?.is_none() {
                return commit(false);
            }
            let now = tx.now().await?;
            let updated = sqlx::query(
                "UPDATE actors SET cancel_epoch = epoch WHERE address = $1 AND lease_owner IS NOT NULL AND lease_until > $2",
            )
            .bind(address.as_str())
            .bind(now)
            .execute(&mut *tx.c)
            .await?;
            commit(updated.rows_affected() == 1)
        })
    }

    // ---- commit ----

    async fn commit(&self, request: CommitRequest) -> StoreResult<CommitResult> {
        support::check_creates(&request.creates)?;
        support::check_controls(&request.address, &request.controls)?;
        let r = &request;
        transaction!(self, false, |tx| -> CommitResult {
            if r.cancels.contains(&r.message_id) {
                return Err(invalid(format!("A turn cannot cancel its own message {}", r.message_id)));
            }
            // The watchers of the actors of an event (a failure, a stop, the deletes) are read first, to be locked with
            // the rest.
            let failing = matches!(r.outcome, TurnOutcome::Failed(_));
            let stopping = matches!(r.state, StateChange::Stop);
            let deletes: BTreeSet<&str> = r.deletes().map(ActorAddress::as_str).collect();
            let mut first_targets = if failing { watch::failure_targets(&r.address) } else { Vec::new() };
            if stopping && !failing {
                first_targets.push(r.address.to_string());
            }
            first_targets.extend(deletes.iter().map(|a| (*a).to_owned()));
            let watchers = watchers_of(tx, &first_targets).await?;
            // Lock every instance this commit touches, in address order, so that concurrent commits cannot deadlock.
            let touched: BTreeSet<&str> = std::iter::once(r.address.as_str())
                .chain(r.creates.iter().map(|c| c.address.as_str()))
                .chain(r.sends.iter().map(|s| s.to.as_str()))
                .chain(r.controls.iter().map(|c| c.actor.as_str()))
                .chain(watchers.iter().map(String::as_str))
                .collect();
            let touched: Vec<&str> = touched.into_iter().collect();
            let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
                "SELECT address, type, parent FROM actors WHERE address = ANY ($1) ORDER BY address FOR UPDATE",
            )
            .bind(&touched)
            .fetch_all(&mut *tx.c)
            .await?;
            let mut types = HashMap::new();
            let mut parents = HashMap::new();
            let mut locked = HashSet::new();
            for (address, t, parent) in rows {
                locked.insert(address.clone());
                types.insert(address.clone(), t);
                parents.insert(address, parent);
            }
            let claimed: Option<Option<String>> = sqlx::query_scalar(
                "SELECT lease_message FROM actors WHERE address = $1 AND epoch = $2 AND lease_owner IS NOT NULL",
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

            // Hold the registrations of every created instance's type in place until this transaction ends, so that
            // `remove_type` either sees the new instances or runs first and this commit is rejected. Before the clock
            // is read, like every other lock.
            let mut created_types: Vec<&str> = r.creates.iter().map(|c| c.address.type_name().as_str()).collect();
            created_types.sort_unstable();
            created_types.dedup();
            if !created_types.is_empty() {
                let registered: Vec<String> =
                    sqlx::query_scalar("SELECT name FROM actor_types WHERE name = ANY ($1) ORDER BY name FOR SHARE")
                        .bind(&created_types)
                        .fetch_all(&mut *tx.c)
                        .await?;
                if registered.len() < created_types.len() {
                    let missing = r
                        .creates
                        .iter()
                        .find(|c| !registered.iter().any(|name| name == c.address.type_name().as_str()))
                        .map(|c| c.address.clone())
                        .expect("a created type is missing");
                    return Ok(Done::Rollback(CommitResult::Rejected(format!(
                        "Create of {missing}: its type is not registered"
                    ))));
                }
            }
            // A delete of an ancestor would remove this instance. Its ancestors stay while its row is locked: removing
            // one removes it too.
            if !deletes.is_empty() {
                let named: Vec<&str> = deletes.iter().copied().collect();
                let ancestors: Vec<String> = sqlx::query_scalar(
                    "WITH RECURSIVE up (address) AS (
                         SELECT parent FROM actors WHERE address = $1
                         UNION
                         SELECT actors.parent FROM actors JOIN up ON actors.address = up.address
                     )
                     SELECT address FROM up WHERE address = ANY ($2)",
                )
                .bind(r.address.as_str())
                .bind(&named)
                .fetch_all(&mut *tx.c)
                .await?;
                if let Some(ancestor) = r.deletes().find(|d| ancestors.iter().any(|a| a == d.as_str())) {
                    let reason = support::ancestor_delete_reason(ancestor, &r.address);
                    return Ok(Done::Rollback(CommitResult::Rejected(reason)));
                }
            }

            // 3. deletes: the tree of each actor a delete names that is there, locked level by level after the rows
            // above, before the clock is read.
            let mut deleted: BTreeSet<String> = BTreeSet::new();
            for target in &deletes {
                if locked.contains(*target) && !deleted.contains(*target) {
                    deleted.extend(lock_tree(tx, target).await?);
                }
            }
            // 4. creates: an instance that did not exist when locking is inserted and locked now, *before* the clock is
            // read. A concurrent transaction creating (and enqueueing to) the same instance is thus waited for here, so
            // the clock reading below follows it and our sends cannot sort before its messages. For an address that
            // the commit also deletes, the row this inserts is its own: it is the create, and nothing is removed. A
            // row that another caller created in between is locked now, removed with its tree and created again below.
            let mut creates: Vec<&ActorCreate> = r.creates.iter().collect();
            creates.sort_by(|a, b| a.address.as_str().cmp(b.address.as_str()));
            for create in creates {
                let at = create.address.as_str();
                if !types.contains_key(at) {
                    let parent = create.child.then_some(r.address.as_str());
                    let (t, parent, inserted) = create_and_lock(tx, create, parent).await?;
                    locked.insert(at.to_owned());
                    types.insert(at.to_owned(), t);
                    parents.insert(at.to_owned(), parent);
                    if !inserted && deletes.contains(at) {
                        deleted.extend(lock_tree(tx, at).await?);
                    }
                }
            }
            locked.extend(deleted.iter().cloned());
            // A child create never takes over an instance: one that exists after the deletes with another parent or
            // none rejects the commit. Only this commit can create a child of its own instance, whose lock it holds.
            let foreign =
                r.creates.iter().filter(|c| c.child && !deleted.contains(c.address.as_str())).find(|c| {
                    parents.get(c.address.as_str()).cloned().flatten().as_deref() != Some(r.address.as_str())
                });
            if let Some(create) = foreign {
                return Ok(Done::Rollback(CommitResult::Rejected(support::foreign_child_reason(
                    &create.address,
                    &r.address,
                ))));
            }
            // A stop removes the tree as it is after the deletes and the creates, locked now, before the clock is read:
            // the children this commit creates are in it, also those it creates again after a delete; what the deletes
            // remove is not, also when this commit creates it again as an independent instance.
            let tree: Vec<String> = match r.state {
                StateChange::Stop => {
                    let mut tree: BTreeSet<String> =
                        lock_tree(tx, r.address.as_str()).await?.into_iter().filter(|a| !deleted.contains(a)).collect();
                    let again = r.creates.iter().filter(|c| c.child && deleted.contains(c.address.as_str()));
                    tree.extend(again.map(|c| c.address.to_string()));
                    tree.into_iter().collect()
                }
                _ => Vec::new(),
            };
            locked.extend(tree.iter().cloned());
            // Every removed actor. The watchers of those whose watchers were not read first are locked after the trees.
            let removed: Vec<String> =
                deleted.iter().chain(&tree).cloned().collect::<BTreeSet<_>>().into_iter().collect();
            let later: Vec<String> = removed.iter().filter(|a| !first_targets.contains(a)).cloned().collect();
            lock_watchers_of(tx, &later, &mut locked).await?;
            // The turn's watches, in the order of its calls, under this instance's lock; then the watches of the event
            // are read again.
            for change in &r.watches {
                let sql = if change.set {
                    "INSERT INTO watches (target, watcher) VALUES ($1, $2) ON CONFLICT (target, watcher) DO NOTHING"
                } else {
                    "DELETE FROM watches WHERE target = $1 AND watcher = $2"
                };
                sqlx::query(sql).bind(change.target.as_str()).bind(r.address.as_str()).execute(&mut *tx.c).await?;
            }
            let mut event_targets = if failing { watch::failure_targets(&r.address) } else { Vec::new() };
            let more: Vec<String> = removed.iter().filter(|a| !event_targets.contains(a)).cloned().collect();
            event_targets.extend(more);
            let (watches, gone) = reread_watches(tx, &event_targets, &locked).await?;

            // Validate before applying the rest (S2); a rejection rolls back the creates and leaves the lease as it is.
            let mut send_ids = HashSet::new();
            for send in &r.sends {
                let to = send.to.as_str();
                let created = r.creates.iter().any(|c| c.address == send.to);
                if !types.contains_key(to) || (deleted.contains(to) && !created) {
                    return Ok(Done::Rollback(CommitResult::Rejected(format!("Send to absent instance {}", send.to))));
                }
                if !send_ids.insert(&send.id) || send.id == r.message_id || is_id_in_use(tx, &send.id).await? {
                    return Ok(Done::Rollback(CommitResult::Rejected(format!(
                        "Message id {} is already in use",
                        send.id
                    ))));
                }
            }

            // 1. outcome, or back to pending
            let row = sqlx::query(concat!("SELECT ", message_columns!(), " FROM messages WHERE id = $1"))
                .bind(r.message_id.as_str())
                .fetch_optional(&mut *tx.c)
                .await?
                .ok_or_else(|| corrupt(format!("the claimed message {} is not pending", r.message_id)))?;
            let claimed_message = message(&row)?;
            let failed = match &r.outcome {
                TurnOutcome::Failed(error) if !claimed_message.notification => {
                    Some((&claimed_message, error.as_str(), r.cause))
                }
                _ => None,
            };
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
                let row = sqlx::query(concat!(
                    "SELECT ",
                    message_columns!(),
                    " FROM messages WHERE id = $1 AND actor_address = $2"
                ))
                .bind(id.as_str())
                .bind(r.address.as_str())
                .fetch_optional(&mut *tx.c)
                .await?;
                if let Some(row) = row {
                    finish(tx, &message(&row)?, OutcomeStatus::Cancelled, None).await?;
                    cancelled.push(id.clone());
                }
            }
            // 3. deletes, and 4. the creates of what they remove. An instance that is removed and created again keeps
            // its row, which starts a new life: its mailbox is purged, its watches as a watcher go and every column is
            // set as a create sets it. A transaction that waits for the row's lock then finds the new life, where a
            // deleted row would be skipped while the inserted one is not yet visible to it.
            let mut again: Vec<&ActorCreate> = Vec::new();
            for create in r.creates.iter().filter(|c| deleted.contains(c.address.as_str())) {
                if !again.iter().any(|c| c.address == create.address) {
                    again.push(create);
                }
            }
            let removed_rows: Vec<String> =
                deleted.iter().filter(|a| !again.iter().any(|c| c.address.as_str() == a.as_str())).cloned().collect();
            remove_tree(tx, &removed_rows).await?;
            for create in again {
                let at = create.address.as_str();
                purge_mailbox(tx, at).await?;
                sqlx::query("DELETE FROM watches WHERE watcher = $1").bind(at).execute(&mut *tx.c).await?;
                let now = tx.now().await?;
                sqlx::query(
                    "UPDATE actors SET state = $2, version = 0, epoch = 0, lease_owner = NULL, lease_until = NULL,
                         lease_message = NULL, cancel_epoch = NULL, paused = false, next_due_at = NULL, parent = $3,
                         created_at = $4
                     WHERE address = $1",
                )
                .bind(at)
                .bind(&create.state)
                .bind(create.child.then_some(r.address.as_str()))
                .bind(now)
                .execute(&mut *tx.c)
                .await?;
            }
            // 5. purges and 6. pauses and resumes, of the rows this commit holds; a resume wakes the nodes of the type
            for target in support::purges(&r.controls).into_iter().filter(|t| locked.contains(t.as_str())) {
                let own = (*target == r.address).then_some(r.message_id.as_str());
                purge_in(tx, target.as_str(), own).await?;
            }
            for (target, paused) in support::pauses(&r.controls) {
                if !locked.contains(target.as_str()) {
                    continue;
                }
                let t: Option<String> =
                    sqlx::query_scalar("UPDATE actors SET paused = $1 WHERE address = $2 RETURNING type")
                        .bind(paused)
                        .bind(target.as_str())
                        .fetch_optional(&mut *tx.c)
                        .await?;
                if let (Some(t), false) = (t, paused) {
                    tx.notify(t);
                }
            }
            // 7. sends
            let mut sent = Vec::with_capacity(r.sends.len());
            for send in &r.sends {
                // A concurrent enqueue may have taken the id since the check above: undo everything.
                let Some(stored) = insert(tx, send, false).await? else {
                    return Ok(Done::Rollback(CommitResult::Rejected(format!(
                        "Message id {} is already in use",
                        send.id
                    ))));
                };
                tx.notify(types[send.to.as_str()].clone());
                sent.push(stored);
            }
            // 8. state, 9. release, 10. the notifications of the event
            let removal = Removal::of_commit(&r.address, &r.message_id, &r.controls, stopping);
            let removals = (!removed.is_empty()).then_some((&removal, &removed[..]));
            let notices = notices(tx, &watches, &gone, failed, removals).await?;
            match &r.state {
                StateChange::Unchanged => {}
                StateChange::NewState(state) => {
                    sqlx::query("UPDATE actors SET state = $1, version = version + 1 WHERE address = $2")
                        .bind(state)
                        .bind(r.address.as_str())
                        .execute(&mut *tx.c)
                        .await?;
                }
                StateChange::Stop => {
                    let version: i64 = sqlx::query_scalar("SELECT version FROM actors WHERE address = $1")
                        .bind(r.address.as_str())
                        .fetch_one(&mut *tx.c)
                        .await?;
                    remove_tree(tx, &tree).await?;
                    notify(tx, &r.event, notices).await?;
                    return commit(CommitResult::Committed { version, sent, cancelled });
                }
            }
            refresh_next_due(tx, r.address.as_str()).await?;
            let (version, has_more): (i64, bool) = sqlx::query_as(
                "UPDATE actors SET lease_owner = NULL, lease_until = NULL, lease_message = NULL WHERE address = $1
                 RETURNING version, next_due_at IS NOT NULL",
            )
            .bind(r.address.as_str())
            .fetch_one(&mut *tx.c)
            .await?;
            if has_more {
                tx.notify(types[r.address.as_str()].clone());
            }
            notify(tx, &r.event, notices).await?;
            commit(CommitResult::Committed { version, sent, cancelled })
        })
    }

    // ---- wake-ups ----

    /// Yields when a `NOTIFY` for one of `types` arrives on this store's channel (from any node, including this one),
    /// and after the listener reconnected. The first call starts the shared listener connection.
    fn wake_ups(&self, types: &BTreeSet<TypeName>) -> Option<BoxStream<'static, ()>> {
        let names: Arc<HashSet<String>> = Arc::new(types.iter().map(|t| t.as_str().to_owned()).collect());
        let receiver = self.listener.subscribe();
        self.listener.start();
        Some(
            futures::stream::unfold(receiver, move |mut receiver| {
                let names = names.clone();
                async move {
                    loop {
                        match receiver.recv().await {
                            Ok(payload) if payload.is_empty() || names.contains(&payload) => {
                                return Some(((), receiver));
                            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(url: &str) -> String {
        match connect_options(url) {
            Err(StoreError::Invalid(message)) => message,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn a_postgres_url_yields_its_location_and_user() {
        let options = connect_options(" postgres://app:s3cr%40t@db.example:6543/orders?sslmode=require ").unwrap();
        assert_eq!(options.get_host(), "db.example");
        assert_eq!(options.get_port(), 6543);
        assert_eq!(options.get_username(), "app");
        assert_eq!(options.get_database(), Some("orders"));
        assert_eq!(connect_options("postgresql://actors@localhost/actors").unwrap().get_username(), "actors");
    }

    #[test]
    fn a_refused_url_is_never_echoed() {
        let urls = [
            "mysql://app:secret@db.example/actors",
            "jdbc:postgresql://db.example/actors?user=app&password=secret",
            "db.example/secret",
            "postgres://app:secret@db.example:notaport/actors",
            "postgres://db.example:notaport/actors?password=secret",
            "postgres://app:secret@db.example/actors?sslmode=secret",
            "postgres://app:secret@[db.example/actors",
        ];
        for url in urls {
            let message = refused(url);
            assert!(!message.contains("secret"), "{url}: {message}");
            assert!(!message.contains("db.example"), "{url}: {message}");
        }
    }
}
