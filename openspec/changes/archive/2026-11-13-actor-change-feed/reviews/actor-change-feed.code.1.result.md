Verdict: fix needed

Blocking findings:
1. openspec/changes/actor-change-feed/specs/change-feed/spec.md:182 — “Runtime shutdown interrupts settlement” has no test, as the builder acknowledges. The SQLite tests keep their runtime alive through settlement. Add a deterministic shutdown/resumption test retaining the Store: interrupt settlement, verify the writer guard is released and rollback-on-next-use still works, then establish a fresh subscription and read the settled database. Do not require timely notification during shutdown.
2. crates/cicada/src/tests/postgres/changes.rs:184 — The retry-isolation test cannot detect the leakage it claims to detect. Both attempts collect the same 60 addresses; ChangeCollector deduplicates them, so a leaked collection still contains 60, never 120. The retry clause of “Commit cancellation and retries” therefore lacks effective coverage. Make the first attempt collect an address absent from the successful attempt, and assert that address does not appear in the committed batch; the test should fail if the collector survives a retry.

Notes:
- Version 0.16.10 is appropriate under README’s policy: the API additions are compatible; HTTP format, schema, contracts and existing guarantees do not break. COMMIT-time 54000 remapping and notification overhead are documented.
- First-LISTEN wake-ups, terminal close, and dropping queued hints at a fresh Resync boundary agree with the design.
- Pre-existing issue, not a blocker here: postgres/listener.rs:103–105 detaches the probe task when the parent is aborted during receive. That task retains its pool after close; the new close test’s channel-text query misses parameterized probe queries.

Not checked: execution of tests/checks (trusted the builder’s report), Docker E2E, actual notification-queue exhaustion, or network blackholes. Reviewed the head artifacts, diff, implementation, tests and recovery example.
