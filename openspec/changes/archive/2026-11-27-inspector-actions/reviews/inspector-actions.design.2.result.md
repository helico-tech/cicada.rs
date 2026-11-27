Verdict: not yet

- **Design §6.** If Delete A answers after selecting unrelated B, preserve B’s pane and fence notices by selection generation. Evict all cached occurrences of A and known descendants. Fence pre-delete pages that could restore them.
- **Design §6.** The paging fallback can reuse a pre-write pane read. Require a read started after acknowledgment and preserve it when Live is paused mid-refresh.
- **Design §4.** A full mailbox page can contain 49 pending messages and one running message. Count pending rows, use the cursor for partial counts, and label missing reads “not read.”

Target checks, typed 404s, the Actors delta, and confirmation keys check out. Cancel Turn asks first. Purge preserves messages under valid leases. In the tasks, use “cancel requested” alone because the handler may have finished. Align the proposal and redact alert text. OpenSpec validation passed. Runtime and AppKit checks unrun.
