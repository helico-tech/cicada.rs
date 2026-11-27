Verdict: ready for tasks

Both blockers are fixed. The pane matches the turn’s message ID across the first 50 mailbox rows, displays only pending rows from that page, and shows the attempt observed by the mailbox read. Typing invalidates the list immediately and debounces the replacement request. The turns sample shares the list generation, and selection invalidates only the pane.

The design justifies fetching state and retaining only metadata. It covers simultaneous paused and running status, idle versus missing, cancellation on context switch or close, separate watcher and exact-target reads, UTF-8 truncation, and batched UTF-16 styling.

Task notes

- Keep the two watch results distinct in the tab.
- Key page requests by generation and cursor, preserve explicit Retry, and cancel queued debounce work on switch or close.

Strict OpenSpec validation passed. Runtime and GUI checks were not run.
