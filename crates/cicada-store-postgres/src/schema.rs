//! The PostgreSQL schema and its versioned migrations, identical to the Kotlin implementation's (ADR 0004).
//!
//! The four tables of the architecture (`actor_types`, `actors`, `messages`, `outcomes`) plus two sequences for the
//! store-wide `seq` and epoch counters. JSON values are `jsonb`, instants `timestamptz` (microsecond precision, the
//! store's precision). Keys and URIs use the `"C"` collation, so ordering and keyset paging are by code point, as in
//! every other backend, and prefix ranges can use the B-tree indexes.
//!
//! Migrations are applied by [`migrate`] on store start: idempotent, serialised between concurrently starting nodes
//! (Kotlin or Rust) by a transaction-scoped advisory lock, and recorded in `cicada_schema_version`. A database written
//! by a newer version is refused. When the schema is already current, start runs no DDL at all, so a node may connect
//! as a user without `CREATE` privileges once the schema was migrated by one that has them.
//!
//! Migrations are additive so that nodes of the previous version keep working during a rolling deploy: version 3
//! keeps `outcomes.retried_as` (written and read only by version 2) as an unused nullable column.

use cicada::store::StoreError;
use sqlx::{AssertSqlSafe, Connection, PgConnection};

use crate::error::map_error;

