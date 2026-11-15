## Implementation

- [x] 1. Add the heartbeat module's public ref, unit actor, messages, status, state and InvalidHeartbeat error, with
  dense rustdoc, typed constructor and RawState conversion. Implement defaults and cross-field/duration/deadline
  validation exactly as designed, retaining unknown-field tolerance and ordinary core depth checks. Test minimal JSON,
  all invalid status/field combinations, zero/MAX_DELAY bounds, timestamp addition/MAX_INSTANT, constructor encoding
  failures and a compile-fail message-family example. Run module unit tests and actor-crate doctests. No core API,
  store behavior, configuration update message or new create hook.
- [x] 2. Implement Beat, Rearm, eligible/ignored Expire and shared arm/cancel logic, with the current-id cancellation
  guard, self-target rejection after no-op dispatch, absolute deadline guard and atomic alarm/silent transition.
  Add virtual-time TestActors coverage for waiting/start, create-plus-Beat, repeated create, ongoing beats, one alarm,
  recovery Beat without notification, early deliver-now, fractional silence, MAX_INSTANT overflow, both equal-delivery
  orders, paused/late beats and node restart. Prove no-op state/version stability for forged/stale timers and inactive
  Rearm. Test purge/cancel and pre-/post-deadline Rearm, Rearm replacing a retrying Expire followed by stale delivery,
  repeated Rearm after expiry, and operator retry/dead-letter copies remaining ineligible. Run targeted heartbeat tests.
- [x] 3. Add failure/metadata/depth and contract proofs: self/absent/unregistered/wrong-message targets, deferred
  payload decoding, configured target retry, shared timer retry, fresh alarm id/from/no correlation, observer headers,
  depth62 accepted/depth63 refused, lost-claim recovery, and known failure atomicity. Extend the existing real-time
  SQLite registry-fault harness for retryable Store errors; do not add a core fault hook or equate unknown acknowledgements
  with rollback. Pin the new contract and state/message samples, update the catalog from eight to nine and lib.rs
  exports/rustdoc table, and regenerate the new reference with CICADA_UPDATE_CONTRACTS=true. Verify all existing
  contract hashes remain unchanged, targeted fault/contract tests pass, and any renamed test has no protected docs
  reference (otherwise name it as blocking in the handoff).
- [ ] 4. Add the compiled `examples/src/heartbeat.rs` worked example and its examples/src/lib.rs module: a 25-hour
  import watch with a synthetic initial Beat, ongoing Beats, one silence alarm and later recovery. Test on paused
  time using TestActors. Add its source-file route to skills/cicada/SKILL.md using the existing ready-made-actors route;
  do not link a future guide anchor. Run the example tests, examples doctests, docs test and skill test.
- [ ] 5. Write docs-handoff.md for the actors guide, architecture row, operational recovery caveats and installation
  version: waiting/synthetic alive, claim-time/mailbox meaning, one committed enqueue rather than exactly-once target
  effects, no recovery notice, stale armed state after timer loss, Rearm versus Beat/retry/redrive, target checks,
  duration/depth bounds and observer headers. Supply the compiled example and proposed heading, asking docs to add
  the guide-anchor skill route only once that heading exists. Identify existing G/S proof rows if new tests belong
  there; add no new guarantee. No known blocking prose if test names/routes stay valid. Add changelog and the normal
  additive patch version chosen at build time, updating skill/generated versions as required. Run docs/skill checks
  and queue status; preserve the subsequent proposals and leave their files to their owners.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate heartbeat-watch --strict`, with PostgreSQL where configured and skipped checks recorded.
  Follow the landed local-push-gate for exact committed-tree evidence and its full required coverage; release E2E
  and publication stay under that workflow and the owner's ordinary Land approval. Confirm only the new actor
  contract is added, existing hashes are stable, no core/store changes occurred, and docs handoff status is truthful.
