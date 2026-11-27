# Design

`inspector-soft-refresh` gives the paged and cached pages a soft refresh that re-reads what is loaded and keeps the
view. The Actors page has more to keep: its open rows and their children (`inspector-children`), its selection by path,
and a pane that reads. This change gives it the same soft refresh, on the same rules where they apply.

## Decisions

1. **The root rows, as the other paged lists.** The root list is ordered by address, so its range ends at the old last
   root address, at most 300 rows; the seam keeps old rows that sort after the new last address and drops any the new
   range holds, so the list stays sorted and unique (`root_index` searches it); the surviving cursor and a range with no
   more follow `inspector-soft-refresh`. The list's generation is not bumped, a page or Retry in flight is waited for,
   failed paging skips the refresh, and the 4-at-a-time limit and 10 s deadline cover root and children reads together.
2. **Children of open rows: the first page, at most 20, nearest the view first.** The view reports the identities of
   the open rows on screen and the first visible row (the outline knows its visible rows; the model has no scroll
   state); the soft refresh re-reads the first children page of those open rows first, then of other open rows nearest
   in outline order, at most 20 parents. A parent's re-read replaces its first page; its rows past the first page, and
   the children kept for open rows not re-read and for closed rows, stay as they were, marked "not refreshed" when the
   row is shown or reopened. A child row that the re-read no longer lists disappears with its own open descendants.
3. **Selection and open rows from the model's state.** As in `inspector-soft-refresh`, no snapshot: after applying, the
   selection is kept by its path when that row is still there, else cleared with an empty pane; the open rows follow
   `inspector-children`' rule, by the state at the time of applying, so a selection or fold the user made while it ran
   stands. A parent read in flight for an open row (`inspector-children`' per-parent marker) is waited for, not doubled.
4. **The pane re-read without a flash.** The pane's reads (`get_actor`, `turn`, the mailbox's first 50 rows, the
   watches) are made again under the pane's generation without resetting to "Loading": the old content stays until each
   part answers; a part that fails keeps its old content with the error shown beside it.
5. **Cost.** Each root page carries full actor states (`list_actors_with_state`); the 300-row bound caps a soft refresh
   at 3 such pages. The handoff names the cost so an operator with large states knows why the inspector reads them.

## Tests

The root range by the old last address, the sorted unique seam, the surviving cursor; children: the visible open rows'
first pages first, at most 20 parents, the rows past the first page and closed rows' children marked, a gone child
removed with its open descendants; a parent read in flight waited for; the selection kept by path or cleared; a fold or
selection made during the refresh standing; the pane re-read without "Loading", a failed part keeping its content; the
shared request limit and deadline. Tests on virtual time with a wall-clock bound, against an in-process server; the
outline's restore gets a manual check list in the tasks.

## Docs

The handoff says the Actors page keeps its open rows and selection across soft refreshes, names the cost of full states,
and leaves the user-facing description to `inspector-live`.
