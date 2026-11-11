//! The PostgreSQL schema and its versioned migrations; the schema is at version 5.
//!
//! The tables of the architecture (`actor_types`, `actors`, `messages`, `outcomes`, `nodes`) plus two sequences for the
//! store-wide `seq` and epoch counters. JSON values are `jsonb`, instants `timestamptz` (microsecond precision, the
//! store's precision). Keys and addresses use the `"C"` collation, so ordering and keyset paging are by code point, as in
//! every other backend, and prefix ranges can use the B-tree indexes.
//!
//! Migrations are applied by [`migrate`] on store start: idempotent, serialised between concurrently starting nodes
//! by a transaction-scoped advisory lock, and recorded in `cicada_schema_version`. A database written by a newer version
//! is refused. When the schema is already current, start runs no DDL at all, so a node may connect as a user without
//! `CREATE` privileges once the schema was migrated by one that has them.
//!
//! Migrations are additive so that nodes of the previous version keep working during a rolling deploy, with four
//! exceptions, after which every old node is stopped before the first new one starts: migration 2 makes `actors.state`
//! required, so a node of version 1 that writes an actor without a state fails; migration 3 removes every registration
//! and rewrites `%3A` to `:` in addresses, and the contracts that nodes register again are not those of version 2
//! (ADR 0021); migration 4 adds `actors.parent`, and a node of version 3 that keeps running deletes or stops a parent
//! without its children, which then stay behind (ADR 0019); migration 5 adds the watches and the mark on a message,
//! and a node of version 4 that keeps running writes no notification for a message that fails there and leaves the
//! watches of an actor it removes behind (ADR 0023). A node of the previous version that opens the migrated database
//! refuses it.
//!
//! A statement of a migration that violates a constraint (SQLSTATE class `23`, such as two old addresses that become one
//! in migration 3) fails with [`StoreError::Invalid`], which names the version it could not reach and holds
//! PostgreSQL's message: trying again cannot help. Every other error keeps its mapping: a lost connection, a deadlock
//! and a missing privilege (`42501`) stay [`StoreError::Unavailable`], so a node without `CREATE` privileges that
//! starts before the one that migrates gets through once the schema is migrated. Either way the transaction is rolled
//! back and the schema stays at its version.

use crate::store::StoreError;
use sqlx::{AssertSqlSafe, Connection, PgConnection};

use crate::store::postgres::error::{map_error, sql_state};

