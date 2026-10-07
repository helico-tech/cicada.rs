---
status: done
created: 2026-10-07
---

# 14. Runnable sample node, README snippets test, contract snapshots and state samples

## Goal

Runnable sample node, README snippets test, contract snapshots and state samples.

## Acceptance criteria

- Binary serves API, console and health; configuration by environment as Kotlin `SampleConfig`.
- Contract snapshot and state sample helpers in `cicada-test`.

## Plan

Port the Kotlin `sample-service` to the crate `cicada-sample-service`: a library (usable in-process by tests) and a thin
`main.rs`.

- `config`: `SampleConfig::from_env` / `from_vars` with the same variables and defaults (`PORT`, `NODE_ID`,
  `HOSTED_TYPES`, `DATABASE_*`, `SQLITE_PATH`, `API_TOKEN`, node timings via `cicada::time::parse_duration`,
  `CONCURRENCY`, `FAILURE_WATCHER_INTERVAL`, `OTEL_EXPORTER_OTLP_ENDPOINT`, `CRASH_LOOP_LIMIT`), the same pool-size rule,
  and a `Debug` without secrets.
- `catalog`: the eleven groups and the singletons; types are type-erased (`CatalogType`: template, contract, host).
- `fakes`, `e2e`: the same fakes and E2E actors (slow, counter, recorder, flaky) with identical templates, message
  names and JSON shapes.
- `app`: `SampleNode::start` (store, node, singletons, failure watcher, telemetry), `router()` (`GET /`, `/console/`,
  `GET /ready`, the API and `GET /e2e/effects` behind bearer auth), `shutdown()`; `serve()` for graceful shutdown.
- `telemetry`: OTLP/HTTP export of traces and metrics with `opentelemetry-otlp`, `ActorTelemetry` on the node.
- `Dockerfile` and a root `.dockerignore`.
- Tests: SampleServiceTest, ContractSnapshotTest, StateSamplesTest. ReadmeSnippetsTest is story 16 (the README).

## Progress log

- 2026-10-07: ported config, catalog, fakes, E2E actors, app and main. 2 unit tests and 14 integration tests pass
  (`sample_service` 10, `contract_snapshots` 1, `state_samples` 3); the PostgreSQL test runs with `CICADA_PG_URL` and is
  skipped without it. Clippy is clean. Contract snapshots recorded in `tests/contracts`, state samples in
  `tests/states`.
- 2026-10-07: smoke-tested the binary: routes, `401` without the token, SIGTERM stops HTTP, node and store; with
  `OTEL_EXPORTER_OTLP_ENDPOINT` set, spans and metrics arrive at `/v1/traces` and `/v1/metrics` and are flushed on exit.
- 2026-10-07: Dockerfile (Rust 1.94 + Node 22 builder, Debian slim runtime, non-root user `cicada`, health check on
  `/ready`); `docker build -f sample-service/Dockerfile -t cicada-rs-sample:dev .` builds.
- Deviations from Kotlin, on purpose:
  - OTLP is HTTP/protobuf only (port 4318, as the Kotlin E2E telemetry overlay uses); `OTEL_EXPORTER_OTLP_PROTOCOL`
    other than `http/protobuf` is refused at start. The Java SDK autoconfiguration also supports gRPC. The other
    `OTEL_*` variables are those the Rust SDK reads (endpoint, headers, timeout, service name, resource attributes,
    `OTEL_BSP_*`, `OTEL_METRIC_EXPORT_INTERVAL`); the W3C trace-context and baggage propagators are installed globally,
    as the autoconfiguration does.
  - Durations accept the short form (`5s`, `250ms`) and ISO-8601 (`PT5S`) through `cicada::time::parse_duration`;
    a negative duration is refused (`LEASE_TTL: not a duration: -1s`).
  - `HOSTED_TYPES` with duplicates keeps the first occurrence; groups keep the given order (as Kotlin's linked set).
  - `SampleNode::router()` answers `401` with `WWW-Authenticate: Bearer realm="api"`, as Ktor's bearer auth.
  - E2E integer fields: `attempt` and `lastAttempt` are `u32` (as `Message::attempt`), Kotlin `Long` fields `i64`,
    `Int` fields `i32`; schemas differ in detail from Kotlin anyway (ADR 0004).
  - The binary is `cicada-sample-service` (`/usr/local/bin` in the image); Kotlin's is `sample-service`.
  - The state samples include copies of the Kotlin samples (`kotlin-*.json`): state written by Kotlin nodes decodes
    in Rust.
- Skipped: `readme/ReadmeSnippetsTest.kt`; the README is story 16, which should port the snippets test with it.

## Follow-ups

- Story 16: port ReadmeSnippetsTest together with the README.
- Story 15 drives the E2E actors over HTTP; their names and JSON shapes match the Kotlin ones.
