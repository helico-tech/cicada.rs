---
status: todo
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

Port `PostgresStore`, `Schema`, `NotificationListener`, `PostgresConnection` URL parsing.

## Progress log

- (none yet)

## Follow-ups

- (none yet)
