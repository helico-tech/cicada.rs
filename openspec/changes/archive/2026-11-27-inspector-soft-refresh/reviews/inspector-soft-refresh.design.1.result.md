Verdict: not yet

Blocking
1. Range end and seam (Decisions 1–2). Cursors are opaque, so define "as far as loaded": a page count, or the old last row's key. Actors must stay sorted and unique (`root_index` uses a binary search): keep only tail rows past the new last address. Messages/Outcomes: dedupe the tail by id and rebuild `seen`. Say whose cursor survives. A range that ends with no `next` drops the tail and sets `more` false.
2. Generation (Decision 4). `ListView.generation` "grows whenever the rows are replaced". A bump drops AppKit's `open`/`close`/`more_children` calls, child reads and the pager's page in flight. No bump means changing that doc. Also cover a root page or Retry that is out, and Failed paging.
3. Pane re-read (Decision 3). `select()` resets the pane to Loading, so it flashes on every refresh. Keep the old reads until new ones answer. Outcomes: what if the pane shows a cause-chain link?
4. Children (Decision 1). Not defined: pages 2+ of a re-read parent and the nested open rows under them, closed rows' stale children, and where "nearest the visible area" comes from.

Notes
- Simpler: drop the snapshot. Selection and folds live in the model, so apply them against current state. AppKit only polls scroll on a timer, so let `render_list`/`reload_table` keep the first visible row across every reloadData.
- The spec has no scenario for a failed or late read.
- Cost: up to 300 full states per refresh.
- Nodes' "no stale list" clashes with keeping old rows.
- Over 5 tasks. Split: flat lists, cached pages and the anchor first, then Actors.

Not checked: inspector-nodes code, AppKit at run time.
