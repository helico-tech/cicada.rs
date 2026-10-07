# Cicada for Rust — agent guide

Durable actors on a shared SQL store: a Rust port of [Cicada for Kotlin](https://github.com/helico-tech/cicada)
that keeps its contract (guarantees, store schema, HTTP API). Read [docs/architecture.md](docs/architecture.md) first,
then the ADRs in [docs/adr/](docs/adr/).

## Build and test

```sh
cargo build --workspace
cargo test --workspace                 # unit, runtime, SQLite and HTTP tests
cargo clippy --workspace --all-targets
CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada-store-postgres   # needs PostgreSQL
```

PostgreSQL for tests: `docker compose up -d postgres` (port 55432). Tests that need it are skipped when
`CICADA_PG_URL` is unset. `cicada-ui` needs Node.js 22+ and npm to build the console.

## Map

| Path | What |
| --- | --- |
| `crates/cicada` | Core: `address`, `message`, `actor` (the `Actor` trait), `turn`, `plugin`, `runtime` (node, executor), `store` (trait, model, support), `contract`, `schema`, `retry`, `retention`, `time`, `api` (HTTP wire model) |
| `crates/cicada-test` | `InMemoryStore`, `TokioClock`, `TestActors` harness |
| `crates/cicada-store-*` | Store backends and the conformance suite |
| `crates/cicada-axum` | HTTP API routers |
| `crates/cicada-client` | Typed HTTP client |
| `crates/cicada-ui` | The embedded console |
| `crates/cicada-otel`, `crates/cicada-extras` | Plugins and building blocks |
| `examples/`, `sample-service/` | Worked examples; a runnable node |
| `docs/` | Architecture and ADRs |
| `work/` | Queue, epics and stories, issues |

## Rules

- The Kotlin repo (`../cicada`) is the reference for behaviour. When porting, read the Kotlin source and tests of the
  same module and keep semantics, wire formats and error texts unless an ADR says otherwise.
- Tests for runtime behaviour run on virtual time: `#[tokio::test(start_paused = true)]` with `TestActors`.
- Store backends must pass `cicada-store-conformance`.
- Public items have doc comments (`missing_docs` warns). Keep comments as dense as the surrounding code.
- Work tracking: take the top item of [work/queue.md](work/queue.md), set it `in-progress`, write the plan in the story,
  log progress there, set `done` when tests pass. Out-of-scope findings become issues in `work/issues/`.
