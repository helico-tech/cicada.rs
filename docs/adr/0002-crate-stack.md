# 0002. Crate stack: tokio, axum, sqlx, serde, schemars, chrono

Date: 2026-10-07

## Status

Accepted

## Context

The Kotlin implementation uses Ktor (HTTP, lifecycle, auth), JDBC with HikariCP, kotlinx.serialization and
kotlinx-schema. The Rust port needs an equivalent for each, chosen for maturity and for fitting "mount the API into the
host's router; the host keeps its own auth".

## Decision

| Need | Choice | Why not the alternatives |
| --- | --- | --- |
| Async runtime | tokio | The ecosystem default; paused time gives virtual-time tests. |
| HTTP server | axum 0.8 + tower | A `Router` nests into any axum app; auth stays tower layers of the host. actix-web and poem have their own app model. |
| SQL | sqlx 0.9 with one concrete store per backend | `sqlx::Any` has no JSON type; code generic over `sqlx::Database` needs unworkable bounds. A `Store` trait with a SQLite and a PostgreSQL implementation mirrors the Kotlin design. |
| JSON | serde + serde_json (`preserve_order`) | The serializer is the only validator, as in Kotlin. |
| JSON Schema | schemars 1.x (draft 2020-12) | Derives schemas from the same serde attributes. |
| Time | chrono | jiff cannot represent the contract's latest instant (`9999-12-31T23:59:59.999999Z`); chrono can, and sqlx maps it to `timestamptz`. Durations are parsed by our own code (ISO-8601 and the Kotlin short form). |
| Errors | thiserror for library errors, anyhow inside `HandlerError` | `?` works on any error in handlers. |
| Plugins and the store trait | async-trait | Both are used as trait objects. |
| HTTP client | reqwest | De-facto standard. |
| Console embedding | rust-embed | Files compiled into the binary, served by an axum handler. |
| Observability | tracing; opentelemetry for the plugin | Ecosystem defaults. |

## Consequences

- Rust services that host actors must run on tokio.
- `rust-version` follows sqlx (1.94).
