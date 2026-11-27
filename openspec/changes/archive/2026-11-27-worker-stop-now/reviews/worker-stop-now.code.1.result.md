Verdict: fix needed

1. `crates/cicada-app/src/node.rs:451` bypasses Stop Now on callback panic. Have the first ready callback save or set its handle and a later callback panic while a turn is active. The panic path awaits the full grace period. Reuse the ordered Stop Now shutdown path after publishing the handle and test that the command still returns the panic error.
2. `crates/cicada-tray/tests/control.rs:505` reopens release before the timeout. Keep it closed and the tray can go Offline while the turn is still listed as running: `executor.rs:504` awaits release with the observer alive. A turn already completing can also commit (`executor.rs:513`). Align the tray spec (`specs/menu-bar-tray/spec.md:5`) and handoff (`docs-handoff.md:26,58`) with these boundaries, qualify the lease-return promise in `node.rs:171`, and test release held through the timeout.
3. `crates/cicada/src/tests/runtime/remote_worker.rs:597` uses a polling loop that exits promptly on the normal stop signal. Removing the early exit from the poll-loop wait would still let the test pass. Hold the polling task pending and verify that Stop Now skips that wait.
4. `crates/cicada-tray/src/lib.rs:122,226` can deadlock if a caller matches on a borrowed Stopping state and calls `stop_now` in that arm. `send_replace` needs the watch write lock. Document the borrow restriction for `stop_now` alongside the existing restrictions.
5. `crates/cicada-app/src/node.rs:171` and `crates/cicada/src/runtime/worker.rs:449` each exceed 120 columns by one. Wrap both comments.

The host-signal path polls stop first by construction. The release hold is integration-test-only. The BREAKING line and protected-file check are correct.

Not run: tests, gate, or live AppKit interaction.
