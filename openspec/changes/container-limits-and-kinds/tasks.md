# Tasks

Normal lane, **BREAKING**: 0.21.0. The unreleased 0.20.9 batch becomes 0.21.0 with this change (builder.md,
Version: rename `## 0.20.9 — unreleased` to `## 0.21.0 — unreleased`, set 0.21.0 in every version place and rewrite
the section's summary for the whole batch, no longer "Additive"); the batch is released when this change has landed.
Builds on `container-inputs`, which must have landed and be archived first (this change's spec modifies its
requirements). Follow `design.md` and ADR 0062 (already written); the specs are `specs/container-actor/spec.md` and
`specs/container-runtime/spec.md`. Runtime behaviour is tested on virtual time (`#[tokio::test(start_paused = true)]`
with `TestActors`) and the fake.

- [ ] 1. **Wire and the out-of-memory reason.** In `crates/cicada-containers/src/actor/wire.rs`: `FailureKind`
  (`non_exhaustive`, camelCase), `kind: Option<FailureKind>` on `ContainerEnd` with `#[serde(default)]`, and a check
  in `ContainerEnd`'s own decoding that refuses a `kind` on an end that is not `failed` (decision 2), so the state and
  a `ContainerEnded` both get it; export `FailureKind` from `lib.rs`. In `src/runtime.rs`: `OUT_OF_MEMORY`, named in
  the rustdoc of `ContainerStatus::Failed` and of the trait (decision 4); `docker/map.rs` uses it. Verify: unit
  tests of the decoding (an old `failed` end without `kind`, a `kind` on a `stopped` end refused in a state and in a
  `ContainerEnded`) and Docker's `status_of` test with `OUT_OF_MEMORY`;
  `cargo test -p cicada-containers --all-features`.
- [ ] 2. **The actor.** In `crates/cicada-containers/src/actor/mod.rs`: `ContainerJob::timeout(request, settings)`
  with its default, used at the start and capped at `MAX_INSTANT` (decision 1); the create step begins with
  `inspect` and adopts a container past `Created` without `spec`, `inputs`, `create`, the copies or `start`, and the
  `inspect` after `create` of `container-inputs` goes (decision 6); a `kind` on every `failed` end by the table of
  decision 3; `start` failing with `InvalidSpec` ends at once as `notStarted` (decision 5). Update the rustdoc of
  `ContainerJob` (`timeout`; `inputs` no longer leaves a running container on a replayed step), `ContainerEnd`,
  `Outcome::Failed` and the refused-start limit. Also `ErrorKind::Unsupported`, returned by the trait's default `logs_since`, `copy_out` and `make_dirs` (update their
  rustdoc and specs), and the actor ends at once only on `Unsupported` from `make_dirs`; `Other` backs off ("Added after
  the container-inputs code review"); a test for each. Rewrap `src/actor/mod.rs:280` to 120 columns. Verify: `cargo test -p cicada-containers --all-features`;
  `cargo clippy -p cicada-containers --all-targets --all-features`.
- [ ] 3. **Virtual-time tests.** In `crates/cicada-containers/tests/actor.rs`, one test or more for every scenario of
  the spec deltas: a lost commit after start (adopted, no `spec` or `inputs` call, no `notStarted`), the kinds of
  failure (each row of the table, other ends without a kind), the timeout per request (shorter, longer, none) and a
  deadline capped at `MAX_INSTANT`, too many errors with `runtimeErrors`; update the call lists of existing tests to
  begin with `inspect` (inputs before start, no inputs, a replayed step after start). Verify:
  `cargo test -p cicada-containers --test actor`.
- [ ] 4. **Contract, version, changelog and docs handoff.**
  `CICADA_UPDATE_CONTRACTS=true cargo test -p cicada-containers --test actor` rewrites
  `tests/contracts/test.container.json`, which is committed, with samples of ends with and without `kind`. Set 0.21.0 as
  builder.md, Version, says for a BREAKING change. In `CHANGELOG.md`: the renamed heading, a summary for the whole batch
  that names the breaking change, `FailureKind`, `kind`, `ContainerJob::timeout` and `OUT_OF_MEMORY` under Added, the
  adoption, the refused start and the capped deadline under Changed, and a **BREAKING** entry: the contract hashes of
  container job types and end targets, and the `ContainerEnd` literal; change "kept across 0.20 patches" in the
  `LogCursor` entry to 0.21. `docs-handoff.md` carries `timeout`, `kind` and its table, `OUT_OF_MEMORY`, the adoption,
  the refused-start limit and the upgrade to 0.21.0 (hosts on 0.20.x follow "Changing a contract" for container job
  types and end targets; add `kind` to `ContainerEnd` literals) for the container guide and `docs/how-to/upgrading.md`;
  nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-containers
  --test actor`; `cargo test -p cicada-examples --test docs`; `cargo test -p cicada-examples --test skill`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-limits-and-kinds --strict`, commit, then
  `python3 scripts/gate.py run` (no E2E: the actor runs on the fake and no task touches `e2e/`; E2E runs at the
  release).