/// Ordered migrations; index + 1 is the version. `{s}` is replaced by the quoted schema name. Never edit one.
const MIGRATIONS: &[&[&str]] = &[
    // 1: initial schema
    &[
        r#"CREATE TABLE {s}.actor_types (
            template      text COLLATE "C" PRIMARY KEY,
            shape         text COLLATE "C" NOT NULL UNIQUE,
            contract      jsonb NOT NULL,
            contract_hash text NOT NULL
        )"#,
        r#"CREATE TABLE {s}.actors (
            uri           text COLLATE "C" PRIMARY KEY,
            type          text COLLATE "C" NOT NULL,
            state         jsonb,
            version       bigint NOT NULL DEFAULT 0,
            epoch         bigint NOT NULL DEFAULT 0,
            lease_owner   text,
            lease_until   timestamptz,
            lease_message text COLLATE "C",
            next_due_at   timestamptz,
            created_at    timestamptz NOT NULL
        )"#,
        // The claim scan: instances with a pending message, in due order.
        "CREATE INDEX actors_due ON {s}.actors (next_due_at, type) WHERE next_due_at IS NOT NULL",
        r#"CREATE TABLE {s}.messages (
            id             text COLLATE "C" PRIMARY KEY,
            actor_uri      text COLLATE "C" NOT NULL,
            seq            bigint NOT NULL,
            type           text NOT NULL,
            payload        jsonb NOT NULL,
            sender         text,
            correlation_id text,
            headers        jsonb NOT NULL,
            deliver_at     timestamptz NOT NULL,
            attempt        integer NOT NULL DEFAULT 0
        )"#,
        "CREATE INDEX messages_mailbox ON {s}.messages (actor_uri, deliver_at, seq)",
        r#"CREATE TABLE {s}.outcomes (
            message_id  text COLLATE "C" PRIMARY KEY,
            actor_uri   text COLLATE "C" NOT NULL,
            status      text NOT NULL,
            error       text,
            finished_at timestamptz NOT NULL,
            envelope    jsonb NOT NULL
        )"#,
        // Retention sweep.
        "CREATE INDEX outcomes_finished ON {s}.outcomes (finished_at)",
        // Failures: by time across all instances, and by URI prefix.
        "CREATE INDEX outcomes_failures ON {s}.outcomes (finished_at, message_id) WHERE status = 'failed'",
        "CREATE INDEX outcomes_failures_by_actor ON {s}.outcomes (actor_uri, finished_at) WHERE status = 'failed'",
        "CREATE SEQUENCE {s}.message_seq",
        "CREATE SEQUENCE {s}.epoch_seq",
    ],
    // 2 (T18): the retry a `done` outcome was replaced by.
    &[r#"ALTER TABLE {s}.outcomes ADD COLUMN retried_as text COLLATE "C""#],
    // 3 (Retry v2): a failed message is rescheduled in place, so the message records its failures. Additive only:
    // `outcomes.retried_as` stays, unused, because nodes still on version 2 write and read it during a rolling deploy.
    &[
        "ALTER TABLE {s}.messages ADD COLUMN failures integer NOT NULL DEFAULT 0",
        "ALTER TABLE {s}.messages ADD COLUMN last_error text",
    ],
    // 4 (Retry v3): the sender's retry policy travels with the message (null for no retry).
    &["ALTER TABLE {s}.messages ADD COLUMN retry_policy jsonb"],
    // 5 (H1): the message whose turn sent a message, for message chains; indexed to find what a message caused.
    &[
        r#"ALTER TABLE {s}.messages ADD COLUMN caused_by text COLLATE "C""#,
        r#"ALTER TABLE {s}.outcomes ADD COLUMN caused_by text COLLATE "C""#,
        "CREATE INDEX messages_caused_by ON {s}.messages (caused_by) WHERE caused_by IS NOT NULL",
        "CREATE INDEX outcomes_caused_by ON {s}.outcomes (caused_by) WHERE caused_by IS NOT NULL",
    ],
    // 6 (H2): the sender's retention policy travels with the message; an outcome is visible until `visible_until`
    // (null: for the store retention), `keep_last` outcomes are trimmed per instance and status in `keep_seq` order
    // (from `message_seq`), and `stripped` marks a hidden record whose content the sweep dropped.
    &[
        "ALTER TABLE {s}.messages ADD COLUMN retention_policy jsonb",
        "ALTER TABLE {s}.outcomes ADD COLUMN visible_until timestamptz",
        "ALTER TABLE {s}.outcomes ADD COLUMN keep_last boolean NOT NULL DEFAULT false",
        "ALTER TABLE {s}.outcomes ADD COLUMN keep_seq bigint",
        "ALTER TABLE {s}.outcomes ADD COLUMN stripped boolean NOT NULL DEFAULT false",
        "CREATE INDEX outcomes_keep_last ON {s}.outcomes (actor_uri, status, keep_seq) WHERE keep_last",
        "CREATE INDEX outcomes_visible_until ON {s}.outcomes (visible_until) WHERE visible_until IS NOT NULL",
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
    for version in current + 1..=VERSION {
        for statement in MIGRATIONS[version as usize - 1] {
            ddl(&mut tx, &statement.replace("{s}", &s)).await?;
        }
        sqlx::query(AssertSqlSafe(format!("INSERT INTO {s}.cicada_schema_version (version) VALUES ($1)")))
            .bind(version)
            .execute(&mut *tx)
            .await
            .map_err(map_error)?;
    }
    tx.commit().await.map_err(map_error)
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

/// Advisory lock key for migrations of `schema`: a fixed namespace in the high bits, the name's Java `hashCode` below,
/// so Kotlin and Rust nodes starting together serialise on the same lock.
fn lock_key(schema: &str) -> i64 {
    (0x4349_4341_i64 << 32) | (java_hash(schema) as i64 & 0xFFFF_FFFF)
}

/// Java's `String.hashCode`: `s[0]*31^(n-1) + … + s[n-1]` over UTF-16 code units, wrapping.
fn java_hash(text: &str) -> i32 {
    text.encode_utf16().fold(0i32, |h, unit| h.wrapping_mul(31).wrapping_add(unit as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_key_matches_kotlins() {
        // Values of Java's "public".hashCode() and "".hashCode().
        assert_eq!(java_hash("public"), -977_423_767);
        assert_eq!(java_hash(""), 0);
        assert_eq!(lock_key(""), 0x4349_4341_0000_0000);
        assert_eq!(lock_key("public"), 0x4349_4341_0000_0000 | (-977_423_767i64 & 0xFFFF_FFFF));
    }

    #[test]
    fn there_are_six_versions() {
        assert_eq!(VERSION, 6);
    }
}
