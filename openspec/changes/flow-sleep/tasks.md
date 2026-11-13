## Implementation

- [x] 1. Reconcile the integration with the landed durable-flows runner, then add lazy Step::sleep and New(Sleep)/
  Waiting scheduling using the existing journal and armed continuation. Validate MAX_DELAY and checked timestamps
  without changing the wire shape. Add library tests for zero/fractional durations, bounds, ids, count and latch
  precedence; run the actor library tests and API doctests.
- [x] 2. Extend virtual-time flow integration tests for the full sleep spec: saved deadline, changed-duration replay,
  early manual Continue/deliver-now/retry/redrive, no second armed timer, late wake, zero-duration commit boundary,
  timer loss, pause/delete, failure after sleep and no work after an abandoned step. Assert closure counters,
  journal contents and pending/armed messages; run those targeted tests and unchanged flow contract/sample fixtures.
- [ ] 3. Extend the flow example and actors-guide doctests with durable wait and recovery, documenting the limits,
  code-deployment requirement and unchanged contracts. Update the changelog. Verify the example, unchanged
  sample-service flow contract snapshots/texts, `cargo test -p cicada-examples --doc` and
  `cargo test -p cicada-examples --test docs`; do not bless a contract difference by regeneration.

## Finishing checks

- [ ] 4. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate flow-sleep --strict`; record results and any unavailable optional checks.
