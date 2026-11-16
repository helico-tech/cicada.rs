Verdict: pass

Blocking findings

None. The previous blocking finding is resolved.

Notes

- The real-row test now uses paused Tokio time and an explicitly advanced store clock. The crash helper joins its separate thread after dropping the node runtime, so the original heartbeat cannot refresh the row after the test advances store time. The shared in-memory store and clock use synchronous locks and do not require the discarded runtime.
- The test checks the actual retained row and alert details, waits for the restarted node's heartbeat timestamp before sampling recovery, then checks clean-stop absence, a completed poll at 140 seconds, and no additional notification. Shutdown waits for the heartbeat before removing the row. Neither reader nor manually driven host registers itself.
- The handoff accurately distinguishes eleven scripted-source cases from the real-row case and explains the crash mechanism. Only the test and handoff changed. The earlier optional stronger replay coverage remains outside this fix.

Not checked: Review was limited to `2ea550f..f0213c0`, the prior blocker, and necessary runtime/store/test context. `git diff --check` passed. I did not rerun Rust suites, targeted tests, PostgreSQL, Docker, or independently reproduce the reported gate evidence.
