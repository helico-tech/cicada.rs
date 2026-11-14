Verdict: pass

Blocking findings: None.

Notes: The round-two blocker is closed. In crates/cicada/src/store/sqlite/change_tests.rs:301–313, the helper now pins and polls close in its caller’s future, without spawning a task. The hook notifies only after taking the selected connection and then stays pending. The select observes that notification; leaving the helper drops the pinned close, including its owned connection and cleanup guard, before the caller inspects state. Unexpected completion panics, and failure to reach the hook is bounded by a 10-second timeout. No helper task remains for Tokio’s scheduler to remove after block_on returns.

The unchanged assertions still require exactly 4, 3, and 2 untouched readers at the three cancellation points, then successful later cleanup. The final-drop test retains both immediate task-count checks, reader retention, terminal stream, sole strong ownership, final weak-reference failure, and the runtime-teardown branch. The fix introduces no production change or new problem found in the reviewed scope.

Not checked: Reviewed only c080e678..db02d5a and necessary cancellation-hook, close, and caller context. No tests, builds, validation suite, Docker E2E, or separate experiment run. Builder checks assessed as reported: fmt and clippy passed; workspace with PostgreSQL 2726 passed, zero failed; SQLite change tests 50/50 runs; SQLite final-drop 300/300; both final-drop tests with PostgreSQL 50/50. Unrelated unchanged code was not reopened.
