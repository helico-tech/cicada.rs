---
status: done
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
- `listener.rs`: one `PgListener` on its own small pool; `try_recv` loop, reconnect wake, back-off
  doubling up to 30 s; a self-sent probe notification detects a hanging connection (Kotlin probes with `SELECT 1`).
- `store.rs`: `PostgresStore` with every `Store` method ported statement by statement: READ COMMITTED transactions,
  `FOR UPDATE` row locks, commit locks touched instances in URI order and creates before reading the clock,
  `SKIP LOCKED` claims, one `clock_timestamp()` per transaction, keyset paging, dedup after insert, `pg_notify` with
  the transaction, re-runs on 40001/40P01, SQLSTATE classes 22/54 → `Invalid`, decode failures → `Corrupt`.
- Tests against the docker-compose PostgreSQL (`CICADA_PG_URL`, skipped when unset), one random schema per test:
  ports of `PostgresStoreTest`, `PostgresRobustnessTest`, `PostgresRuntimeIntegrationTest`, `PostgresConnectionTest`.
  Then the conformance suite (story 04, merged from main) with a fresh schema per case.

## Progress log

- 2026-10-07: Started. Read the Kotlin module and its tests, the Rust store trait and `InMemoryStore`.
- 2026-10-07: Store, schema, listener, config and URL parsing written. Transactions run through a `transaction!`
  macro instead of a function taking an async closure: the futures of closures that borrow their `&mut Tx` argument
  cannot be proven `Send` for every lifetime, which `async_trait` requires.
- 2026-10-07: The listener opens a fresh two-connection pool per (re)connect (LISTEN + probe), so a hung old
  connection whose clean-up never finishes cannot block the reconnect. Probes: a `pg_notify` to itself every 5 s;
  10 s of silence counts as a hang.
- 2026-10-07: Ports of `PostgresStoreTest` (8), `PostgresRobustnessTest` (6 + a socket-timeout test),
  `PostgresRuntimeIntegrationTest` (3) and `PostgresConnectionTest` (unit tests) pass against PostgreSQL 17.5.
- 2026-10-07: Merged main with the conformance suite; `tests/conformance.rs` runs it with a fresh schema per case,
  peers as separate stores and reopen support. Fixed in the store: `failures(since = DateTime::MIN_UTC)` was
  refused ("timestamp out of range"); instants used as lower bounds are clamped to 0001-01-01. The suite's 102
  cases running in parallel exceeded the compose PostgreSQL's 100 connections; the fixture now runs 4 cases at once
  and gives peers one connection each (issue 0500). All 102 cases pass; 133 tests in the crate.

## Follow-ups

- [0500 The docker-compose PostgreSQL allows only 100 connections](../../issues/0500-compose-postgres-max-connections.md)
- [0501 `PostgresStoreConfig` has no map of extra connection properties](../../issues/0501-postgres-extra-connection-properties.md)
