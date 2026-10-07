---
status: in-progress
created: 2026-10-07
---

# 06. PostgreSQL store on sqlx with LISTEN/NOTIFY wake-ups

## Goal

PostgreSQL store on sqlx with LISTEN/NOTIFY wake-ups.

## Acceptance criteria

- Same schema and `cicada_schema_version` as Kotlin.
- Passes the conformance suite against a real PostgreSQL.
- Wake-ups via `PgListener` with reconnect.

## Plan

Port `actors-store-postgres` (PostgresStore, Schema, NotificationListener, PostgresConnection, PostgresStoreConfig)
into `crates/cicada-store-postgres`, one module each:

- `config.rs`: `PostgresStoreConfig` with the Kotlin defaults and validation texts. `socket_timeout` becomes a
  client-side timeout per transaction attempt (sqlx has no per-read timeout); the connection is closed when it fires.
- `connection.rs`: `PostgresConnection::parse` accepts `postgres(ql)://` and `jdbc:postgresql://` URLs and normalises
  them to a libpq-style URL plus credentials (the reverse direction of Kotlin, as sqlx speaks libpq URLs). Port of
  `PostgresConnectionTest`.
- `schema.rs`: migrations 1–6 with the same DDL, `cicada_schema_version`, the same advisory-lock key (Java
  `String.hashCode`), no DDL when current, refusal of newer versions.
- `listener.rs`: one `PgListener` on a dedicated one-connection pool; `try_recv` loop, reconnect wake, back-off
  doubling up to 30 s; a self-sent probe notification detects a hanging connection (Kotlin probes with `SELECT 1`).
- `store.rs`: `PostgresStore` with every `Store` method ported statement by statement: READ COMMITTED transactions,
  `FOR UPDATE` row locks, commit locks touched instances in URI order and creates before reading the clock,
  `SKIP LOCKED` claims, one `clock_timestamp()` per transaction, keyset paging, dedup after insert, `pg_notify` with
  the transaction, re-runs on 40001/40P01, SQLSTATE classes 22/54 → `Invalid`, decode failures → `Corrupt`.
- Tests against the docker-compose PostgreSQL (`CICADA_PG_URL`, skipped when unset), one random schema per test:
  ports of `PostgresStoreTest`, `PostgresRobustnessTest`, `PostgresRuntimeIntegrationTest`, `PostgresConnectionTest`.
  The conformance suite is wired in later (story 04 runs in parallel).

## Progress log

- 2026-10-07: Started. Read the Kotlin module and its tests, the Rust store trait and `InMemoryStore`.

## Follow-ups

- (none yet)
