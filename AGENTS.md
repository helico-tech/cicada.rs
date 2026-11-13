# Cicada for Rust — agent guide

Durable actors on a shared SQL store, in Rust. Read [docs/architecture.md](docs/architecture.md) first, then the ADRs
([docs/adr/README.md](docs/adr/README.md) is their index). The user guide is one guide per layer in
[docs/guide/](docs/guide/): [core.md](docs/guide/core.md), [http.md](docs/guide/http.md),
[app.md](docs/guide/app.md) and [actors.md](docs/guide/actors.md); [README.md](README.md) is an overview that
points into them. [docs/operations.md](docs/operations.md) covers sizing, deploying, upgrading and watching nodes.

## Build and test

```sh
cargo build --workspace
cargo test --workspace                 # unit, runtime, SQLite, HTTP, doctest and docs tests; every feature of cicada and cicada-app on
cargo test -p cicada-examples --doc    # only the Rust blocks of README.md, the guides and docs/operations.md
cargo test -p cicada-examples --test docs   # the table of guarantees and the links between the Markdown files
cargo clippy --workspace --all-targets
cargo fmt --all                        # rustfmt.toml: 120 columns; CI checks it
cargo check -p cicada [--features sqlite|postgres|testing]   # the core with no feature or one alone; CI builds each
cargo check -p cicada-app [--features sqlite|postgres|tui]   # the app layer the same way; CI builds each
cargo test -p cicada-app               # the crate alone: without the terminal UI, and without the check of docs/reference/cli.md
CICADA_UPDATE_CLI=true cargo test --workspace         # rewrites docs/reference/cli.md (or -p cicada-app --features tui)
CICADA_UPDATE_CONTRACTS=true cargo test --workspace   # rewrites contract snapshots that differ, and docs/reference/contracts
CICADA_UPDATE_OPENAPI=true cargo test --workspace     # rewrites docs/openapi.json
CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada   # with the PostgreSQL tests
PROPTEST_CASES=100000 cargo test -p cicada --lib tests::fuzz   # a long randomised run; a short one is part of cargo test
CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture   # E2E and fault tests in Docker, ~3 min; see e2e/README.md
```

PostgreSQL for tests: `docker compose up -d postgres` (port 55432). Tests that need it are skipped when
`CICADA_PG_URL` is unset.

## Map

| Path | What |
| --- | --- |
| `crates/cicada` | Core: `address`, `message`, `actor` (the `Actor` trait), `turn`, `watch` (`WatchTarget`, the notifications `Failed` and `Stopped`, the `FailureCause` of a `Failed`, the check of a watcher), `observer` (`TurnObserver`), `runtime` (node, `Worker` and its `TurnSource`, executor, the claim limit), `store` (`Store`, the private `Backend` trait, model, support; `sqlite` and `postgres` behind their features), `testing` (feature `testing`: in-memory store, clocks, `TestActors`), `contract`, `schema`, `retry`, `retention`, `time`, `api` (HTTP wire model), `tests` (the conformance suite, the store, fuzz and runtime tests that need the private trait) |
| `crates/cicada-test` | Exports `TestActors`, the clocks and `claim_and_abandon`; contract snapshots and state samples |
| `crates/cicada-axum` | HTTP API routers |
| `crates/cicada-client` | Typed HTTP client |
| `crates/cicada-app` | `App`: a set of actors as a binary with `serve`, `worker` and the operator commands (`command`, `settings`, `node`; the command line `cli`, `operator`, `config`, `output`, and `tui` behind its feature; `cli_reference`, the test of `docs/reference/cli.md`); the binary `cicada` |
| `crates/cicada-otel` | The telemetry observer |
| `crates/cicada-actors` | The ready-made actors (layer 3): `supervisor`, `dead_letters`, `topic`, `scheduler`, `rate_limiter`, `debouncer`, `batcher`, `event`; `flow`, durable flows under a host's type name (a test flow's contract in `tests/contracts`); `StoredMessage` and `Schedule`; their contracts are checked against `docs/reference/contracts` in `tests/wire_formats.rs` |
| `examples/`, `sample-service/` | Worked examples, the doctests of the README and the guides (`src/lib.rs`, `build.rs`) and the checks of the docs (`tests/docs.rs`); a runnable node |
| `e2e/` | End-to-end and fault tests against sample nodes in Docker |
| `docs/` | `architecture.md`; `guarantees.md`, the tests behind each guarantee; `guide/`, one guide per layer; `operations.md`; `reference/`, the generated help of the command line (`cli.md`) and the contracts of the ready-made actors (`contracts/`); `openapi.json`; `adr/` and its index `adr/README.md` |
| `openspec/` | Specs, changes in progress and finished changes |
| `scripts/` | `release-check.sh`; `status.py`, the order and state of the changes; `review.py`, the review requests between the loops |
| `.claude/skills/` | The three loops (ADR 0030): `design-loop`, `build-loop` with the builder's brief, `review-loop` with the briefs of a design and a code review |

