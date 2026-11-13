## Implementation

- [x] 1. Add ActorChange/ActorChanges, ActorSystem::changes, private backend/Store forwarding, the bounded collector
  and shared hub/readiness state machine, with memory publication. Keep the stream Send + Unpin and next cancel-safe,
  apply literal prefix filtering with bounded poll work, and close it independently of worker stop. Add virtual-time
  fake-hub and TestActors tests in the core for startup/lag/reconnect/closed races, pending-resync receiver reset,
  cancellation/drop, dedup/count/byte overflow, unstarted/unhosted systems and memory write coverage. Run targeted
  module/runtime tests and `cargo check -p cicada` with no features and with testing.
- [x] 2. Collect SQLite changes within WriteTx and publish after confirmed commit. Add owned writer-guard settlement
  for uncertain/cancelled endings, distinguishing pre-COMMIT cancellation, in-flight COMMIT, known rollback and success.
  Preserve transaction cleanup and close behaviour. Use deterministic private driver barriers/fault fixtures to test
  cancellation before COMMIT, queued/in-flight COMMIT, commit errors, confirmed rollback, no double publication,
  settlement before resync/read, next-writer exclusion and close. Test same-Store clones and the documented boundary
  at separate file opens. Run targeted SQLite tests and `cargo check -p cicada --features sqlite`.
- [x] 3. Add PostgreSQL transactional batch collection/encoding and one parameterized flush for wake-ups plus feed.
  Map COMMIT-time SQLSTATE 54000 to Unavailable; retain other error mappings and retry classification. Refactor the
  listener to typed source events, disable eager_reconnect, gate Up on successful LISTEN, and serialize start/close.
  Test valid/malformed/noncanonical payloads, ordinary wake-up/probe isolation, thresholds, fresh retry collectors,
  error mapping (including unchanged statement-time class-54 errors), two stores on one schema, cancelled commits,
  listener termination/reconnect readiness, lag under wake-up traffic and stream close. Use gated PostgreSQL fault
  fixtures without filling the server's real notification queue; run targeted tests and the postgres-only check.
- [x] 4. Add/register shared change-feed conformance coverage for every included and excluded actor write path:
  operator/turn/child creates, equal-value state writes, pauses/resumes, restart, stop/delete descendants, idempotent
  or absent operations, mailbox/lease/watch/registry-only work, refused/rolled-back/LeaseLost commits and coalescing.
  Assert committed reads after hints; use a later marker rather than timing-based absence assertions. Run conformance
  on memory, SQLite and PostgreSQL with CICADA_PG_URL, documenting unavailable fixtures. Add new tests proving existing
  S2/S3 guarantees to docs/guarantees.md without inventing a durable-feed guarantee.
- [x] 5. Document the core API and a compiling subscribe/resync/read loop using list_actors_with_state; update
  architecture, core guide, operations, changelog/version and Store::close's stream exception. Explain exact scope,
  version-reset rationale, coarse resyncs, no replay/order guarantee, SQLite shutdown/separate-handle limits, all-writer
  upgrades, PostgreSQL queue usage/commit cost/failures and host reconciliation. Keep watches' failure/removal-only
  role explicit. Run affected doctests, `cargo test -p cicada-examples --doc` and `--test docs`. No HTTP/SSE or actor
  contract work in this piece; regenerate version-bearing references only if existing drift checks require it.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate actor-change-feed --strict`; include PostgreSQL where configured and record unavailable optional
  checks. Record any limits of fault-injection coverage; no notification-queue exhaustion experiment is required.
