## Implementation

- [ ] 1. Add the public limiter ref, state, checked decoder/constructor, message family and typed submission helper
  in `crates/cicada-actors/src/rate_limiter.rs`, with crate exports. Implement checked slot arithmetic. Verify with
  unit tests for validation, fractional rounding, inclusive delay and timestamp overflow, and a compile-fail doctest
  for a message of the wrong family; run `cargo test -p cicada-actors --lib` and `cargo test -p cicada-actors --doc`.
- [ ] 2. Implement atomic forwarding and failure handling. Add `tests/rate_limiter.rs` using virtual-time
  `TestActors` for the spec's burst/spacing, rollback, creation, envelope, target retry, redrive, dedup and cancellation
  scenarios. Extend the existing `tests/store_failure.rs` fixture for retryable registry failures if needed.
  Verify with the limiter and store-failure integration tests; cover commit-time clamping through existing core
  delivery tests plus the guide's explicit limitation rather than introducing a private runtime hook.
- [ ] 3. Pin the contract and valid/invalid message/state samples in `tests/wire_formats.rs`; register the type
  without an automatic instance in the sample service. Regenerate and commit `docs/reference/contracts` and the
  sample-service contract snapshots/texts using `CICADA_UPDATE_CONTRACTS=true`. Verify the actor wire-format tests
  and the sample-service contract/samples tests, including unchanged existing contracts.
- [ ] 4. Add a tested example and actors-guide doctests showing startup creation, create-and-submit, target retries
  and a payload reply address. State the scheduling, commit-delay and replacement limitations. Update guide tables,
  README, crate docs, architecture's crate row, AGENTS map and changelog. Verify the example test,
  `cargo test -p cicada-examples --doc` and `cargo test -p cicada-examples --test docs`.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate rate-limiter-actor --strict`; record the results and any unavailable optional checks.
