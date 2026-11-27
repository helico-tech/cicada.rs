Verdict: not yet

Blocking findings

- Design §4, “Following the cause” scenario. Same-section selection creates no place today. Save prefix, status filter and row selection separately from linked IDs. Define Back for an unloaded or expired source row. Replay links by `status` outside filters. On context switch forget both stacks and the current target, and cancel outstanding reads. Distinguish unknown/hidden messages from request failures.
- Design §4, proposal “What Changes”. Caused includes operator retries (ADR 0015), so describe linked messages without attributing them all to turn sends. Give Caused one in-flight request per pane, append via unchanged cursor even after an empty page with `next`, retry the same cursor, and distinguish empty from an expired parent.
- Design §1. Specify that a switch invalidates rows, cursor, seen IDs, pending debounce, selection and pane work. All omits the filter and includes Purged. Show row status and advance through duplicate-only pages as Messages does.
- Design §2, “Failed outcomes” scenario. ⌘R is page-specific. Define whether Outcomes refresh also updates unfiltered `/stats` and what appears during loading, errors and context switches. Label the badge a store-wide sample of visible failures.

Notes

- Design §3 can use the row snapshot. Align the proposal and reserve fresh `status` for links.
- Five tasks fit: list/badge, pane/history, Caused, AppKit, handoff/checks. Test held replies across switches, duplicate-only pages and Caused retries against the in-process server. Build and UI checks not run.
