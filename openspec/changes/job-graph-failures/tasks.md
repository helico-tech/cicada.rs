# Tasks

Normal lane, additive: 0.20.8, the unreleased batch, released together with `job-graph`, `job-graph-cancel` and
`job-graph-retry` (check the `release:` line of `python3 scripts/status.py`, builder.md, Version). Builds on
`job-graph`, which must have landed first. Follow `design.md`, ADR 0059 and ADR 0060; the spec is
`specs/job-graph/spec.md`, a delta on `job-graph`'s capability (archive `job-graph` before this one). Runtime behaviour
is tested on virtual time (`#[tokio::test(start_paused = true)]` with `TestActors`).

- [x] 1. **State rules.** In `crates/cicada-actors/src/job_graph.rs`: the skip loop (decision 1) and the end status
  (decision 2) as methods of the state, and the decoder's new checks (decision 8). Verify: unit tests of the skip loop
  (transitive, from failed, skipped and cancelled needs), of the end status (succeeded, failed, cancelled) and of each
  new decoder check; `cargo test -p cicada-actors --lib`.
- [x] 2. **The handler.** After every change of a job's status, in its commit: the skip loop, then the start of ready
  jobs, then the check of the run's end (decision 7); the `running` condition on `Dispatch` and on its failure (decision
  6); the watch on the target in the `Dispatch` turn and the unwatch when no running job has that target; a
  store-written `Failed` about a target matched to a running job's current job message, and a `Stopped` about a target,
  each ending their jobs `failed` (decision 3). From the job-graph review: a `Result` taken before its `Dispatch` ran
  leaves the attempt's `dispatchedAt` empty; say so in the doc comment of `JobAttempt`. Verify: `cargo test -p cicada-actors`; `cargo clippy -p cicada-actors
  --all-targets`.
- [x] 3. **Virtual-time tests.** In `crates/cicada-actors/tests/job_graph.rs`, one test or more for every scenario of
  the spec delta: transitive skips, a job message that fails at the worker, a deleted worker with two jobs, another
  failed message of a shared worker ignored, a worker that reports and stops in one turn (normal order), client
  notifications ignored, a slot freed by a gone worker starting the next job, and a client's result that ends a job
  before its failed `Dispatch` is reported. Update `job-graph`'s test of a failed job, whose dependents now skip.
  Verify: `cargo test -p cicada-actors --test job_graph`.
- [x] 4. **Wire format.** A sample state with skipped jobs and an ended run in
  `crates/cicada-actors/tests/wire_formats.rs`, taken by the schema and the decoder; `CICADA_UPDATE_CONTRACTS=true cargo
  test -p cicada-actors --test wire_formats` only if a doc comment changed the reference, whose hash must stay. Verify:
  `cargo test -p cicada-actors --test wire_formats`.
- [x] 5. **Changelog and docs handoff.** The unreleased changelog section names the skipped dependents, the end status
  and the watch on each target, and ADR 0060 (already written; ADR 0059 is not edited). `docs-handoff.md` from "Docs it
  breaks" in the design; nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo
  test -p cicada-examples --test docs`, `cargo test -p cicada-examples --test skill`.
- [ ] 6. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test
  --workspace`, `openspec validate job-graph-failures --strict`, commit, then `python3 scripts/gate.py run` (no E2E: no
  sample service, HTTP crate or app command changes; E2E runs at the release).
