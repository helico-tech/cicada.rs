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
PROPTEST_CASES=100000 cargo test -p cicada-fuzz   # a long randomised run; a short one is part of cargo test --workspace
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
| `crates/cicada-cli` | The `cicada` command line and terminal UI, over HTTP |
| `crates/cicada-otel`, `crates/cicada-extras` | Plugins and building blocks |
| `crates/cicada-fuzz` | Randomised tests: parsers, backend comparison, node simulation under faults |
| `examples/`, `sample-service/` | Worked examples; a runnable node |
| `e2e/` | End-to-end and fault tests against sample nodes in Docker |
| `docs/` | Architecture, ADRs and the operations guide |
| `openspec/` | Specs, changes in progress and finished changes |
| `.claude/skills/change-loop/` | The change loop: the skill, the briefs of its agents and the status script |

## Rules

- [docs/architecture.md](docs/architecture.md) is the reference for behaviour. This repo has no contract with the
  Kotlin implementation (ADR 0008). A change to a guarantee or a wire format needs an ADR; a schema change is an
  appended migration.
- Tests for runtime behaviour run on virtual time: `#[tokio::test(start_paused = true)]` with `TestActors`.
- Store backends must pass `cicada-store-conformance`.
- Releases are tags on `main`; [docs/operations.md](docs/operations.md), Releasing, has the steps. Tag a release when
  its change is implemented, every check in Releasing has passed, and the work is committed on `main`; never move or
  delete a tag that has a GitHub Release.
- Public items have doc comments (`missing_docs` warns). Keep comments as dense as the surrounding code.
- The Rust blocks of `README.md` and `docs/operations.md` are doctests of `cicada-examples` (`ReadmeDoctests`,
  `OperationsDoctests` in `examples/src/lib.rs`): a change to a public API must keep them compiling. Use `no_run` for
  blocks that start a server or need a database.

## Specs and changes: OpenSpec

Specs, changes, designs and tasks live in `openspec/` ([OpenSpec](https://github.com/Fission-AI/OpenSpec), ADR 0010).
Work on them goes through the change loop (ADR 0020).

- `openspec/specs/`: what the system does, one folder per capability. It grows as changes are archived;
  `docs/architecture.md` stays the reference for behaviour.
- `openspec/changes/<name>/`: a change: `proposal.md`, `design.md`, `tasks.md` and spec deltas. A change with only a
  `proposal.md` is not started. `tasks.md` is written last: a change that has one is agreed and ready to build.
- Every proposal has an `Order` section that names the change before it, so the open changes form one line.
  `python3 .claude/skills/change-loop/status.py` prints the line and the state of each change.
- `openspec/changes/archive/`: finished changes, and the old epics and issues (`2026-11-09-work-tracker-history/`).
- Flow: the owner starts the loop with `/change-loop`. The main session describes the next change together with the
  owner, one change ahead, and has it reviewed before its tasks are written. A subagent builds the first ready change
  in a worktree under `.claude/worktrees/`, a second one validates it, and the owner decides whether it lands. Every
  landed change is a release. A change is built through this loop unless the owner says otherwise. The rules for the
  artifacts are in `openspec/config.yaml`.
- Out-of-scope findings become a change with only a `proposal.md` and an `Order` section.
- Use the CLI (`openspec list`, `openspec show`, `openspec validate`) to read and check them.
