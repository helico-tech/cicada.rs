//! The SQLite schema: the tables of the architecture plus a `sequences` table for the store-wide `seq` and epoch
//! counters and the store clock's high-water mark. All instants are INTEGER microseconds since the epoch; JSON values
//! are TEXT.
//!
//! The schema is versioned with `PRAGMA user_version` and is at version 5: opening a store applies every missing
//! migration in one transaction and refuses a database written by a newer version. A database without a version is set
//! up only when it has no tables; one with tables belongs to another program and is refused (ADR 0017). Migrations are
//! additive so that nodes of the previous version keep working during a rolling deploy, with four exceptions, after
//! which every old node is stopped before the first new one starts: migration 2 makes `actors.state` required, so a
//! node of version 1 that writes an actor without a state fails; migration 3 removes every registration and rewrites
//! `%3A` to `:` in addresses, and the contracts that nodes register again are not those of version 2 (ADR 0021);
//! migration 4 adds `actors.parent`, and a node of version 3 that keeps running deletes or stops a parent without its
//! children, which then stay behind (ADR 0019); migration 5 adds the watches and the mark on a message, and a node of
//! version 4 that keeps running writes no notification for a message that fails there and leaves the watches of an
//! actor it removes behind (ADR 0023). A node of the previous version that opens the migrated database refuses it.
//!
//! A statement of a migration that violates a constraint (`SQLITE_CONSTRAINT`, such as two old addresses that become
//! one in migration 3) fails the open with [`StoreError::Invalid`], which names the version it could not reach and
//! holds SQLite's message: trying again cannot help. Every other error, such as a database that is locked, is
//! [`StoreError::Unavailable`]. Either way the transaction is rolled back and the database stays at its version.
//!
//! SQLite keeps the text of every `CREATE` in `sqlite_master`, so the statement texts are part of the schema.

use std::path::Path;

use crate::store::{StoreError, StoreResult};
use sqlx::{Row, SqliteConnection};

use crate::store::sqlite::conn::{Conn, Mode, db_error};

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
    // 2: every actor has a state (ADR 0018). A state that is absent or JSON null becomes `{}`. A registration whose
    // contract has no state schema (a stateless type) is removed, so that a node hosting the type registers it again
    // with its new contract. `actors.state` becomes NOT NULL: SQLite cannot add that to a column, so the table is
    // rebuilt with the same columns and its indexes are created again. Not additive (see the module docs).
    &[
        "UPDATE actors SET state = '{}' WHERE state IS NULL OR json_type(state) = 'null'",
        "DELETE FROM actor_types WHERE json_extract(contract, '$.state') IS NULL",
        "CREATE TABLE actors_v2 (
    address       TEXT PRIMARY KEY,
    type          TEXT NOT NULL,
    state         TEXT NOT NULL,
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
        "INSERT INTO actors_v2 (address, type, state, version, epoch, lease_owner, lease_until, lease_message, cancel_epoch,
    paused, next_due_at, created_at)
SELECT address, type, state, version, epoch, lease_owner, lease_until, lease_message, cancel_epoch, paused, next_due_at,
    created_at FROM actors",
        "DROP TABLE actors",
        "ALTER TABLE actors_v2 RENAME TO actors",
        "CREATE INDEX actors_due ON actors (type, next_due_at) WHERE next_due_at IS NOT NULL AND paused = 0",
        "CREATE INDEX actors_running ON actors (address) WHERE lease_owner IS NOT NULL",
    ],
    // 3: a key is one name (ADR 0021). Key fields leave every contract, so every hash changes: every registration is
    // removed, and a node that hosts a type registers it again when it starts. A `:` in a name is written as it is, so
    // `%3A` becomes `:` in the address columns that SQL matches on; the other stored addresses are read through
    // `ActorAddress::parse`, which writes the new spelling. Two old addresses that become one fail the migration on
    // the primary key of `actors`. Not additive (see the module docs).
    &[
        "DELETE FROM actor_types",
        "UPDATE actors SET address = replace(address, '%3A', ':') WHERE instr(address, '%3A') > 0",
        "UPDATE messages SET actor_address = replace(actor_address, '%3A', ':') WHERE instr(actor_address, '%3A') > 0",
        "UPDATE outcomes SET actor_address = replace(actor_address, '%3A', ':') WHERE instr(actor_address, '%3A') > 0",
    ],
    // 4: child actors (ADR 0019). `parent` is the instance whose commit created this one as a child, null for an
    // independent one, so every existing actor is independent. No foreign key: the store removes a tree itself, as
    // every removed mailbox needs its `purged` outcomes. `actors_parent` serves the listing of children and the walk
    // of a removal. Not additive (see the module docs).
    &[
        "ALTER TABLE actors ADD COLUMN parent TEXT",
        "CREATE INDEX actors_parent ON actors (parent, address) WHERE parent IS NOT NULL",
    ],
    // 5: watches (ADR 0018, ADR 0023). A watch is the pair of a target (an address, a type prefix or `*`) and a
    // watcher; the primary key serves the lookup of the watchers of an event, `watches_watcher` the listing by watcher
    // and the removal of a watcher's watches. No foreign keys, as elsewhere. `messages.notification` marks a message the
    // store wrote as a notification; every existing message is none, and an outcome keeps the mark in its envelope,
    // where an older envelope without it reads as none. Not additive (see the module docs).
    &[
        "CREATE TABLE watches (
    target  TEXT NOT NULL,
    watcher TEXT NOT NULL,
    PRIMARY KEY (target, watcher)
)",
        "CREATE INDEX watches_watcher ON watches (watcher, target)",
        "ALTER TABLE messages ADD COLUMN notification INTEGER NOT NULL DEFAULT 0",
    ],
];

