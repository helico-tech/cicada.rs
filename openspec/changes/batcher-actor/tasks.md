## Implementation

- [ ] 1. Add the batcher ref, actor, state, validated constructor/decoder, messages and serialization helper with
  crate exports. Implement count-triggered inline forwarding, the first-item timer and public Flush with checked
  time arithmetic. Verify settings, state consistency and timestamps in library tests; run actor library tests
  and doctests.
- [ ] 2. Add virtual-time `TestActors` tests in `tests/batcher.rs` for the spec: strict count bound with queued Adds,
  count 1, timers, ordered items/null, same-time sequence orders, absent-target rollback and redriven Add, timer loss,
  restored nonempty state without armed, manual/stale Flush, depth boundaries, creation and target envelope policies.
  Extend the existing SQLite registry-failure fixture if needed. Run the batcher and store-failure integration tests.
- [ ] 3. Add message/state samples and contract references in `tests/wire_formats.rs`; register the type without
  instances in the sample service. Regenerate and commit actor references and sample-service contract snapshots/texts
  with `CICADA_UPDATE_CONTRACTS=true`. Verify wire-format and sample-service contract/sample tests and ensure no
  existing contract changes.
- [ ] 4. Add a tested batching example and actors-guide doctests with a typed items-array target. Explain group
  aggregation, public recovery, failed final Add, count versus byte limits, depth, timer loss and deadline semantics.
  Update guide tables, README, crate docs, architecture's crate row, AGENTS map and changelog. Verify the example,
  `cargo test -p cicada-examples --doc` and `cargo test -p cicada-examples --test docs`.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate batcher-actor --strict`; record results and any unavailable optional checks.
