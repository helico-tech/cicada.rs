## Implementation

- [ ] 1. Add the debouncer ref, actor, state, checked constructor/decoder, pending-message type, messages and typed
  helper with crate exports. Implement the single-turn Submit/Flush effects and checked times. Verify settings and
  state consistency in unit tests and the typed helper with a compile-fail doctest; run the actor crate's library
  tests and doctests.
- [ ] 2. Add virtual-time `TestActors` integration tests in `tests/debouncer.rs` for every spec scenario: last-wins
  replacement, quiet/maxWait, both sequence orders at an equal due time, public/empty Flush, rollback, timer-loss
  recovery, redrive into a later burst, depth boundaries, creation and target policies. Extend the existing SQLite
  store-failure fixture for registry retries and cancellation of a retrying timer if needed. Run those targeted tests.
- [ ] 3. Add contract and message/state samples to `tests/wire_formats.rs`, including empty/pending/invalid states.
  Host the type without an automatic instance in the sample service. Regenerate and commit actor references and
  sample-service snapshots/texts with `CICADA_UPDATE_CONTRACTS=true`; verify wire-format and sample-service
  contract/sample tests, ensuring existing contracts do not change.
- [ ] 4. Add a tested example and actors-guide doctests covering explicit creation, replacement, Flush recovery,
  deadlines, payload depth and reply/sender semantics. Update guide tables, README, crate docs, architecture's crate
  row, AGENTS map and changelog. Run the example test, `cargo test -p cicada-examples --doc` and
  `cargo test -p cicada-examples --test docs`.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate debouncer-actor --strict`; record results and any unavailable optional checks.
