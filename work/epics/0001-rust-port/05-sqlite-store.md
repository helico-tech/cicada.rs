---
status: todo
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

Port `SqliteStore` with one writer connection behind a mutex, `BEGIN IMMEDIATE`, a reader pool, WAL and `synchronous=FULL`.

## Progress log

- (none yet)

## Follow-ups

- (none yet)
