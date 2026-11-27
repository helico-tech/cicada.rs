Verdict: not yet

Blocking
1. Per-parent reads (design D2). Nothing says one read per parent at a time, or what folding does. Two reads for one parent under the same generation and identity both match, so both append: open, close, open, or a double "more…", duplicates children. Decide: keep loaded children on fold, or drop and re-read on open. Add a per-parent request token, like `State::page`.
2. Fold state on reload (D4). "Reading their children again when they become visible" has two readings: in the viewport, or loaded. And "gone" is not defined for an open row the reads have not reached yet (a later root page, a later child page). Rule: forget it once a page passes its address, or the list ends.
3. Item lifetime (D1). NSOutlineView does not retain its items. Say when map entries go: never before `reloadData` swaps them out. Re-opened rows are expanded through `expandItem` under `syncing`, so the expand callback does not read twice.

Notes
- D3: `get_actor` returns the whole state for each visible child. The actors design avoided that for rows. "8 at a time" can mean in flight or per batch, and reads for rows scrolled away are not cancelled. Simpler: child rows show address, type and running; the rest fills the pane on select. That also eases five tasks.
- Cycles cannot happen: a parent is fixed for life, a child create of an existing actor fails, and deleting an ancestor fails (architecture). Reads at different times can still show an actor under a former descendant; paths stay finite. Say so; no depth guard.
- A child outside the prefix shows a blank status, which looks idle.
- Selection must become a path: `render_list` re-selects by address. `wants_more` must take a root index. Every row looks expandable until read: show "No children."
- Search: is the fold set compared against the debounced prefix?

Not checked: the AppKit APIs at runtime.
