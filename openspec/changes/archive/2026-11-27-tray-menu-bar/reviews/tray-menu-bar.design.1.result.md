Verdict: not yet

## Blocking

1. **Stop Now by abort does not stop the turns** (design.md D6, spec "Stop Now"). Turns run in their own tasks (`worker.rs` `hand_off` uses `tracker.spawn`; `executor.rs:433-442` spawns the work and `keep_lease`), and `Worker` has no `Drop`. An abort drops only `run_with` and its pending `worker.stop()`. The turns keep renewing their leases and may commit, and their records stay running. The tray says `Offline` while work runs, and a reconnect runs beside it. Fix: cancel through the worker with a core API (`Worker::cancel_turns()`, or a stop raised to zero grace), plumbed through `Stop`. Cancelled turns release their leases. Move this into its own change, which also keeps this one at 5 tasks or fewer.
2. **Quit is undefined** (D2). Not decided: how it waits for `Offline` without blocking the main thread; `terminate:` (calls `exit`, so `run` never returns its `ExitCode`) or `stop:`; logout (`applicationShouldTerminate:` → `NSTerminateLater`) and Ctrl-C. Add scenarios.
3. **A library cannot enforce the deployment target** (D5). The binary sets `MACOSX_DEPLOYMENT_TARGET` (rustc defaults to 11.0). Before 14.4, `subtitle` is an unknown selector, so the app crashes. Decide: refuse at `run` after an OS check, or fall back with `available!`. Add a scenario.

## Notes

- "In place" (D3): update existing items and insert or remove only changed rows. Recreating items breaks the keyboard highlight and closes open submenus.
- `claimed_at` uses the store's clock and "now" the local clock. Clamp negative elapsed times to 0.
- Timer and action targets need `define_class!`; a block-based `NSTimer` adds `block2`.
- D8: say where a pending context switch lives, and which pick wins if the user picks again while stopping.

## Not checked

Apple headers (from memory: header and badge 14.0, subtitle 14.4); cargo deny.
