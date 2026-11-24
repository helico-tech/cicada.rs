# Tasks

Normal lane, additive: 0.20.8, the unreleased batch, released together with `job-graph`, `job-graph-failures` and
`job-graph-cancel` (check the `release:` line of `python3 scripts/status.py`, builder.md, Version). Builds on
`job-graph-failures` and `job-graph-cancel`, which must have landed first. Follow `design.md`, ADR 0059, ADR 0060 and
ADR 0061; the spec is `specs/job-graph/spec.md`, a delta on `job-graph`'s capability (archive `job-graph-failures`
and `job-graph-cancel` before this one). Runtime behaviour is tested on virtual time
(`#[tokio::test(start_paused = true)]` with `TestActors`).

- [x] 1. **State, messages and decoder.** In `crates/cicada-actors/src/job_graph.rs`: the state's `ends` (`u32`,
  absent: 0) and `end` in `JobGraphEnd`; `JobGraphMsg::Retry { job }`; the decoder's changed status rules (decision 7:
  a `waiting` or `cancelled` job may have a last attempt closed `failed` or `cancelled`; an ended run has `ends` of 1
  or more). Update the doc comments of `JobGraphEnd` and `Report`: the highest `end` per run wins, numbers can have
  gaps, a repeat has the same number. Verify: unit tests of the decoder's changed rules and of `ends`;
  `cargo test -p cicada-actors --lib`.
- [x] 2. **The handler.** `Retry` (decisions 1–3): the refusals for good of decision 5 with the state unchanged; the
  job back to `waiting` with its output cleared; every `skipped` job back to `waiting` and the skip loop again; an
  ended run reopened; then `job-graph-failures`' skip, start and end step in the same commit. Each end raises `ends`,
  and `report` sends it as `end`; a job's `reason` in the report only when its last attempt's outcome is its status
  (decision 1). Verify: `cargo test -p cicada-actors`; `cargo clippy -p cicada-actors --all-targets`.
- [x] 3. **Virtual-time tests.** In `crates/cicada-actors/tests/job_graph.rs`, one test or more for every scenario of
  the spec delta: retry of a failed job (attempt 2 in the same commit), skipped dependents coming back (and one skipped
  for another job staying), a late result of a replaced attempt, a worker `Failed` about a replaced attempt and one about another job on a
  shared worker (both ignored; from the job-graph-failures review), retry before and after the cancel notice ran, a
  retried job cancelled before it starts (no old reason), retry within the in-flight limit, a run that ends twice (end
  2), a report while running again, a waiting job with old attempts decoded, and each refusal (a running job, a job
  whose need failed, an unknown job). Verify: `cargo test -p cicada-actors --test job_graph`.
- [x] 4. **Contract and wire format.** `CICADA_UPDATE_CONTRACTS=true cargo test -p cicada-actors --test wire_formats`
  rewrites `docs/reference/contracts/cicada.job-graph.json`, which is committed; samples of `Retry`, a reopened state
  with `ends` and old attempts, and an end report with `end` are taken by the schemas and the decoders. Verify:
  `cargo test -p cicada-actors --test wire_formats`.
- [x] 5. **Changelog and docs handoff.** The 0.20.8 changelog section names `Retry`, the end number and ADR 0061
  (already written). `docs-handoff.md` from "Docs it breaks" in the design; nothing blocks. No edit to `README.md`,
  `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test docs`,
  `cargo test -p cicada-examples --test skill`.
- [x] 6. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate job-graph-retry --strict`, commit, then `python3 scripts/gate.py run`
  (no E2E: no sample service, HTTP crate or app command changes; E2E runs at the release).
