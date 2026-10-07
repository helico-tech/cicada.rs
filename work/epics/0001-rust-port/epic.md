---
status: in-progress
created: 2026-10-07
---

# Epic 0001: Rust port of Cicada

## Goal

An idiomatic Rust implementation of Cicada with the same contract as the Kotlin implementation: guarantees, store schema, HTTP API and console.

## Scope

Every module of the Kotlin repo: core, test harness, conformance suite, SQLite and PostgreSQL stores, HTTP API, client, console, OpenTelemetry, extras, examples, sample service, end-to-end tests and docs.

## Done when

All stories are done, `cargo test --workspace` passes (PostgreSQL tests included), and the repo is pushed.

## Stories

- [01 Core model](01-core-model.md) — done
- [02 Runtime](02-runtime.md) — done
- [03 Test harness](03-test-harness.md) — done
- [04 Store conformance suite S1–S10 as a reusable crate, run against InMemoryStore](04-conformance-suite.md) — todo
- [05 SQLite store on sqlx with the Kotlin schema](05-sqlite-store.md) — done
- [06 PostgreSQL store on sqlx with LISTEN/NOTIFY wake-ups](06-postgres-store.md) — todo
- [07 Port the runtime test suite of `actors-test` (dispatch, leases, retry, hooks, outcomes, timeouts, shutdown)](07-runtime-tests.md) — todo
- [08 HTTP API as axum routers with the Kotlin wire model and problem+json](08-http-api.md) — todo
- [09 Typed HTTP client on reqwest](09-client.md) — todo
- [10 Embed the React console and serve it with `actor_ui()`](10-console.md) — todo
- [11 OpenTelemetry plugin](11-otel.md) — todo
- [12 Extras](12-extras.md) — todo
- [13 Worked examples with tests](13-examples.md) — done
- [14 Runnable sample node, README snippets test, contract snapshots and state samples](14-sample-service.md) — todo
- [15 Multi-node end-to-end tests against PostgreSQL](15-e2e.md) — todo
- [16 README and operations guide for the Rust port](16-docs.md) — todo
