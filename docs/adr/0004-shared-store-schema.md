# 0004. The SQL schema is shared with the Kotlin implementation

Date: 2026-10-07

## Status

Superseded by 0008

## Context

Nodes coordinate only through the store, so the store schema is the protocol between nodes. Kotlin keeps a versioned,
additive schema (SQLite `PRAGMA user_version`, PostgreSQL `cicada_schema_version`), currently at version 6.

## Decision

The Rust stores create and migrate exactly the same tables, columns, indexes, sequences and version records, with the
same encodings (SQLite: integer microseconds and JSON text; PostgreSQL: `timestamptz` and `jsonb`; envelopes as the
same JSON). A database written by either implementation can be read and served by the other.

## Consequences

- Kotlin and Rust nodes can share one store and serve each other's instances over HTTP.
- Contract hashes differ between the implementations, because the generated JSON Schemas differ in detail. A template
  registered by a Kotlin node conflicts with the same template from a Rust node, so one actor type is hosted by one
  implementation at a time. Types of different templates mix freely.
- A schema change needs a migration in both implementations, with the same version number.