/// The schema version this code writes.
pub const VERSION: i64 = MIGRATIONS.len() as i64;

/// Applies every missing migration in one write transaction; refuses a database written by a newer version, and one
/// at `path` without a version that already holds tables.
pub(crate) async fn migrate(conn: &mut Conn, path: &Path) -> StoreResult<()> {
    conn.begin(Mode::Write).await.map_err(db_error)?;
    let result = apply(conn.raw(), path).await;
    conn.end(result).await
}

async fn apply(c: &mut SqliteConnection, path: &Path) -> StoreResult<()> {
    apply_up_to(c, path, VERSION).await
}

/// Applies the migrations up to `target`; [`apply`] with every migration.
async fn apply_up_to(c: &mut SqliteConnection, path: &Path, target: i64) -> StoreResult<()> {
    let current: i64 = sqlx::query("PRAGMA user_version").fetch_one(&mut *c).await.map_err(db_error)?.get(0);
    if current > VERSION {
        return Err(StoreError::Invalid(format!(
            "Database schema version {current} is newer than supported ({VERSION})"
        )));
    }
    if current == 0 {
        // SQLite's own tables (`sqlite_sequence`, `sqlite_stat1`) do not count.
        let foreign: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND substr(name, 1, 7) <> 'sqlite_' LIMIT 1",
        )
        .fetch_optional(&mut *c)
        .await
        .map_err(db_error)?;
        if foreign.is_some() {
            return Err(StoreError::Invalid(format!(
                "Database {} is not a cicada store: it holds other tables",
                path.display()
            )));
        }
    }
    if current < target {
        for (version, statements) in (current + 1..).zip(&MIGRATIONS[current as usize..target as usize]) {
            for statement in *statements {
                sqlx::raw_sql(*statement).execute(&mut *c).await.map_err(|e| migration_error(version, e))?;
            }
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("PRAGMA user_version = {target}")))
            .execute(&mut *c)
            .await
            .map_err(db_error)?;
    }
    Ok(())
}

/// The error of a statement of migration `version`: a violated constraint (`SQLITE_CONSTRAINT`, 19, with its extended
/// codes) is [`StoreError::Invalid`], as the data stops the migration and trying again cannot help; anything else is
/// [`db_error`].
fn migration_error(version: i64, e: sqlx::Error) -> StoreError {
    match e.as_database_error() {
        Some(d) if d.code().and_then(|code| code.parse::<i32>().ok()).is_some_and(|code| code & 0xFF == 19) => {
            StoreError::Invalid(format!("The database cannot be migrated to schema version {version}: {}", d.message()))
        }
        _ => db_error(e),
    }
}

/// Sets up the database at `path` as a store of schema version `version` would have: the migrations up to it, in one
/// transaction. For tests of an upgrade.
#[cfg(test)]
pub(crate) async fn migrate_to(path: &Path, version: i64) -> StoreResult<()> {
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(path).create_if_missing(true);
    let mut conn = Conn::connect(&options).await?;
    conn.begin(Mode::Write).await.map_err(db_error)?;
    let result = apply_up_to(conn.raw(), path, version).await;
    let result = conn.end(result).await;
    conn.close().await;
    result
}
