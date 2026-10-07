---
status: done
created: 2026-10-07
---

# 16. README and operations guide for the Rust port

## Goal

README and operations guide for the Rust port.

## Acceptance criteria

- README covers every module and the HTTP API; OPERATIONS adapted.
- Every Rust code block of the README compiles (and runs, unless it starts a server or needs a database) as a doctest.

## Plan

- `README.md`: adapt the Kotlin README section by section (what Cicada is, crates, quick start, HTTP API, console,
  testing, contract snapshots and state samples, retry, retention, message chains, plugins and OpenTelemetry, extras,
  configuration, examples, sample service, interoperability with Kotlin nodes, decisions and deviations). Names and
  signatures taken from the Rust source of every crate; the HTTP tables checked against `cicada-axum`.
- README snippets as doctests: `#[doc = include_str!("../../README.md")] #[cfg(doctest)]` on a struct in
  `examples/src/lib.rs`, with the crates the snippets use as dev-dependencies of `cicada-examples`. `no_run` for
  servers and databases; hidden `#` lines for setup. Replaces Kotlin's `ReadmeSnippetsTest`.
- `docs/operations.md`: adapt `OPERATIONS.md` (sizing, deploying, probes, contract changes, upgrades, on call,
  telemetry) to `NodeConfig`, `PostgresStoreConfig`, `SqliteOptions`, tokio (`spawn_blocking`) and `tracing`. Its Rust
  blocks run as doctests too.
- Link the guide from README and `AGENTS.md`; fix `docs/architecture.md` only where it is wrong.

## Progress log

- 2026-10-07: started; read the Kotlin README and OPERATIONS.md, the Rust crates' public APIs, ADRs, issues and the
  progress logs of stories 01–13.
- 2026-10-07: `README.md` written: the Kotlin structure, plus sections on plugins, examples, interoperability with
  Kotlin nodes and a "Decisions and deviations" section (ADRs 0001–0005, the inherited open questions, and the
  deviations from issues 0200, 0300, 0501, 0600, 0701 and the logs of stories 05, 07, 12, 13). HTTP tables checked
  against `cicada-axum` (handlers, request parsing, problem types).
- 2026-10-07: `docs/operations.md` written: sizing (tokio workers, `spawn_blocking`, pool vs `max_connections`
  including the listener's two connections), deploying (signal handling is the host's job; stop, then close the
  store), probes, contract changes (also across implementations), upgrading (schema 6, mixed fleets), on call, a
  table of the `tracing` events worth an alert, store tuning tables, telemetry.
- 2026-10-07: doctests: `ReadmeDoctests` and `OperationsDoctests` in `examples/src/lib.rs`; dev-dependencies added
  to `examples/Cargo.toml` (anyhow, async-trait, axum, cicada-axum, cicada-client, cicada-store-postgres,
  cicada-store-sqlite, cicada-ui, tracing-subscriber). 15 README blocks: 11 compile and run, 4 `no_run` (server,
  client, contract snapshots and state samples, which write files); 1 `no_run` block in the operations guide. They
  caught one error while writing (a missing `Store` import). `cargo test -p cicada-examples` (all tests and 17
  doctests) passes; `cargo clippy -p cicada-examples --all-targets` is clean.
- 2026-10-07: `AGENTS.md` links the guide and says the README blocks are doctests. `docs/architecture.md` needed no
  factual correction.

## Follow-ups

- The sample service (story 14) was written in parallel: check the README's sample-service section (environment
  variables taken from the Kotlin `SampleConfig`, OTLP export) against its code once it is merged.
- Hidden `#` setup lines of the README blocks show on GitHub (rustdoc convention); keep them few.
- Story 15 may add an E2E / OpenTelemetry compose setup; the README's "Building and testing" and the guide's
  telemetry section then need its commands.
