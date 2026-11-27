# Design

Live updates (`inspector-live`) will reload the page on screen often. Today a page's Refresh starts over: the first page
only, the selection cleared, the open rows folded. That is right for ⌘R but would make live updates unusable. This
change adds a soft refresh to each page's model: it re-reads what is loaded, within fixed limits, and keeps what the user
sees. `inspector-live` calls it; ⌘R keeps today's full Refresh.

## Decisions

1. **What a soft refresh reads, bounded.** For a paged list (Actors, Messages, Outcomes, Watches, Running Turns): its
   root pages from the start, by cursor, as far as the rows loaded before, but at most 3 pages (300 rows). For Actors'
   open rows (`inspector-children`): the first page of children of at most 20 open rows, those nearest the visible area
   first. For the cached pages (Types, Nodes, the Overview): their one read. At most 4 requests at a time, and the whole
   soft refresh has a deadline of 10 s: what has answered by then is applied, the rest left as it was.
2. **Rows inside and outside the refreshed range.** Rows within the re-read range are replaced by the new answer, so a
   row that is gone there disappears. Rows past it (beyond 300, or children of rows not re-read) stay as they were,
   marked "not refreshed" (a dimmed time in the footer: "rows past 300 as of HH:MM"), and are never removed by a soft
   refresh. A manual Refresh clears the mark.
3. **Putting back the view.** After the answer is applied: the selection by its id or path when that row is still
   there (else cleared, and the pane emptied, as a manual Refresh does); the scroll position by the first visible row's
   identity, or the same offset when that row is gone; the open rows by `inspector-children`' rule. The pane is read
   again under its own generation when its row was re-read. The search prefix, the status switch and the filter text do
   not change.
4. **The user wins.** A soft refresh takes a snapshot of the view's state (selection, first visible row, open rows, the
   pane's generation) when it starts. When the user changes any of them while it runs (selects, scrolls, folds, picks
   Show Actor, presses Retry, types), the refresh applies its rows but not the restore of what changed; a Show Actor
   jump or a Retry in progress is left alone. A new prefix, status, page switch or context switch drops the soft refresh
   entirely (its generation no longer matches).
5. **One at a time.** At most one soft refresh runs per page; a request while one runs is remembered once and starts when
   it ends. A manual Refresh cancels a running soft refresh.
6. **Where it lives.** A `soft_refresh` on each page's model in `cicada-inspector`, with the limits in one place; the
   view's state for the snapshot and the restore comes from the AppKit layer through the model's existing selection and
   fold state, plus the first visible row the view reports. Tests against an in-process server, on virtual time with a
   wall-clock bound.

## Tests

Per paged page: a soft refresh re-reads up to the loaded rows and at most 300; a row gone within the range disappears, a
row past it stays marked; the selection kept, and cleared with an empty pane when its row is gone; the first visible row
kept; at most 4 requests at a time; the 10 s deadline applying what answered. Actors: open rows' first pages re-read for
at most 20, nearest the visible area first; fold state kept. Cached pages: one read, selection kept. The user wins: a
selection, scroll, fold, Show Actor or Retry during a soft refresh is not undone. A new prefix or context drops it. One
at a time, with one remembered request. A manual Refresh cancels it. The window's scroll and fold restore get a manual
check list in the tasks.

## Docs

The handoff tells the docs session that soft refresh is internal (no user-facing change by itself) and that
`inspector-live` will describe it; the changelog's 0.26.0 section gets nothing until `inspector-live`, or a line under
Changed if the builder finds a user-visible effect.
