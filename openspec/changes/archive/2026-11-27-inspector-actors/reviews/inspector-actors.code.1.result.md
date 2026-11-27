Verdict: fix needed

1. `crates/cicada-inspector/src/appkit.rs:385,644–656`: Select A, then search for a prefix excluding A. The table deselects under `syncing`, but the pane still shows A and accepts its outstanding reads. Refresh rereads A too. When a prefix excludes the selection, clear it in the controller and model, invalidate the pane reads, and render the empty pane.
2. `crates/cicada-inspector/src/appkit/actors.rs:441`: Selecting a running actor puts a running row in the Mailbox tab. The design and task require pending rows only. Keep the first-50 ID match for the turn header, remove the running row from the tab, and correct `docs-handoff.md:25`.
3. `crates/cicada-inspector/src/appkit/actors.rs:420–421`: With a successful turn read and a failed mailbox read, the header claims the message is outside the first 50. Reserve that claim for a successful mailbox read without a match. Keep the running turn visible and show the mailbox error in its tab.
4. `crates/cicada-inspector/src/tests/actors.rs:297,311`: The search test waits for the new page alone. If the new turns sample answers while the test joins cancelled tasks, `has_changed` misidentifies that valid update as stale. Wait for both new reads to settle before checking for old results.
5. `crates/cicada-inspector/Cargo.toml:32`: The changed feature list is 142 columns. Wrap it to 120.

All four spec scenarios have model tests. The diff leaves protected docs untouched. The View menu item is standalone-only. Clarify that in the handoff.

Cargo tests and manual AppKit checks were not run for this review.
