# Cicada for Rust — agent guide

Durable actors on a shared SQL store, in Rust. Read [docs/architecture.md](docs/architecture.md) first,
then the ADRs in [docs/adr/](docs/adr/). [README.md](README.md) is the user guide; [docs/operations.md](docs/operations.md)
covers sizing, deploying, upgrading and watching nodes.

## Build and test

```sh
cargo build --workspace
cargo test --workspace                 # unit, runtime, SQLite, HTTP and README tests
cargo test -p cicada-examples --doc    # only the Rust blocks of README.md and docs/operations.md
cargo clippy --workspace --all-targets
cargo fmt --all                        # rustfmt.toml: 120 columns; CI checks it
CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada-store-postgres   # needs PostgreSQL
CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture   # E2E and fault tests in Docker, ~3 min; see e2e/README.md
```

PostgreSQL for tests: `docker compose up -d postgres` (port 55432). Tests that need it are skipped when
`CICADA_PG_URL` is unset.

## Map

| Path | What |
| --- | --- |
| `crates/cicada` | Core: `address`, `message`, `actor` (the `Actor` trait), `turn`, `plugin`, `runtime` (node, executor), `store` (trait, model, support), `contract`, `schema`, `retry`, `retention`, `time`, `api` (HTTP wire model) |
| `crates/cicada-test` | `InMemoryStore`, `TokioClock`, `TestActors` harness |
| `crates/cicada-store-*` | Store backends and the conformance suite |
| `crates/cicada-axum` | HTTP API routers |
| `crates/cicada-client` | Typed HTTP client |
| `crates/cicada-otel`, `crates/cicada-extras` | Plugins and building blocks |
| `examples/`, `sample-service/` | Worked examples; a runnable node |
| `e2e/` | End-to-end and fault tests against sample nodes in Docker |
| `docs/` | Architecture, ADRs and the operations guide |
| `openspec/` | Specs, changes in progress and finished changes |

## Rules

- [docs/architecture.md](docs/architecture.md) is the reference for behaviour. This repo has no contract with the
  Kotlin implementation (ADR 0008). A change to a guarantee or a wire format needs an ADR; a schema change is an
  appended migration.
- Tests for runtime behaviour run on virtual time: `#[tokio::test(start_paused = true)]` with `TestActors`.
- Store backends must pass `cicada-store-conformance`.
- Releases are tags on `main`; [docs/operations.md](docs/operations.md), Releasing, has the steps. Do not tag or release
  unless asked.
- Public items have doc comments (`missing_docs` warns). Keep comments as dense as the surrounding code.
- The Rust blocks of `README.md` and `docs/operations.md` are doctests of `cicada-examples` (`ReadmeDoctests`,
  `OperationsDoctests` in `examples/src/lib.rs`): a change to a public API must keep them compiling. Use `no_run` for
  blocks that start a server or need a database.

## Specs and changes: OpenSpec

Specs, changes, designs and tasks live in `openspec/` ([OpenSpec](https://github.com/Fission-AI/OpenSpec), ADR 0010).

- `openspec/specs/`: what the system does, one folder per capability. It grows as changes are archived;
  `docs/architecture.md` stays the reference for behaviour.
- `openspec/changes/<name>/`: a change: `proposal.md`, `design.md`, `tasks.md` and spec deltas. A change with only a
  `proposal.md` is not started.
- `openspec/changes/archive/`: finished changes, and the old epics and issues (`2026-11-09-work-tracker-history/`).
- Flow: `/opsx:explore` to think it through, `/opsx:propose` to write the change, `/opsx:apply` to build it,
  `/opsx:archive` when done. Project rules for these are in `openspec/config.yaml`.
- Out-of-scope findings become a change with only a `proposal.md`.
- Use the CLI (`openspec list`, `openspec show`, `openspec validate`) to read and check them.
