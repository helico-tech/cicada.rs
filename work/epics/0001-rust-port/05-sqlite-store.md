---
status: in-progress
created: 2026-10-07
---

# 05. SQLite store on sqlx with the Kotlin schema

## Goal

SQLite store on sqlx with the Kotlin schema.

## Acceptance criteria

- Same tables, migrations and `user_version` as Kotlin.
- Passes the conformance suite.
- Kotlin SQLite store tests ported.

## Plan

Port `actors-store-sqlite` (`SqliteStore.kt`, `Schema.kt`, `Jdbc.kt`) to `crates/cicada-store-sqlite`:

- `schema.rs`: migrations 1–6 with the Kotlin DDL text after `trimIndent()`, `PRAGMA user_version`, refusal of a newer
  version, all in one `BEGIN IMMEDIATE` transaction.
- `conn.rs`: a sqlx `SqliteConnection` with explicit `BEGIN IMMEDIATE` / `BEGIN DEFERRED` … `COMMIT` (no sqlx
  `Transaction`); `BEGIN` retried on `SQLITE_BUSY` (3 attempts); rollback on any error. A connection remembers that a
  transaction may be open, so a write whose future was dropped (cancelled) is rolled back before the next use.
- `store.rs`: `SqliteStore::open(path, SqliteOptions { clock, retention, busy_timeout, readers })`; one writer behind a
  tokio mutex, `readers` read-only connections behind a semaphore; WAL, `synchronous = FULL`, `busy_timeout`, foreign
  keys on. Store time = `max(clock, persisted mark in sequences('clock'), latest handed out)`. The same SQL as Kotlin
  per operation.
- Errors: sqlx errors → `Unavailable`; bad arguments / refused data → `Invalid`; undecodable rows → `Corrupt`; closed
  store → `Unavailable("Store is closed")`.
- Tests: port `SqliteStoreTest`, `SqliteContentionTest`, `SqliteRuntimeIntegrationTest`; a schema test that compares
  `sqlite_master` with the DDL extracted from the Kotlin `Schema.kt` (skipped when the Kotlin repo is absent) and reads
  the Rust-created file with the `sqlite3` CLI when installed. The conformance suite is wired in by a later step.

## Progress log

- 2026-10-07: `SqliteStore` built as planned (`schema.rs`, `conn.rs`, `rows.rs`, `store.rs`). The SQL per operation is
  Kotlin's; the only change is `UPDATE sequences … RETURNING value` instead of `UPDATE` + `SELECT` for the next sequence
  value (same effect inside the transaction). A database written by a newer schema version is refused with
  `StoreError::Invalid("Database schema version N is newer than supported (6)")` (Kotlin: `IllegalStateException`).
- Tests (20, plus a doctest): `sqlite_store.rs` (port of `SqliteStoreTest`, migrations from versions 1–5, plus
  high-water mark across a reopen, encodings, zero readers), `contention.rs` (port of `SqliteContentionTest`, plus a
  write cancelled while it waits for the lock), `runtime.rs` (port of `SqliteRuntimeIntegrationTest`: two nodes on one
  file, real time), `differential.rs` (one scenario over every `Store` method against `InMemoryStore` and
  `SqliteStore`; all answers equal), `kotlin_schema.rs` (below).
- Compatibility with Kotlin (ADR 0004): `kotlin_schema.rs` checks that the migration statements equal those of the
  Kotlin `Schema.kt` text for text (the Kotlin repo is found next to an ancestor directory, or via
  `CICADA_KOTLIN_DIR`), and that the `sqlite3` CLI fed the Kotlin statements builds the same `sqlite_master`,
  `user_version` and sequences as `SqliteStore`. Both are skipped when the repo or the CLI is missing. Manual checks
  done once: `CICADA_KOTLIN_DB=<copy of the Kotlin sample-service actors.db, schema 4>` opens, migrates to 6 and reads
  with the Rust store; `CICADA_WRITE_DB=<file>` writes a database with every kind of row, which the Kotlin
  `SqliteStore` (a throw-away test in a scratch copy of the Kotlin repo, `gradle --offline`) read, claimed from and
  committed to, after which the Rust store read the result again.
- `cargo test -p cicada-store-sqlite` and `cargo clippy -p cicada-store-sqlite --all-targets` are clean. Not done yet:
  the conformance suite (story 04, written in parallel) is not wired in; this story stays `in-progress` until it is.

## Follow-ups

- Wire `cicada-store-conformance` in (`tests/conformance.rs`, a fixture with a step-back clock, peers as extra
  `SqliteStore` instances on the same file, reopen), and port `SqliteStoreRealTimeConformanceTest`.
- [0400 No rustfmt configuration](../../issues/0400-no-rustfmt-config.md).
