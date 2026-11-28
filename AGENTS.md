# Cicada for Rust — agent guide

Durable actors on a shared SQL store, in Rust. [docs/README.md](docs/README.md) is the documentation entry point;
[docs/agents.md](docs/agents.md) routes agents using Cicada in another project to their resolved version's sources.
For behavior changes, read [docs/architecture.md](docs/architecture.md) first, then the relevant
[ADRs](docs/adr/README.md). User tasks belong in the layer guides: [core](docs/guide/core.md),
[HTTP](docs/guide/http.md), [app](docs/guide/app.md) and [actors](docs/guide/actors.md).
Use [Design with actors](docs/guide/design.md) to choose actors, ownership and protocols.
[Operations](docs/operations.md) covers running nodes; [upgrading](docs/how-to/upgrading.md) and
[releasing](docs/how-to/releasing.md) have their own procedures.

## Build and test

```sh
python3 scripts/gate.py run            # committed HEAD: record checks (PostgreSQL required)
python3 scripts/gate.py run --e2e      # add Docker E2E when the tasks require it; also checked at release
python3 scripts/gate.py status         # evidence for HEAD and what a push needs
python3 -m unittest discover -s scripts -p 'test_*.py'   # script tests, including the gate
cargo build --workspace
cargo test --workspace                 # unit, runtime, SQLite, HTTP, doctest and docs tests; every feature of cicada and cicada-app on
cargo test -p cicada-examples --doc    # only the Rust blocks of README.md, the guides, docs/operations.md and the upgrade guide
cargo test -p cicada-examples --test docs   # the table of guarantees and the links between the Markdown files
cargo test -p cicada-examples --test skill  # consumer skills: versions, routes, design/review structure, resolver and packaging (python3 3.11+)
cargo test -p cicada --test fixtures   # fixed portable expectations in fixtures/; no update variable (ADR 0047)
node --test examples/tests/websocket_session_client.test.mjs   # the example browser client (Node 26+)
cargo clippy --workspace --all-targets
cargo fmt --all                        # rustfmt.toml: 120 columns; the gate checks it
cargo deny check advisories           # RustSec advisories in Cargo.lock; deny.toml lists accepted ones
cargo check -p cicada [--features sqlite|postgres|testing]   # the gate builds the core with each feature alone
cargo check -p cicada-app [--features sqlite|postgres|tui]   # the gate builds the app layer the same way
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

Full gate checks require PostgreSQL: `CICADA_GATE_PG_URL` defaults to the compose URL above. The gate checks the exact committed
tree in its owned worktree. Once installed, the pre-push hook requires evidence for every pushed main tip, including
docs. See [local checks](docs/how-to/releasing.md#local-checks-and-the-pre-push-gate); automatic Actions are disabled.
Full checks also require Node 26 or later as `node` on PATH for `client-tests`; no npm packages are needed.
The `advisories` check needs `cargo-deny` 0.20.2 and network access; see the release guide for installation.

## Map

| Path | What |
| --- | --- |
| `crates/cicada` | Core: `address`, `message`, `actor` (the `Actor` trait), `turn`, `watch` (`WatchTarget`, the notifications `Failed` and `Stopped`, the `FailureCause` of a `Failed`, the check of a watcher), `observer` (`TurnObserver`), `runtime` (node, `Worker` and its `TurnSource`, executor, the claim limit), `store` (`Store`, the private `Backend` trait, model, support; `sqlite` and `postgres` behind their features), `testing` (feature `testing`: in-memory store, clocks, `TestActors`), `contract`, `schema`, `retry`, `retention`, `time`, `api` (HTTP wire model; `leases`, the bodies of the remote turn routes), `tests` (the conformance suite, the store, fuzz and runtime tests that need the private trait) |
| `crates/cicada-test` | Exports `TestActors`, the clocks and `claim_and_abandon`; contract snapshots and state samples |
| `crates/cicada-axum` | HTTP API routers |
| `crates/cicada-client` | Typed HTTP client; `turns`, its `TurnSource` for a worker over HTTP |
| `crates/cicada-app` | `App`: a set of actors as a binary with `serve`, `worker` and the operator commands (`command`, `settings`, `node`; the command line `cli`, `operator`, `config`, `output`, and `tui` behind its feature; `cli_reference`, the test of `docs/reference/cli.md`); the binary `cicada` |
| `crates/cicada-tray` | The tray (ADR 0074): `src/lib.rs` runs an HTTP worker from an `App` factory and tracks its lifecycle; `src/turns.rs` observes running and finished turns; `src/menu.rs`, the portable menu model, and `src/appkit`, the macOS menu bar view; `tests/control.rs` tests it over HTTP on virtual time |
| `crates/cicada-otel` | The telemetry observer |
| `crates/cicada-actors` | The ready-made actors (layer 3): `supervisor`, `dead_letters`, `topic`, `scheduler`, `rate_limiter`, `debouncer`, `batcher`, `permits`, `placement`, `job_graph`, `heartbeat_watch`, `node_down`, `event`; `flow`, durable flows under a host's type name (a test flow's contract in `tests/contracts`); `StoredMessage` and `Schedule`; their contracts are checked against `docs/reference/contracts` in `tests/wire_formats.rs` |
| `crates/cicada-containers` | The container runtime (ADR 0056): the `ContainerRuntime` trait, its types and `ContainerError`; `DockerRuntime` behind `docker` (`docker/map.rs`, its pure mappings; `docker/auth.rs`, registry logins); `FakeRuntime` behind `testing`; the container actor (`actor`, ADR 0057), with a test job's contract in `tests/contracts`; `outputs`, the host's `OutputStore` with `MemoryOutputStore` and `check_output_store` behind `testing`; `archive`, the `LogArchiver` that keeps container logs in it; `orphans`, the `OrphanWatch` that reports labelled containers without their actor; no other library crate depends on it |
| `examples/`, `sample-service/` | Worked examples, the doctests of the README and the guides (`src/lib.rs`, `build.rs`) and the checks of the docs (`tests/docs.rs`); a runnable node |
| `fixtures/` | Portable test data for other implementations (ADR 0047): canonical JSON, contract hashes, lease bodies and status classification; fixed, reviewed and checked by `crates/cicada/tests/fixtures.rs` |
| `e2e/` | End-to-end and fault tests against sample nodes in Docker |
| `docs/` | `architecture.md`; `guarantees.md`, the tests behind each guarantee; `guide/`, the layer and actor-design guides; `operations.md`; `reference/`, the generated help of the command line (`cli.md`) and the contracts of the ready-made actors (`contracts/`); `openapi.json`; `adr/` and its index `adr/README.md` |
| `openspec/` | Specs, changes in progress and finished changes; a GUI change keeps its mockup boards in its own `mockup/` folder with a `SOURCE.md`, archived with it |
| `skills/cicada/` | The consumer implementation skill (ADR 0041): `SKILL.md`, which routes a consumer's agent into the resolved source, and its read-only resolver `scripts/cicada_source.py`; checked by `examples/tests/skill.rs` with the Python tests in `examples/tests/skill/` |
| `skills/actor-design/` | The consumer design/review skill (ADR 0051): a step-by-step design interview and a review of existing systems using the resolved guide; `assets/design-template.md` for saved designs and a review-report skeleton in `SKILL.md`; uses the sibling `cicada` resolver and is checked by the skill tests |
| `scripts/` | `gate.py`, local check evidence, pre-push and release (ADR 0046); `gate-policy.json` and `ci/`, its checks; `release-check.sh`; `package-skill.py` and `package-fixtures.py`, the release archives, with shared helpers in `release_archive.py`; `status.py`, the order and state of the changes; `review.py`, the change loop's review requests |
| `.claude/skills/` | The change loop (ADR 0055): `change-loop` with the builder's brief and docs handoff template, and `review-loop` with the briefs of a design and a code review, for reviews in another harness |

## Rules

- **Documentation ownership.** The dedicated `docs` session owns README, AGENTS.md and handwritten documentation.
  Other sessions report source/test changes to it instead of editing the prose. ADRs, generated CLI/OpenAPI/contract
  references and README release pins stay with their producing change. Follow
  [the documentation maintenance procedure](docs/maintaining/maintenance.md); local hooks enforce commit ownership
  when installed. This does not provide filesystem or remote access isolation. The owner exempts documentation-only
  maintenance from library releases: merge checked docs without a version bump or tag. Builders write user guidance
  into `openspec/changes/<change>/docs-handoff.md` (template in
  [.claude/skills/change-loop/builder.md](.claude/skills/change-loop/builder.md)). A doctest, link, guarantee row or skill
  route needing new prose blocks landing until this session prepares the fix ([ADR 0045](docs/adr/0045-docs-handoff-between-loops.md)).
- [docs/architecture.md](docs/architecture.md) is the reference for behaviour. This repo has no contract with the
  Kotlin implementation (ADR 0008). A change to a guarantee or a wire format needs an ADR; a schema change is an
  appended migration.
- Tests for runtime behaviour run on virtual time: `#[tokio::test(start_paused = true)]` with `TestActors`.
- Store backends must pass the conformance suite (`crates/cicada/src/tests/conformance`). The backend trait is
  private to `cicada` (ADR 0018): a test that calls it lives in `crates/cicada/src/tests`, one that needs only the
  public API in a `tests/` folder.
