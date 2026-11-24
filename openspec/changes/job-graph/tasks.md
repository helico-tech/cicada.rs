# Tasks

Normal lane, additive: the next patch. At the time of the build, take the version from the `release:` line of
`python3 scripts/status.py` (builder.md, Version): the unreleased batch's version if one is open, else the patch after
the last release. Follow `design.md` and ADR 0059; the spec is `specs/job-graph/spec.md`. Runtime behaviour is tested
on virtual time (`#[tokio::test(start_paused = true)]` with `TestActors`).

- [ ] 1. **Types, state and decoder.** In `crates/cicada-actors/src/job_graph.rs`: `JobGraph` (`cicada.job-graph`,
  `JobGraph::named`), `JobGraphMsg` (`Start`, `Result`, the public `Report`, the internal `Dispatch`, and
  `Failed`/`Stopped` for the watch on itself) with the `succeeded` and `failed` helpers (the latter cuts the reason to
  2,000 characters), `JobGraphState::new(jobs, max_in_flight, &report_to, report_type)` with its jobs, attempts, run
  status and job statuses (`skipped` and `cancelled` included, never set), `JobStart`, `JobGraphEnd`, and the error
  type; the constructor's and decoder's checks of decision 12 (cycles by Kahn's algorithm, no recursion; attempts at
  most 20, consecutive and ending at the current one); exported from `lib.rs`, with its row in the rustdoc table.
  Verify: unit tests of the constructor and the decoder (cycle, unknown, repeated and self needs, every bound, the
  attempt rules, status consistency); `cargo test -p cicada-actors --lib`.
- [ ] 2. **The handler.** `Start` and the start of ready jobs in graph order up to `maxInFlight`, each with a new
  attempt and a `Dispatch` to itself and a watch on itself; `Dispatch` acted on only when the run sent it, for the
  current attempt not yet dispatched (job message with the crate's shared retry policy, `dispatchedAt` set); a
  `Failed` the store wrote for its own `Dispatch` (causes `refused`, `commit` and others) ends that attempt `failed`
  with the cause and reason; `Result` taken only for a running job's current attempt, any other refused for good; the
  run's end and its `Report` in a turn of its own; the refusals for good of decision 12, state unchanged (decisions
  2–12). Verify: `cargo test -p cicada-actors`; `cargo clippy -p cicada-actors --all-targets`.
- [ ] 3. **Virtual-time tests.** `crates/cicada-actors/tests/job_graph.rs`, one test or more for every scenario of the
  spec: a diamond in order, the in-flight limit, a second `Start`, a gone target, an unregistered target type and a
  type that does not take the message each failing only their attempt, a foreign `Dispatch` and `Failed` ignored, a
  result for another attempt, a late result, a result from another actor taken, an output too large, a failed job
  blocking only its dependents, the end report, a report sent again, and a gone report target recovered by a later
  `Report`. Verify: `cargo test -p cicada-actors --test job_graph`.
- [ ] 4. **Contract and wire format.** `cicada.job-graph` joins the contracts checked in
  `crates/cicada-actors/tests/wire_formats.rs` (update the count in its module comment), with samples of each message,
  a state mid-run, a `JobStart` and a `JobGraphEnd` taken by the schemas and the decoders;
  `CICADA_UPDATE_CONTRACTS=true cargo test -p cicada-actors --test wire_formats` writes
  `docs/reference/contracts/cicada.job-graph.json`, which is committed. Verify:
  `cargo test -p cicada-actors --test wire_formats`.
- [ ] 5. **Changelog and docs handoff.** The unreleased changelog section names the job graph and ADR 0059.
  `docs-handoff.md` from "Docs it breaks" in the design, including that a report target treats a repeated report for
  the same run as one; nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --test docs`, `cargo test -p cicada-examples --test skill`.
- [ ] 6. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate job-graph --strict`, commit, then `python3 scripts/gate.py run` (no E2E:
  no sample service, HTTP crate or app command changes; E2E runs at the release).
