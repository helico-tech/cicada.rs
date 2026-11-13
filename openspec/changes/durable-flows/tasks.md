## Implementation

- [x] 1. Add the public Flow/FlowActor/FlowRun/FlowState/FlowMsg/Step API, journal/status types and validated serde in
  `cicada-actors`, with exports. Implement ordered replay, lazy Step futures, InProgress and outcome latches, result
  validation and the armed continuation protocol. Add library tests for state/id/count/result validation and API
  doctests showing ordinary async closures, error propagation and typed refs without requiring F: Clone/Debug.
  Run the actor crate's library tests and doctests.
- [x] 2. Add virtual-time TestActors integration tests for every flow scenario: one new step per commit, replay and
  changed-order/type/early-return errors, retry and terminal watch notifications, interruption before commit with
  stable external keys, public recovery, done/control semantics, depth/count limits, and swallowed failures.
  Include external timeout/select cancellation followed by Ok/Err/another step, caught invocation/serialization
  panics, an unpolled future, and timeout inside a closure; count actual closure invocations. Run those targeted tests.
- [x] 3. Pin a test flow's contract in actor-test fixtures, plus literal initial/progress/done/Sleep state samples
  and FlowState::result behavior in wire-format tests. Preserve the planned sleep representation without exposing
  Step::sleep yet. Verify valid and invalid state samples and regenerate only the intended fixtures.
- [x] 4. Add `examples/src/flow_onboarding.rs` alongside the unchanged steps example, using its service interface.
  Test normal completion, failure/recovery and idempotency keys; host it in the sample service without automatic runs
  and add its contract snapshots/texts and message/state samples. Verify example and sample-service targeted tests;
  regenerate snapshots with `CICADA_UPDATE_CONTRACTS=true` where applicable.
- [ ] 5. Add actors-guide doctests for hosting, creation, execution, result reads, watches/recovery and host contract
  snapshots. Document replay obligations, timeouts inside closures, run-key reuse, code evolution, limits and cleanup.
  Apply ADR 0031's architecture description; update guide tables, README, crate docs, AGENTS map and changelog.
  Verify `cargo test -p cicada-examples --doc` and `cargo test -p cicada-examples --test docs`.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate durable-flows --strict`; record results and any unavailable optional checks.