## Rules

- [docs/architecture.md](docs/architecture.md) is the reference for behaviour. This repo has no contract with the
  Kotlin implementation (ADR 0008). A change to a guarantee or a wire format needs an ADR; a schema change is an
  appended migration.
- Tests for runtime behaviour run on virtual time: `#[tokio::test(start_paused = true)]` with `TestActors`.
- Store backends must pass the conformance suite (`crates/cicada/src/tests/conformance`). The backend trait is
  private to `cicada` (ADR 0018): a test that calls it lives in `crates/cicada/src/tests`, one that needs only the
  public API in a `tests/` folder.
- Releases are tags on `main`; [docs/operations.md](docs/operations.md), Releasing, has the steps. Tag a release when
  its change is implemented, every check in Releasing has passed, and the work is committed on `main`; never move or
  delete a tag that has a GitHub Release.
- Public items have doc comments (`missing_docs` is an error). Keep comments as dense as the surrounding code.
- The Rust blocks of `README.md`, of the guides in `docs/guide/` and of `docs/operations.md` are doctests of
  `cicada-examples` (`ReadmeDoctests`, `CoreGuideDoctests`, `HttpGuideDoctests`, `AppGuideDoctests`,
  `ActorsGuideDoctests` and `OperationsDoctests` in `examples/src/lib.rs`): a change to a public API must keep them
  compiling. Use `no_run` for blocks that start a server or need a database. A block that shows a worked example
  names its file of `examples/`. User-facing behaviour is documented in the guide of its layer.
- Generated references are checked for drift by a test and rewritten by a variable: a change to the command line
  regenerates `docs/reference/cli.md` (`CICADA_UPDATE_CLI=true`), one to a contract of a ready-made actor its file
  in `docs/reference/contracts/` (`CICADA_UPDATE_CONTRACTS=true`), one to the HTTP API `docs/openapi.json`
  (`CICADA_UPDATE_OPENAPI=true`). Commit what they write.
- `docs/guarantees.md` names the tests behind each of G1–G6 and S1–S10, and `examples/tests/docs.rs` fails when a
  named test is gone or a guarantee has no row: a change that adds a guarantee, or a test that proves one, adds it
  to the table. The same file checks the relative links and anchors of every Markdown file outside `target` and
  `openspec/changes/archive`.
- A new ADR gets its row in [docs/adr/README.md](docs/adr/README.md).

## Specs and changes: OpenSpec

Specs, changes, designs and tasks live in `openspec/` ([OpenSpec](https://github.com/Fission-AI/OpenSpec), ADR 0010).
Work on them goes through three loops (ADR 0030).

- `openspec/specs/`: what the system does, one folder per capability. It grows as changes are archived;
  `docs/architecture.md` stays the reference for behaviour.
- `openspec/changes/<name>/`: a change: `proposal.md`, `design.md`, `tasks.md` and spec deltas. A change with only a
  `proposal.md` is not started. `tasks.md` is written last: a change that has one is agreed and ready to build.
- Every proposal has an `Order` section that names the change before it, so the open changes form one line.
  `python3 scripts/status.py` prints the line and the state of each change.
- `openspec/changes/archive/`: finished changes, and the old epics and issues (`2026-11-09-work-tracker-history/`).
- Flow: three sessions. `/design-loop` describes the next change with the owner, has it reviewed and commits it as
  ready. `/build-loop` has a subagent build the first ready change in a worktree under `.claude/worktrees/`, has it
  reviewed, and lands and releases it on the owner's yes. `/review-loop` reviews; any agent harness can run it by
  reading `.claude/skills/review-loop/SKILL.md`. They hand over through files: `scripts/review.py` keeps the review
  requests in `.reviews/`. A change is small (one behaviour, at most 5 tasks) and built through these loops unless the
  owner says otherwise. Every landed change is a release. The rules for the artifacts are in `openspec/config.yaml`.
- Out-of-scope findings become a change with only a `proposal.md` and an `Order` section.
- Use the CLI (`openspec list`, `openspec show`, `openspec validate`) to read and check them.
