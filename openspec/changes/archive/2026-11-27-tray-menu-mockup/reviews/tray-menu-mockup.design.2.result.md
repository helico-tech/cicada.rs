Verdict: not yet

Round 1's five blockers are fixed. The MODIFIED requirement copies the original whole. Two new blockers:

1. **The retried row departs from the board without saying so** (design, decision 4). Board Running shows "Render · deadline passed, retry 14:40" with the red cross, and Main counts that row in "1 failed". In code that is `Rescheduled`. The design gives it an orange clock and does not count it. Either follow the board, or list a departure.
2. **Where the offline pick is kept** (decision 3). Today Connect uses the config file's `current-context`, and `menuWillOpen` reloads it from disk. Writing the pick to the file changes the CLI's default context. Keeping it in memory is lost when the menu reloads, unless it is stored apart from that. My pick: keep it in `MenuControl`, start from the config's context, never write the file.

Notes

- The bar's start and grace, a Stop Now flag and the failure time are all missing from `TrayState`. That enum is public and not `#[non_exhaustive]`. Keep the new data crate-private, or name the break.
- Stopping before the worker is ready: there is no grace. Say no bar, and no Stop Now (the existing spec says "not offer").
- `Delivery::After`: retry = finish time + delay.
- Disconnected with no contexts: what does the Connect row say?
- Five tasks fit.

Not checked: AppKit rendering. `openspec validate --strict` passes.
