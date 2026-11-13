## Implementation

- [x] 1. Reconcile the landed registration, lease and completion APIs, then add Worker/WorkerBuilder/WorkerConfig,
  WorkerError, SourceError and the object-safe TurnSource exports described in the design. Implement the reference
  ActorSystem adapter using public operations and forwarding for Arc<T>; document source fencing, nonshortening
  renewal and no hidden mutation retries. Test error classification, full type definitions, exact contract/hash
  handling and object safety in crates/cicada/tests/worker.rs. Run the targeted public API tests and crate doctests.
- [x] 2. Extract the private store/source executor and registry seams, preserving the local executor's policy and
  scheduling. Convert complete leases without losing message fields, return reports from shared handler work, and
  retain the full-JSON contract cache. Add server-granted timing, independent watchdog and fixed-budget remote
  completion with permanent Rejected transitions. In private runtime tests, cover short TTLs, peer TTL changes,
  late grants, renewal errors/hangs/panic, simultaneous renewal/watchdog readiness and completion-start expiry.
  Check expiration/loss before extension so polling order cannot revive a grant. Run affected runtime, observer,
  lease/commit failure and fuzz tests; keep direct-store behavior unchanged.
- [x] 3. Implement Worker initialization, polling/manual batches, shared capacity reservations and tracked cleanup.
  Handle stop during registration, malformed batch cleanup, detached lease handoff, first-error automatic stop,
  stopped waiters and draining manual work in New. Test mixed/concurrent manual and background calls, sparse/full
  polling, temporary/permanent source failures, empty workers, dropped callers, late responses, stop races and
  non-yielding work in crates/cicada/tests/worker.rs and private runtime/remote_worker.rs. Run these targeted tests
  on virtual time; assert no source lifecycle call and no capacity reuse before cleanup.
- [x] 4. Add a parity/fault matrix using an ActorSystem source and deterministic fault wrappers. Compare local and
  Worker outcomes for state/effects, raw and typed sends/creates, watches, controls, retry, panic/decode, MAX_TURN,
  overtime and observer headers. Cover completion acknowledgement loss, retention-hidden outcomes, actual failed
  Completed results, retryable-failure rejection, rejection followed by uncertain failure commit, malformed results,
  hung completion and fixed budget exhaustion. Assert no handler re-execution, no original effects after Rejected,
  and no release after handler end. Test operator cancellation through renewal separately from shutdown/manual
  cancellation, including operator cancellation followed by shutdown. Use start_paused and TestActors where
  applicable; run targeted Worker/runtime tests with memory, SQLite and PostgreSQL where configured.
- [ ] 5. Update ADR 0039/index as needed, architecture, the core guide, operations and public API docs. Include a
  compiled in-process Worker example and describe source duties, registration, timing estimates, per-turn TTLs,
  manual capacity, cancellation distinctions, bounded completion and uncertain results. Document Worker lifecycle
  independence and the absence of node membership; HTTP and CLI remain following changes. Link applicable new
  guarantee tests in docs/guarantees.md, update CHANGELOG.md and version. Run `cargo test -p cicada-examples --doc`,
  `cargo test -p cicada-examples --test docs` and affected crate doctests.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate remote-worker-runtime --strict`; include PostgreSQL with CICADA_PG_URL where available and
  record unavailable optional checks. Preserve existing conformance and local executor tests. Regenerate references
  only when existing drift checks require it; this change introduces no HTTP wire format or schema migration.
