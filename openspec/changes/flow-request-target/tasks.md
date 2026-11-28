# Tasks

Normal lane, **BREAKING** (the contract of every flow type): in the same version as `flow-ends`, which the change loop
names (design, intro). Follow `design.md`; the spec is `specs/flows/spec.md`; ADR 0081 is committed with the change.
Build on `flow-ends` as landed. Run the crate's tests: `cargo test -p cicada-actors`.

- [ ] 1. **`Start` and the route.** `FlowMsg::Start { reply: ReplyRoute<FlowOutcome<Value>> }`, `FlowMsg::start`, and
  `Step::request_flow::<F>(id, target)` returning `Request<FlowOutcome<F::Output>>`; the state's `reply` and `replied`;
  the first route saved (an actor route without a correlation id gets the `Start`'s), a different later route refused
  for good, the same one a no-op; `Start` acting as a host `Continue` on a running or cancelling run (decisions 1 and
  2). Verify: tests for the scenarios "A flow starts and awaits a child run" and "A second caller"; `cargo test -p
  cicada-actors`.
- [ ] 2. **The outcome.** `FlowOutcome<O>` tagged by type (`FlowDone { run, output }`, `FlowCancelled { run }`,
  `FlowFailed { run, reason }`) (decision 3); the end turn of a run with a route arms a `Continue`; any `Continue` on an
  ended run whose route is not replied sends the outcome, marks it replied and clears `armed`; a `Start` on an ended
  run without a route replies at once; a run without a route sends nothing (decisions 4 and 5). Verify: the tests of
  decision 7 on virtual time with `TestActors`, including the deleted actor caller, the lost outcome sent again, the
  late start and the 60-level output; `cargo test -p cicada-actors`.
- [ ] 3. **The contract.** Decode checks the invariants of decision 6; regenerate the test flow's contract and the sample
  service's `flows.onboarding` snapshot with `CICADA_UPDATE_CONTRACTS=true cargo test --workspace`, and commit what it
  writes. Verify: decode tests for each broken invariant; `cargo test -p cicada-actors --test wire_formats`.
- [ ] 4. **Changelog and docs handoff.** The unreleased section of `CHANGELOG.md` gets the change with **BREAKING** (the
  flow contract, and `FlowMsg`'s new variant for exhaustive matches) and ADR 0081. `docs-handoff.md` carries the items
  of "Docs this breaks" in the design; nothing new blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs.
  Verify: `cargo test -p cicada-examples --doc` and `--test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate flow-request-target --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the sample service's contract changes).
