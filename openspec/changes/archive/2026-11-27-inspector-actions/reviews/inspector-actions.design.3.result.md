Verdict: not yet

- **Design §6.** With paging idle, soft refresh inherits the same-actor pane deduplication rule. A pending pre-write mailbox read can leave stale rows after Purge. Require a post-answer pane read on both paths.
- **Design §5.** The client can put an arbitrary middleware response body in `Problem.detail`. Specify a status-only fallback when alert text cannot be safely redacted.

The revision covers selection-fenced Delete, cache eviction and page fencing, the fresh paging fallback through Live pause, and cursor-based pending counts with “not read.” The proposal matches the confirmations and “cancel requested” stands alone. Strict OpenSpec validation passed. Runtime and AppKit checks unrun.
