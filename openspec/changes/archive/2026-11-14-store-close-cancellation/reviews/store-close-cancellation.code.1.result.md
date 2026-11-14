Verdict: fix needed

Blocking findings:

1. crates/cicada/src/store/sqlite/change_tests.rs:305: the connection-close cancellation test assumes the first poll is Pending. sqlx's worker can acknowledge shutdown before its receiver is polled; all closes can finish in that poll, failing correct code. Also, `idle_readers <= READERS` accepts zero, so it does not prove untouched connections survive. Use a controlled close barrier after ownership transfer, cancel with known readers retained, then verify resumed cleanup. Exercise cancellation during an idle reader's close too, so taking the entire vector would fail.

2. crates/cicada/src/store/sqlite/change_tests.rs:347: the start barrier does not control the return/drain ordering. The old closed-check-before-lock implementation can pass all 20 rounds. This does not supply the planned controlled proof for the racing-reader scenario. Force the relevant flag-read/pool-lock/drain interleaving with private synchronization and show the old ordering fails deterministically while the corrected ordering empties the pool.

3. openspec/changes/store-close-cancellation/specs/store-backends/spec.md:27 and :76: “Concurrent first polls” and “Final drop and runtime shutdown” have no corresponding tests. Existing closers are polled sequentially; the existing runtime test cancels a write, not close. Add controlled overtaking between flag publication and the remaining prefix for both SQL backends, plus cancellation-then-final-drop/runtime-teardown coverage of retained cleanup resources. Verify ownership/drop behavior without promising synchronous driver cleanup.

Notes: No production defect found in the cleanup gates, retained listener join, or reader locking. sqlx 0.9 source supports the synchronous pool-close prefix. Version/generated changes and the authorized docs handoff match the scope.

Not checked: No tests, builds, validation suite, or Docker E2E run. Finishing checks assessed from the builder's report; E2E was reported unrun.