- Releases are tags on `main`, published by the change loop with `python3 scripts/gate.py release vX.Y.Z` on the
  owner's yes; [the release procedure](docs/how-to/releasing.md) has the steps. The change must be implemented,
  checked and committed on `main`. Never move or delete a remote version tag, whether a GitHub Release exists or not.
- Public items have doc comments (`missing_docs` is an error). Keep comments as dense as the surrounding code.
- The Rust blocks of `README.md`, of the guides in `docs/guide/` (`containers.md` included), of `docs/operations.md`
  and of `docs/how-to/upgrading.md` are doctests of `cicada-examples` (`ReadmeDoctests`, `CoreGuideDoctests`,
  `HttpGuideDoctests`, `AppGuideDoctests`, `ActorsGuideDoctests`, `DesignGuideDoctests`, `ContainersGuideDoctests`,
  `OperationsDoctests` and `UpgradeGuideDoctests` in `examples/src/lib.rs`): a public API change keeps them compiling
  through fixes prepared by the docs session (ADR 0045). Use `no_run` for blocks that start a server or need a database.
  A worked example names its file in `examples/`. User-facing behaviour goes in its layer guide, written by the docs
  session from the change's handoff.
- Generated references are checked for drift by a test and rewritten by a variable: a change to the command line
  regenerates `docs/reference/cli.md` (`CICADA_UPDATE_CLI=true`), one to a contract of a ready-made actor its file
  in `docs/reference/contracts/` (`CICADA_UPDATE_CONTRACTS=true`), one to the HTTP API `docs/openapi.json`
  (`CICADA_UPDATE_OPENAPI=true`). Commit what they write.
