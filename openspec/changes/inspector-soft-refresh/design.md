# Design

Live updates (`inspector-live`) will reload the page on screen often. Today a page's Refresh starts over: the first page
only, the selection cleared. That is right for ⌘R but would make live updates unusable. This change adds a soft refresh
to the paged lists (Messages, Outcomes, Watches, Running Turns) and the cached pages (Types, Nodes, the Overview), and
keeps the table's scroll position across every reload. The Actors page, with its children, open rows and pane, follows
in `inspector-soft-refresh-actors` (design review: all of it did not fit five tasks). `inspector-live` calls the soft
refresh; ⌘R keeps today's full Refresh.

## Decisions

1. **Where the re-read range ends.** Cursors are opaque, so the range is defined by the old last row. For the lists
   ordered by a key the client knows (Running Turns: the actor's address; Watches: target then watcher), a soft refresh
   reads root pages from the start, by cursor, until a page's rows reach or pass the old last key. Messages and Outcomes
   are ordered by the server (by delivery or finish time and sequence, not by id), so their read stops at the page that
   holds the old last message id. Either way it stops after at most 3 pages (300 rows), or when the server has no more.
2. **The seam.** The new rows replace the old ones in the range. Old rows past the range are kept and marked "not
   refreshed" (the footer: "rows past 300 as of HH:MM"), after removing any whose key or id the new range already holds;
   the seen set of Messages and Outcomes is rebuilt from both. The cursor that survives is the new range's last one, so
   paging on picks up after it, and a row it brings again is dropped as a duplicate. When the range ends with no next
   cursor, the old rows past it are dropped and the list has no more. A manual Refresh clears every mark.
3. **The list's generation stays.** A soft refresh does not bump the list's generation (whose rustdoc changes to: it
   grows on a new list, a new prefix, status, Refresh or context), so a page in flight, a Retry, the open selection and
   Show Actor's waiting jump are not dropped; the soft refresh has its own counter for its own reads. It waits while a
   root page or a Retry is in flight, and is skipped while paging has failed (the Retry row is the user's to press). At
   most one runs per page; a request while one runs is remembered once. A manual Refresh, a new prefix or status, a page
   switch or a context switch cancels it.
4. **No snapshot: the model's state at the time of applying.** Watches keeps all its loaded rows and filters them for
   the table: a soft refresh replaces the loaded rows and filters them again. The selection lives in the model, so a
   soft refresh applies its rows and then keeps the selection as it is at that moment: by id when the row is still in
   the list, else cleared with an empty pane, as a manual Refresh does. A selection the user made while it ran is simply
   the current one. A pane filled from its row (Messages, Outcomes' selected outcome, Watches, Running Turns) is filled
   again from the new row; a pane that reads (a Types schema, Outcomes' followed link with `status`) is read again
   without resetting to "Loading": its old content stays until the new read answers, under the pane's generation.
5. **The scroll position in AppKit.** Every `reloadData` of a page's table (soft refresh, paging, manual Refresh) keeps
   the first visible row by its key, or the same offset when that row is gone; the view does this itself, so the model
   needs no scroll state.
6. **Cached pages.** Types, Nodes and the Overview re-read their one answer; the selection is kept by name or id. A soft
   refresh that fails keeps the old list, marked "not refreshed" with the error in the footer's tooltip; a manual
   Refresh that fails keeps no stale list, as `inspector-types-watches` and `inspector-nodes` say.
7. **Bounds.** At most 4 requests at a time across a soft refresh, and a deadline of 10 s: what has answered by then is
   applied, the rest left as it was and marked.

## Tests

Per paged page: the range ends at the old last key or after 300 rows; a row gone in the range disappears; a row past it
stays marked; a duplicate at the seam removed; the surviving cursor pages on without duplicates; a range that ends with
no more drops the rows past it; the generation unchanged, so a page in flight and a Retry are kept; waiting while a page
is in flight, skipped while paging failed; one at a time with one remembered request; cancelled by Refresh, prefix,
status, page and context; the selection kept by id, cleared when gone; the pane re-read without "Loading"; Outcomes'
followed link re-read; the 10 s deadline and a failed read. Cached pages: re-read, selection kept, a failed soft refresh
keeping the old list marked. The scroll restore gets a manual check list in the tasks. Tests on virtual time with a
wall-clock bound, against an in-process server.

## Docs

The handoff says soft refresh is internal until `inspector-live` and names the scroll position kept across reloads as
the one visible change; the changelog's 0.26.0 section gets that line under Changed.
