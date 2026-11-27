Verdict: not yet

### Blocking

1. Design §2. `PageView<MessageView>` exposes delivery timestamps without store read time. Drop “due” to keep this inspector-only and respect S9.
2. Design §§1, 5. Build after Children lands. Extract root paging only, keeping both regression suites, per-parent cursors and path-based selection and folds. Show Actor selects the exact root path or reports absence after the first successful ordered prefix page. Bind the jump to context and generation, cancel on departure or search, and allow Retry. History stores sections only. Replay the address on Forward and restore the source filter and row on Back.
3. Design §4. Zero visible matches can leave nothing to scroll. Keep the raw cursor independent of the filter and offer Load more or Retry even when empty. Reserve “no matches” for exhaustion.
4. Design §3. Per-schema cuts leave aggregate rendering unbounded. Format and render one chosen schema on demand with the existing byte and run caps.
5. Proposal scope. Three pages plus pager extraction and history exceed five small tasks. Land Messages with paging and Show Actor first. Give Types and Watches their own changes.

### Notes

- §2. Keep first-seen rows per ID per generation and advance the raw cursor through duplicate-only pages. Refresh can recover missed rows. `list_pending` includes messages being handled.
- §3. Filter cached Types by literal name prefix without refetching.

OpenSpec strict validation passed. Rust tests and AppKit were not run.
