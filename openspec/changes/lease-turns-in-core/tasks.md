## Implementation

- [x] 1. Make Backend::renew atomically retain max(existing expiry, store now + TTL) in memory, SQLite and
  PostgreSQL, returning that actual expiry for both normal and cancel-requested renewal. Keep the signature,
  epoch/held checks and expired-current-epoch behavior. Add shared conformance for short/long/concurrent renewal,
  cancellation flag and fencing. Fix `s5_renew_and_commit_still_work_after_expiry_as_long_as_nobody_re_claimed` in
  conformance/robustness.rs to advance beyond the retained expiry and assert expiry before its final commit; do not
  accidentally stop testing expired commits. Run targeted renewal/conformance tests on every configured backend.
- [x] 2. Add TurnLease, LeaseTurnsResult, RenewTurnResult, MAX_LEASE_BATCH/MAX_LEASE_TYPES and the public
  ActorSystem lease_turns/renew_turn/release_turn methods. Validate inputs/config, check the name/hash map in one
  registry read, use server TTL, record the worker owner and preserve the documented lifecycle and error precedence.
  Keep local handler initialization separate and return enough time information for conservative client deadlines.
  Generalize the private claim helper to retain its ClaimBatch guard through post-processing and final state/deadline
  checks. Keep existing local dispatch/run_due_turns behavior. Add public tests in crates/cicada/tests/leases.rs for
  fields, validation, hash mismatch, unstarted/empty servers, local contention, owner visibility, peer renewal,
  repeated/stale release, expired-but-held renewal, delete/recreate fencing, stopped server and store close.
  Use virtual time for runtime behavior and run targeted lease tests with sqlite,postgres,testing features.
- [x] 3. Filter at-limit claims with one concurrent fenced failure-commit attempt per claim, reusing request and
  successful wake/observer logic without loading a hosted handler. Keep claims guarded until a confirmed commit or
  cleanup; do not add retry/keeper tasks for this path. Add mixed/all-limited batches, disabled limit, retry ignored,
  watcher-once and observer ordering tests; ordinary returned leases start no server observer. Cover commit failure,
  refusal, LeaseLost and an applied commit whose reply was lost. Keep local executor retries unchanged. Run targeted
  remote lease/claim-limit tests and the existing local claim-limit/runtime fault tests.
- [x] 4. Add deterministic fault/race tests in crates/cicada/src/tests/runtime/remote_lease.rs using the existing
  Faulty/Faults harness. Cover cancellation during the detached claim and during limit filtering; late initial and
  post-filter responses; stop during claim/filtering; failed cleanup and runtime/closed-store boundaries; checked
  monotonic overflow. Withhold a shorter peer-renewal answer and prove a competing claim cannot start before the
  original expiry. Check that a response lost after handoff leaves a recoverable lease and counts lost only when
  replaced. Run these tests on virtual time and preserve the existing delayed-claim cleanup tests; add shared-store
  SQL peer coverage where available, without sleep-based runtime race tests.
- [ ] 5. Update ADR 0037/index as needed, architecture lease/turn/lifecycle descriptions, core guide and operations
  for public leasing, trusted ownership, no node membership/local concurrency accounting, non-atomic hash checks,
  retained expiry, TTL versus deadline, best-effort cleanup, no receipts, and the G6/release obligation. Include a
  compiled example that inspects then relinquishes a lease without implying completion or a finished HTTP worker.
  Add references for new S4/S5/G3/G4 tests to docs/guarantees.md as applicable. Update CHANGELOG.md and version; run
  `cargo test -p cicada-examples --doc`, `cargo test -p cicada-examples --test docs` and affected crate doctests.
  No HTTP routes, transport trait, worker runtime or database migration belongs here.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate lease-turns-in-core --strict`. Include PostgreSQL with CICADA_PG_URL where available and record
  unavailable optional checks. Run backend conformance after the renewal behavior change; verify existing local
  executor/cleanup/fuzz behavior. Regenerate version-bearing references only as required by existing drift checks.
