## Implementation

- [x] 1. Reconcile the landed lease/registration APIs, then add the public runtime report/effect/result types and
  the plain ActorCreate, Control/ControlEffect and WatchChange exports described in the design. Keep Backend,
  CommitRequest/Result, OutgoingMessage, Claim and EffectBuffer private. Extract shared helpers for run-report
  conversion, report-to-Decision policy, sanitization/request construction and fallback selection; keep the local
  executor's retry, fallback, handler-phase, validation order, observers and deadlines unchanged. Exercise every
  existing outcome/failure mapping with the local runtime/claim-limit/fault tests, including timeouts after a
  returned handler, state encoding and post-handler registry failures. Run the affected core runtime and fuzz tests.
- [x] 2. Add ActorSystem::complete_turn with failure-combination validation, remote storage-limit backstops,
  Pending/Running retry snapshot, derived identities and at most one commit attempt. Return exact Rejected failure
  reports for known backend refusal; reject a refused terminal failure without recursion. Preserve logging/wakes,
  no initialization/config dependency, no server observers and no release. Add public tests in
  crates/cicada/tests/completion.rs for all report/state/effect variants, every failure combination, retries and
  out-of-range reschedule fallback, structural/identity refusal, caller send ids, server-derived from/caused_by,
  state validation discarding effects, raw buffer order, peer/unstarted/stopped servers and close. Cover memory,
  SQLite and PostgreSQL where configured; use virtual time for runtime behavior. Run targeted completion tests and
  existing backend conformance unchanged.
- [ ] 3. Add virtual-time fault/race tests in crates/cicada/src/tests/runtime/remote_completion.rs using Faulty/Faults.
  Cover an error before commit, application followed by reply loss, concurrent identical/different reports, stale
  epochs and delete/recreate, status-read failure, hidden outcomes under retention, and lost rejection responses.
  Test the explicit acknowledged-Rejected transition: only the returned Failed report is retried after an uncertain
  failed commit, even if an original target later appears. Drop complete_turn during an in-flight commit and retry
  the current report through a peer, including SQL cancellation/settlement coverage. Assert at most one outcome and
  effect/notification set, no handler re-execution and no release call on every path. Run these tests and the existing
  local lease/commit failure suite; do not add sleep-based runtime race tests.
- [ ] 4. Update ADR 0038/index as needed, architecture, core guide, operations and public API docs. Document a report
  as a finished buffer, server policy versus executor duties, cancellation arbitration, one-attempt completion,
  validation-derived failures versus Rejected, retention/acknowledgement uncertainty and no release after handler
  end. Add a compiled example that handles Completed/Rejected/LeaseLost without implying the HTTP worker exists yet.
  Link new G3/G4/G6/S2/S5 tests from docs/guarantees.md as applicable; update CHANGELOG.md and version. Run
  `cargo test -p cicada-examples --doc`, `cargo test -p cicada-examples --test docs` and affected crate doctests.
  No HTTP route, transport trait, dispatcher, receipt table or Backend/schema change belongs here.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate complete-leased-turns --strict`; include PostgreSQL with CICADA_PG_URL where available and record
  any unavailable optional checks. Preserve local executor/fuzz behavior and all existing conformance checks;
  regenerate version-bearing references only when existing drift tests require it.