- `docs/guarantees.md` names the tests behind each of G1–G6 and S1–S10, and `examples/tests/docs.rs` fails when a
  named test is gone or a guarantee has no row: a change that adds a guarantee, or a test that proves one, names
  the row in its handoff for the docs session to update. The same file checks relative links and anchors of every
  Markdown file outside `target` and `openspec/changes/archive`.
- A new ADR gets its row in [docs/adr/README.md](docs/adr/README.md).
- The consumer skills route; the guides own the knowledge. `skills/cicada/SKILL.md` covers implementation;
  `skills/actor-design/SKILL.md` holds design and review methods and uses the sibling's resolver. Both skills'
  `metadata.cicada-version` equals the workspace version. Their routes, design template, methods and report structure
  are checked by `examples/tests/skill.rs`; new or renamed headings, guides and examples keep them resolving. Missing
  consumer advice goes into its guide through the docs handoff.

## Specs and changes: OpenSpec

Specs, changes, designs and tasks live in `openspec/` ([OpenSpec](https://github.com/Fission-AI/OpenSpec), ADR 0010).
Work on them goes through one change loop (ADR 0055; two builders by ADR 0073).

- `openspec/specs/`: what the system does, one folder per capability. It grows as changes are archived;
  `docs/architecture.md` stays the reference for behaviour.
- `openspec/changes/<name>/`: a change: `proposal.md`, `design.md`, `tasks.md` and spec deltas. A change with only a
  `proposal.md` is not started. `tasks.md` is written last: a change that has one is agreed and ready to build.
  Fast-lane changes omit the design and spec deltas, as below.
- Every proposal has an `Order` section that names the change before it, so the open changes form one line.
- Every proposal's `## Impact` starts with `Crates: <names>`, or `Crates: none` for process and docs changes.
  `status.py` names up to two changes to build, whose crates meet nowhere; a change without the line is built only alone.
  `python3 scripts/status.py` prints the line and the state of each change.
- `openspec/changes/archive/`: finished changes, and the old epics and issues (`2026-11-09-work-tracker-history/`).
- Flow: one session, `/change-loop`, describes the next change with the owner, has up to two builders build changes
  that share no crate, each in its own worktree (a Claude subagent, and GPT through Codex), has a fresh reviewer of the
  other model review each change, lands the changes on `main` one at a time and in any order, and releases the batch
  when it is time. Reviews go through `scripts/review.py`, which keeps requests in `.reviews/`, records who built a
  change and refuses a reviewer of the same model family; another harness can take one with `/review-loop`
  (`.claude/skills/review-loop/SKILL.md`). Each built change hands user prose to the
  docs session in `docs-handoff.md` (ADR 0045). A change is small (one behaviour, at most 5 tasks) and built through
  the change loop unless the owner says otherwise. The rules for the artifacts are in `openspec/config.yaml`.
- The fast lane is for rustdoc and comments, tests only, and internal refactors. Its proposal has `Lane: fast` in a
  `## Lane` section, and `.openspec.yaml` has `skip_specs: true`. It has no design, spec deltas or design review;
  code review stays. Changes to the public API, wire formats, schema, guarantees, command line, actor contracts or
  docs ownership rules never use this lane.
- A landed implementation change waits under the `unreleased` heading of `CHANGELOG.md`; the change loop releases
  a batch when `python3 scripts/status.py` says `release now` in its `release:` line, or when the owner asks.
  See [releasing](docs/how-to/releasing.md) for the conditions. Documentation-only maintenance is exempt.
- Out-of-scope findings become a change with only a `proposal.md` and an `Order` section.
- Use the CLI (`openspec list`, `openspec show`, `openspec validate`) to read and check them.
