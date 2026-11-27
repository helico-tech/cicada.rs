Verdict: fix needed

1. `crates/cicada-app/src/node.rs:173` and `openspec/changes/worker-stop-now/docs-handoff.md:20,57` still promise lease return and completion. Stop Now during a pending post-handler type lookup can abandon without releasing (`executor.rs:497,723,746`). A handler that stops after the cancel timeout or a failed release can also leave the lease to expire (`executor.rs:477,504`). Describe release as an attempt and completion as possible. Likewise, `crates/cicada-tray/src/lib.rs:217` should tie Abandoned to the turn ending without an acknowledged commit, which can happen after a failed release.

Both app stop paths share the ordered helper. Real time is justified for the panic regression because the synchronous callback waits for a concurrently scheduled turn. The split tray spec and virtual-time test cover Offline while release is held, then abandonment after reopening it. The polling helper is test-only and crate-private. The borrow rule is documented and both comments are wrapped.

Passed: 30 tray tests, four core end_grace tests, and diff check. App tests, full gate, and live AppKit were not run.
