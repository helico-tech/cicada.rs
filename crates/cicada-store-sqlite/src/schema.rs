//! The SQLite schema: the four tables of the architecture plus a `sequences` table for the store-wide `seq` and epoch
//! counters and the store clock's high-water mark. All instants are INTEGER microseconds since the epoch; JSON values
//! are TEXT.
//!
//! The schema is versioned with `PRAGMA user_version` and is the Kotlin implementation's, statement for statement
//! (ADR 0004): opening a store applies every missing migration in one transaction and refuses a database written by a
//! newer version. Migrations are additive so that nodes of the previous version keep working during a rolling deploy:
//! version 3 keeps `outcomes.retried_as` (written and read only by version 2) as an unused nullable column. Version 4
//! adds the nullable `messages.retry_policy`. Version 5 adds the nullable `caused_by` to messages and outcomes. Version
//! 6 adds the message's retention policy and the outcome's visibility (`visible_until`, `keep_last`, `stripped`).
//!
//! The statement texts are those of `Schema.kt` after `trimIndent()`, because SQLite keeps the text of every `CREATE`
//! in `sqlite_master`: identical text gives identical databases.

use cicada::store::{StoreError, StoreResult};
use sqlx::{Row, SqliteConnection};

use crate::conn::{Conn, Mode, db_error};

/// Ordered migrations; index + 1 is the version. Never edit one, append a new one (and the same one in Kotlin).
const MIGRATIONS: &[&[&str]] = &[
    // 1: initial schema
    &[
        "CREATE TABLE actor_types (
    template      TEXT PRIMARY KEY,
    shape         TEXT NOT NULL UNIQUE,
    contract      TEXT NOT NULL,
    contract_hash TEXT NOT NULL
)",
        "CREATE TABLE actors (
    uri           TEXT PRIMARY KEY,
    type          TEXT NOT NULL,
    state         TEXT,
    version       INTEGER NOT NULL DEFAULT 0,
    epoch         INTEGER NOT NULL DEFAULT 0,
    lease_owner   TEXT,
    lease_until   INTEGER,
    lease_message TEXT,
    next_due_at   INTEGER,
    created_at    INTEGER NOT NULL
)",
        "CREATE INDEX actors_due ON actors (type, next_due_at) WHERE next_due_at IS NOT NULL",
        "CREATE TABLE messages (
    id             TEXT PRIMARY KEY,
    actor_uri      TEXT NOT NULL,
    seq            INTEGER NOT NULL UNIQUE,
    type           TEXT NOT NULL,
    payload        TEXT NOT NULL,
    sender         TEXT,
    correlation_id TEXT,
    headers        TEXT NOT NULL,
    deliver_at     INTEGER NOT NULL,
    attempt        INTEGER NOT NULL DEFAULT 0
)",
        "CREATE INDEX messages_mailbox ON messages (actor_uri, deliver_at, seq)",
        "CREATE TABLE outcomes (
    message_id  TEXT PRIMARY KEY,
    actor_uri   TEXT NOT NULL,
    status      TEXT NOT NULL,
    error       TEXT,
    finished_at INTEGER NOT NULL,
    envelope    TEXT NOT NULL
)",
        "CREATE INDEX outcomes_failures ON outcomes (status, finished_at, message_id)",
        "CREATE INDEX outcomes_finished ON outcomes (finished_at)",
        "CREATE TABLE sequences (name TEXT PRIMARY KEY, value INTEGER NOT NULL)",
        "INSERT INTO sequences (name, value) VALUES ('seq', 0), ('epoch', 0)",
    ],
    // 2 (T18): the retry a `done` outcome was replaced by; the store clock's high-water mark (microseconds).
    &["ALTER TABLE outcomes ADD COLUMN retried_as TEXT", "INSERT INTO sequences (name, value) VALUES ('clock', 0)"],
    // 3 (Retry v2): a failed message is rescheduled in place, so the message records its failures. Additive only:
    // `outcomes.retried_as` stays, unused, because nodes still on version 2 write and read it during a rolling deploy.
    &[
        "ALTER TABLE messages ADD COLUMN failures INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE messages ADD COLUMN last_error TEXT",
    ],
    // 4 (Retry v3): the sender's retry policy travels with the message (JSON, null for no retry).
    &["ALTER TABLE messages ADD COLUMN retry_policy TEXT"],
    // 5 (H1): the message whose turn sent a message, for message chains; indexed to find what a message caused.
    &[
        "ALTER TABLE messages ADD COLUMN caused_by TEXT",
        "ALTER TABLE outcomes ADD COLUMN caused_by TEXT",
        "CREATE INDEX messages_caused_by ON messages (caused_by) WHERE caused_by IS NOT NULL",
        "CREATE INDEX outcomes_caused_by ON outcomes (caused_by) WHERE caused_by IS NOT NULL",
    ],
    // 6 (H2): the sender's retention policy travels with the message; an outcome is visible until `visible_until`
    // (null: for the store retention), `keep_last` outcomes are trimmed per instance and status in `keep_seq` order
    // (from the store-wide sequence), and `stripped` marks a hidden record whose content the sweep dropped.
    &[
        "ALTER TABLE messages ADD COLUMN retention_policy TEXT",
        "ALTER TABLE outcomes ADD COLUMN visible_until INTEGER",
        "ALTER TABLE outcomes ADD COLUMN keep_last INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE outcomes ADD COLUMN keep_seq INTEGER",
        "ALTER TABLE outcomes ADD COLUMN stripped INTEGER NOT NULL DEFAULT 0",
        "CREATE INDEX outcomes_keep_last ON outcomes (actor_uri, status, keep_seq) WHERE keep_last = 1",
        "CREATE INDEX outcomes_visible_until ON outcomes (visible_until) WHERE visible_until IS NOT NULL",
    ],
];

/// The schema version this code writes.
pub const VERSION: i64 = MIGRATIONS.len() as i64;

/// The statements of `version` alone, for tests that build an old database.
pub fn statements_of(version: usize) -> &'static [&'static str] {
    MIGRATIONS[version - 1]
}

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