/// Ordered migrations; index + 1 is the version. `{s}` is replaced by the quoted schema name. Never edit one.
const MIGRATIONS: &[&[&str]] = &[
    // 1: the schema. A message records its failures (it is rescheduled in place), the sender's retry and retention
    // policies (null for none) and the message whose turn sent it (`caused_by`, indexed; an outcome keeps it in its own
    // column). An outcome is visible until
    // `visible_until` (null: for the store retention), `keep_last` outcomes are trimmed per instance and status in
    // `keep_seq` order (from `message_seq`), and `stripped` marks a hidden record whose content the sweep dropped.
    // `actors.cancel_epoch` is the epoch an operator asked to cancel, and a paused actor is left out of `actors_due`.
    // `nodes` is display only: one row per node, with hosted types as a JSON array.
    &[
        r#"CREATE TABLE {s}.actor_types (
            name          text COLLATE "C" PRIMARY KEY,
            contract      jsonb NOT NULL,
            contract_hash text NOT NULL
        )"#,
        r#"CREATE TABLE {s}.actors (
            address       text COLLATE "C" PRIMARY KEY,
            type          text COLLATE "C" NOT NULL,
            state         jsonb,
            version       bigint NOT NULL DEFAULT 0,
            epoch         bigint NOT NULL DEFAULT 0,
            lease_owner   text,
            lease_until   timestamptz,
            lease_message text COLLATE "C",
            cancel_epoch  bigint,
            paused        boolean NOT NULL DEFAULT false,
            next_due_at   timestamptz,
            created_at    timestamptz NOT NULL
        )"#,
        // The claim scan: unpaused instances with a pending message, in due order.
        "CREATE INDEX actors_due ON {s}.actors (next_due_at, type) WHERE next_due_at IS NOT NULL AND NOT paused",
        // The running turns listing.
        "CREATE INDEX actors_running ON {s}.actors (address) WHERE lease_owner IS NOT NULL",
        r#"CREATE TABLE {s}.messages (
            id               text COLLATE "C" PRIMARY KEY,
            actor_address    text COLLATE "C" NOT NULL,
            seq              bigint NOT NULL,
            type             text NOT NULL,
            payload          jsonb NOT NULL,
            sender           text,
            correlation_id   text,
            headers          jsonb NOT NULL,
            deliver_at       timestamptz NOT NULL,
            attempt          integer NOT NULL DEFAULT 0,
            failures         integer NOT NULL DEFAULT 0,
            last_error       text,
            retry_policy     jsonb,
            caused_by        text COLLATE "C",
            retention_policy jsonb
        )"#,
        "CREATE INDEX messages_mailbox ON {s}.messages (actor_address, deliver_at, seq)",
        "CREATE INDEX messages_caused_by ON {s}.messages (caused_by) WHERE caused_by IS NOT NULL",
        r#"CREATE TABLE {s}.outcomes (
            message_id    text COLLATE "C" PRIMARY KEY,
            actor_address text COLLATE "C" NOT NULL,
            status        text NOT NULL,
            error         text,
            finished_at   timestamptz NOT NULL,
            envelope      jsonb NOT NULL,
            caused_by     text COLLATE "C",
            visible_until timestamptz,
            keep_last     boolean NOT NULL DEFAULT false,
            keep_seq      bigint,
            stripped      boolean NOT NULL DEFAULT false
        )"#,
        // The outcomes listing without a status, and the retention sweep.
        "CREATE INDEX outcomes_finished ON {s}.outcomes (finished_at, message_id)",
        // Failed outcomes: by time across all instances, and by address prefix.
        "CREATE INDEX outcomes_failures ON {s}.outcomes (finished_at, message_id) WHERE status = 'failed'",
        "CREATE INDEX outcomes_failures_by_actor ON {s}.outcomes (actor_address, finished_at) WHERE status = 'failed'",
        "CREATE INDEX outcomes_caused_by ON {s}.outcomes (caused_by) WHERE caused_by IS NOT NULL",
        "CREATE INDEX outcomes_keep_last ON {s}.outcomes (actor_address, status, keep_seq) WHERE keep_last",
        "CREATE INDEX outcomes_visible_until ON {s}.outcomes (visible_until) WHERE visible_until IS NOT NULL",
        r#"CREATE TABLE {s}.nodes (
            id         text COLLATE "C" PRIMARY KEY,
            hosted     jsonb NOT NULL,
            started_at timestamptz NOT NULL,
            last_seen  timestamptz NOT NULL
        )"#,
        "CREATE SEQUENCE {s}.message_seq",
        "CREATE SEQUENCE {s}.epoch_seq",
    ],
    // 2: every actor has a state (ADR 0018). A state that is absent or JSON null becomes `{}`; a registration whose
    // contract has no state schema (a stateless type) is removed, so that a node hosting the type registers it again
    // with its new contract; `actors.state` becomes NOT NULL. Not additive (see the module docs).
    &[
        "UPDATE {s}.actors SET state = '{}'::jsonb WHERE state IS NULL OR state = 'null'::jsonb",
        "DELETE FROM {s}.actor_types WHERE contract->'state' IS NULL OR contract->'state' = 'null'::jsonb",
        "ALTER TABLE {s}.actors ALTER COLUMN state SET NOT NULL",
    ],
    // 3: a key is one name (ADR 0021). Key fields leave every contract, so every hash changes: every registration is
    // removed, and a node that hosts a type registers it again when it starts. A `:` in a name is written as it is, so
    // `%3A` becomes `:` in the address columns that SQL matches on; the other stored addresses are read through
    // `ActorAddress::parse`, which writes the new spelling. Two old addresses that become one fail the migration on
    // the primary key of `actors`. Not additive (see the module docs).
    &[
        "DELETE FROM {s}.actor_types",
        "UPDATE {s}.actors SET address = replace(address, '%3A', ':') WHERE strpos(address, '%3A') > 0",
        "UPDATE {s}.messages SET actor_address = replace(actor_address, '%3A', ':') WHERE strpos(actor_address, '%3A') > 0",
        "UPDATE {s}.outcomes SET actor_address = replace(actor_address, '%3A', ':') WHERE strpos(actor_address, '%3A') > 0",
    ],
    // 4: child actors (ADR 0019). `parent` is the instance whose commit created this one as a child, null for an
    // independent one, so every existing actor is independent. No foreign key: the store removes a tree itself, as
    // every removed mailbox needs its `purged` outcomes. `actors_parent` serves the listing of children and the walk
    // of a removal. Not additive (see the module docs).
    &[
        r#"ALTER TABLE {s}.actors ADD COLUMN parent text COLLATE "C""#,
        "CREATE INDEX actors_parent ON {s}.actors (parent, address) WHERE parent IS NOT NULL",
    ],
    // 5: watches (ADR 0018, ADR 0023). A watch is the pair of a target (an address, a type prefix or `*`) and a
    // watcher; the primary key serves the lookup of the watchers of an event, `watches_watcher` the listing by watcher
    // and the removal of a watcher's watches. No foreign keys, as elsewhere. `messages.notification` marks a message the
    // store wrote as a notification; every existing message is none, and an outcome keeps the mark in its envelope,
    // where an older envelope without it reads as none. Not additive (see the module docs).
    &[
        r#"CREATE TABLE {s}.watches (
            target  text COLLATE "C" NOT NULL,
            watcher text COLLATE "C" NOT NULL,
            PRIMARY KEY (target, watcher)
        )"#,
        "CREATE INDEX watches_watcher ON {s}.watches (watcher, target)",
        "ALTER TABLE {s}.messages ADD COLUMN notification boolean NOT NULL DEFAULT false",
    ],
];

