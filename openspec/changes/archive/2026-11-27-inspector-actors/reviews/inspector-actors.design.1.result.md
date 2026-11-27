Verdict: not yet

Blocking findings

- Design §4 and pane scenario. `turn` lacks message type, attempt and claim time. Show message ID, node and lease expiry. Mailbox includes running entries. Define filtering, the first-50 boundary and watch direction.
- Design §§3–5. Address equality admits stale A→B→A and same-actor refresh results. Fence completions by context, list and selection request generations. Invalidate on typing before debounce, cancel on switch and close, and distinguish missing actor, idle turn and partial failures.
- Design §1. Pass opaque `next` unchanged. Guard duplicate requests by generation and cursor. Recheck the root tail after append and resize, including underfilled viewports. Keep the cursor for explicit retry. `/actors/states` downloads full states. Publish metadata only and justify page size.
- Design §1. Requerying the first 200 address-ordered turns misses later actors when truncated. Sample once per prefix refresh, mark incomplete or failed coverage, and define paused-plus-running.
- Design §§2,6. Children return addresses only. Specify sparse columns, prefix behavior and leaf discovery. Keep a cursor per parent and stable outline identities per occurrence, including duplicates at root. Give “More” a separate action.
- Design §4. Bound formatting off-thread to 256 KiB at a UTF-8 boundary. Flag truncation. This caps display only. Batch UTF-16 attribute edits and cap style runs or fall back to plain text.

Split folding into its own change to fit five tasks. Test races and retries with delayed HTTP responses. Check scroll, folding and dense JSON on macOS.

OpenSpec validated. Runtime and GUI checks unrun.
