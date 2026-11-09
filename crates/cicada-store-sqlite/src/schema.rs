//! The SQLite schema: the tables of the architecture plus a `sequences` table for the store-wide `seq` and epoch
//! counters and the store clock's high-water mark. All instants are INTEGER microseconds since the epoch; JSON values
//! are TEXT.
//!
//! The schema is versioned with `PRAGMA user_version`: opening a store applies every missing migration in one
//! transaction and refuses a database written by a newer version. Migrations are additive so that nodes of the previous
//! version keep working during a rolling deploy.
//!
//! SQLite keeps the text of every `CREATE` in `sqlite_master`, so the statement texts are part of the schema.

use cicada::store::{StoreError, StoreResult};
use sqlx::{Row, SqliteConnection};

use crate::conn::{Conn, Mode, db_error};

/// Ordered migrations; index + 1 is the version. Never edit one, append a new one.
const MIGRATIONS: &[&[&str]] = &[
    // 1: the schema. The `clock` sequence is the store clock's high-water mark (microseconds). A message records its
    // failures (it is rescheduled in place), the sender's retry and retention policies (JSON, null for none) and the
    // message whose turn sent it (`caused_by`, indexed; an outcome keeps it in its own column). An outcome is visible until `visible_until` (null: for the store
    // retention), `keep_last` outcomes are trimmed per instance and status in `keep_seq` order (from the store-wide
    // sequence), and `stripped` marks a hidden record whose content the sweep dropped. `actors.cancel_epoch` is the epoch
    // an operator asked to cancel (only that turn), and a paused actor is left out of `actors_due` and `claim_due`.
    // `nodes` is display only: one row per node, with hosted types as a JSON array.
    &[
        "CREATE TABLE actor_types (
    name          TEXT PRIMARY KEY,
    contract      TEXT NOT NULL,
    contract_hash TEXT NOT NULL
)",
        "CREATE TABLE actors (
    address       TEXT PRIMARY KEY,
    type          TEXT NOT NULL,
    state         TEXT,
    version       INTEGER NOT NULL DEFAULT 0,
    epoch         INTEGER NOT NULL DEFAULT 0,
    lease_owner   TEXT,
    lease_until   INTEGER,
    lease_message TEXT,
    cancel_epoch  INTEGER,
    paused        INTEGER NOT NULL DEFAULT 0,
    next_due_at   INTEGER,
    created_at    INTEGER NOT NULL
)",
        // The claim scan: unpaused instances with a pending message. Claims and `next_due_at` state `paused = 0` literally,
        // so SQLite can prove this partial index applies.
        "CREATE INDEX actors_due ON actors (type, next_due_at) WHERE next_due_at IS NOT NULL AND paused = 0",
        // The running turns listing.
        "CREATE INDEX actors_running ON actors (address) WHERE lease_owner IS NOT NULL",
        "CREATE TABLE messages (
    id               TEXT PRIMARY KEY,
    actor_address    TEXT NOT NULL,
    seq              INTEGER NOT NULL UNIQUE,
    type             TEXT NOT NULL,
    payload          TEXT NOT NULL,
    sender           TEXT,
    correlation_id   TEXT,
    headers          TEXT NOT NULL,
    deliver_at       INTEGER NOT NULL,
    attempt          INTEGER NOT NULL DEFAULT 0,
    failures         INTEGER NOT NULL DEFAULT 0,
    last_error       TEXT,
    retry_policy     TEXT,
    caused_by        TEXT,
    retention_policy TEXT
)",
        "CREATE INDEX messages_mailbox ON messages (actor_address, deliver_at, seq)",
        "CREATE INDEX messages_caused_by ON messages (caused_by) WHERE caused_by IS NOT NULL",
        "CREATE TABLE outcomes (
    message_id    TEXT PRIMARY KEY,
    actor_address TEXT NOT NULL,
    status        TEXT NOT NULL,
    error         TEXT,
    finished_at   INTEGER NOT NULL,
    envelope      TEXT NOT NULL,
    caused_by     TEXT,
    visible_until INTEGER,
    keep_last     INTEGER NOT NULL DEFAULT 0,
    keep_seq      INTEGER,
    stripped      INTEGER NOT NULL DEFAULT 0
)",
        // The outcomes listing: by time (`outcomes_finished`), and by status and time (`outcomes_failures`).
        "CREATE INDEX outcomes_failures ON outcomes (status, finished_at, message_id)",
        "CREATE INDEX outcomes_finished ON outcomes (finished_at, message_id)",
        "CREATE INDEX outcomes_caused_by ON outcomes (caused_by) WHERE caused_by IS NOT NULL",
        "CREATE INDEX outcomes_keep_last ON outcomes (actor_address, status, keep_seq) WHERE keep_last = 1",
        "CREATE INDEX outcomes_visible_until ON outcomes (visible_until) WHERE visible_until IS NOT NULL",
        "CREATE TABLE nodes (
    id         TEXT PRIMARY KEY,
    hosted     TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL
)",
        "CREATE TABLE sequences (name TEXT PRIMARY KEY, value INTEGER NOT NULL)",
        "INSERT INTO sequences (name, value) VALUES ('seq', 0), ('epoch', 0), ('clock', 0)",
    ],
];

/// The schema version this code writes.
pub const VERSION: i64 = MIGRATIONS.len() as i64;

/// Applies every missing migration in one write transaction; refuses a database written by a newer version.
pub(crate) async fn migrate(conn: &mut Conn) -> StoreResult<()> {
    conn.begin(Mode::Write).await.map_err(db_error)?;
    let result = apply(conn.raw()).await;
    conn.end(result).await
}

async fn apply(c: &mut SqliteConnection) -> StoreResult<()> {
    let current: i64 = sqlx::query("PRAGMA user_version").fetch_one(&mut *c).await.map_err(db_error)?.get(0);
    if current > VERSION {
        return Err(StoreError::Invalid(format!(
            "Database schema version {current} is newer than supported ({VERSION})"
        )));
    }
    if current < VERSION {
        for statements in &MIGRATIONS[current as usize..] {
            for statement in *statements {
                sqlx::raw_sql(*statement).execute(&mut *c).await.map_err(db_error)?;
            }
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("PRAGMA user_version = {VERSION}")))
            .execute(&mut *c)
            .await
            .map_err(db_error)?;
    }
    Ok(())
}