/// The schema version this code writes.
pub const VERSION: i32 = MIGRATIONS.len() as i32;

/// Creates `schema` if needed and applies every missing migration, in one transaction on `c`. Fails with
/// [`StoreError::Invalid`] when the database was migrated by a newer version.
pub(crate) async fn migrate(c: &mut PgConnection, schema: &str) -> Result<(), StoreError> {
    if current_version(c, schema).await? == Some(VERSION) {
        return Ok(());
    }
    migrate_up_to(c, schema, VERSION).await
}

/// Applies the migrations up to `target`; [`migrate`] with every migration.
async fn migrate_up_to(c: &mut PgConnection, schema: &str, target: i32) -> Result<(), StoreError> {
    let s = quote(schema);
    let mut tx = c.begin().await.map_err(map_error)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key(schema))
        .execute(&mut *tx)
        .await
        .map_err(map_error)?;
    ddl(&mut tx, &format!("CREATE SCHEMA IF NOT EXISTS {s}")).await?;
    ddl(
        &mut tx,
        &format!(
            "CREATE TABLE IF NOT EXISTS {s}.cicada_schema_version \
             (version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT clock_timestamp())"
        ),
    )
    .await?;
    let current: i32 =
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT coalesce(max(version), 0) FROM {s}.cicada_schema_version")))
            .fetch_one(&mut *tx)
            .await
            .map_err(map_error)?;
    if current > VERSION {
        return Err(StoreError::Invalid(format!(
            "Database schema version {current} is newer than supported ({VERSION})"
        )));
    }
    for version in current + 1..=target {
        for statement in MIGRATIONS[version as usize - 1] {
            sqlx::raw_sql(AssertSqlSafe(statement.replace("{s}", &s)))
                .execute(&mut *tx)
                .await
                .map_err(|e| migration_error(version, e))?;
        }
        sqlx::query(AssertSqlSafe(format!("INSERT INTO {s}.cicada_schema_version (version) VALUES ($1)")))
            .bind(version)
            .execute(&mut *tx)
            .await
            .map_err(map_error)?;
    }
    tx.commit().await.map_err(map_error)
}

/// Sets up `schema` as a store of schema version `version` would have: the migrations up to it. For tests of an
/// upgrade.
#[cfg(test)]
pub(crate) async fn migrate_to(c: &mut PgConnection, schema: &str, version: i32) -> Result<(), StoreError> {
    migrate_up_to(c, schema, version).await
}

/// The error of a statement of migration `version`: a violated constraint (SQLSTATE class `23`) is
/// [`StoreError::Invalid`], as the data stops the migration and trying again cannot help; anything else is
/// [`map_error`].
fn migration_error(version: i32, e: sqlx::Error) -> StoreError {
    match &e {
        sqlx::Error::Database(db) if sql_state(&e).is_some_and(|state| state.starts_with("23")) => StoreError::Invalid(
            format!("The database cannot be migrated to schema version {version}: {}", db.message()),
        ),
        _ => map_error(e),
    }
}

async fn ddl(c: &mut PgConnection, statement: &str) -> Result<(), StoreError> {
    sqlx::raw_sql(AssertSqlSafe(statement.to_owned())).execute(c).await.map_err(map_error)?;
    Ok(())
}

/// The recorded schema version, or `None` when `schema` has no version table yet. Reads only.
async fn current_version(c: &mut PgConnection, schema: &str) -> Result<Option<i32>, StoreError> {
    let table = format!("{}.cicada_schema_version", quote(schema));
    let mut tx = c.begin().await.map_err(map_error)?;
    let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(&table)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_error)?;
    let version = if exists {
        Some(
            sqlx::query_scalar(AssertSqlSafe(format!("SELECT coalesce(max(version), 0) FROM {table}")))
                .fetch_one(&mut *tx)
                .await
                .map_err(map_error)?,
        )
    } else {
        None
    };
    tx.commit().await.map_err(map_error)?;
    Ok(version)
}

/// Quotes a validated schema identifier.
pub(crate) fn quote(schema: &str) -> String {
    format!("\"{schema}\"")
}

/// Advisory lock key for migrations of `schema`: a fixed namespace in the high bits, a hash of the name below. It must
/// not change, so that nodes of different versions starting together serialise on the same lock.
fn lock_key(schema: &str) -> i64 {
    (0x4349_4341_i64 << 32) | i64::from(name_hash(schema))
}

/// The 32-bit FNV-1a hash of the name's bytes.
fn name_hash(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |h, byte| (h ^ u32::from(byte)).wrapping_mul(0x0100_0193))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_key_is_stable() {
        // Fixed values: other ones would split the lock between node versions.
        assert_eq!(name_hash(""), 0x811c_9dc5);
        assert_eq!(name_hash("public"), 0xcc90_9380);
        assert_eq!(lock_key(""), 0x4349_4341_811c_9dc5);
        assert_eq!(lock_key("public"), 0x4349_4341_cc90_9380);
    }

    #[test]
    fn the_schema_is_at_version_5() {
        assert_eq!(VERSION, 5);
    }
}
