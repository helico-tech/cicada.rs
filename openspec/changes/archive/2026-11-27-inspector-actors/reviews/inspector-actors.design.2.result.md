Verdict: not yet

- Design §4 and the pane scenario leave the mailbox boundary ambiguous. `mailbox(limit=50)` returns mixed states. Match the turn's message ID against the raw page, then display pending entries from those same 50 rows. Qualify the scenario accordingly. The attempt is a mailbox observation because separate reads can straddle a reclaim.
- Design §§2–3, 7 and the search scenario leave a debounce gap. Invalidate on typing and debounce the replacement request. Fence the turns sample with the list generation. Selection changes should invalidate only the pane.

Notes for tasks

- Retain metadata only from `/actors/states` and justify 100 full-state rows. Key requests by generation and cursor, with explicit Retry.
- Show paused and running together when both apply. Distinguish idle from missing and cancel pending work on context switch and close. Separate watcher and exact-target results.
- Cut at a UTF-8 boundary and batch UTF-16 AppKit ranges under the style-run cap.

With folding deferred, five tasks fit across the list model, pane model, JSON formatting, AppKit table, and AppKit pane. Include delayed HTTP and manual window checks in those tasks.

Strict OpenSpec validation passed. Runtime and GUI checks were not run.
