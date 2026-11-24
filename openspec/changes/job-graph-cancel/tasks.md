# Tasks

Normal lane, additive: 0.20.8, the unreleased batch, released together with `job-graph`, `job-graph-failures` and
`job-graph-retry` (check the `release:` line of `python3 scripts/status.py`, builder.md, Version). Builds on
`job-graph` and `job-graph-failures`, which must have landed first. Follow `design.md`, ADR 0059 and ADR 0060; the
spec is `specs/job-graph/spec.md`, a delta on `job-graph`'s capability (archive `job-graph` and `job-graph-failures`
before this one). Runtime behaviour is tested on virtual time (`#[tokio::test(start_paused = true)]` with
`TestActors`).

- [x] 1. **State and messages.** In `crates/cicada-actors/src/job_graph.rs`: the optional job field `cancelType`
  (bounds of decision 2) in the constructor and the decoder; the state's own attempt-outcome type `succeeded | failed
  | cancelled`, apart from the `Result`'s outcome type (decision 3); `JobCancel {run, job, attempt}`;
  `JobGraphMsg::Cancel { job? }` and the internal `SendCancel {job, attempt}`; the decoder's new checks (decision 4).
  Verify: unit tests of the `cancelType` bounds and of the new decoder check (a cancelled job's open or succeeded
  last attempt); `cargo test -p cicada-actors --lib`.
- [x] 2. **The handler.** `Cancel` of one job or all (decision 1), followed in its commit by `job-graph-failures`' skip,
  start and end step; the unwatch of a cancelled job's target unless another running job has it; `SendCancel` sent in
  the `Cancel` commit for a dispatched job with a `cancelType`, and acted on only by the run itself while that attempt
  is kept, dispatched and closed `cancelled`, whatever the job's status (decision 2). Verify:
  `cargo test -p cicada-actors`; `cargo clippy -p cicada-actors --all-targets`.
- [x] 3. **Virtual-time tests.** In `crates/cicada-actors/tests/job_graph.rs`, one test or more for every scenario of
  the spec delta: `Cancel` of one running job (dependent skipped, late result refused), of the whole run (ends
  `cancelled`, reported), of the whole run after a failure (ends `failed`), of one job of an otherwise succeeding run
  (ends `cancelled`), a cancel that frees a slot for the next job, of an ended job, of an unknown job, a failed
  dispatch reported after the cancel, a worker told by `cancelType`, a gone worker's `SendCancel` failing alone, and a
  job cancelled before its `Dispatch`. Verify: `cargo test -p cicada-actors --test job_graph`.
- [x] 4. **Contract and wire format.** `CICADA_UPDATE_CONTRACTS=true cargo test -p cicada-actors --test wire_formats`
  rewrites `docs/reference/contracts/cicada.job-graph.json`, which is committed; samples of `Cancel`, `SendCancel`,
  `JobCancel` and a state with cancelled jobs are taken by the schemas and the decoders, and the `JobGraphEnd` schema
  does not change. Verify: `cargo test -p cicada-actors --test wire_formats`.
- [ ] 5. **Changelog and docs handoff.** The 0.20.8 changelog section names `Cancel`, `cancelType` and ADR 0060
  (already written; ADR 0059 is not edited). `docs-handoff.md` from "Docs it breaks" in the design; nothing blocks.
  No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test docs`,
  `cargo test -p cicada-examples --test skill`.
- [ ] 6. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate job-graph-cancel --strict`, commit, then `python3 scripts/gate.py run`
  (no E2E: no sample service, HTTP crate or app command changes; E2E runs at the release).
