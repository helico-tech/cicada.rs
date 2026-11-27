# Design

`inspector-soft-refresh` gives the paged and cached pages a soft refresh that re-reads what is loaded and keeps the
view. The Actors page has more to keep: its open rows and their children (`inspector-children`), its selection by path,
and a pane that reads. This change gives it the same soft refresh, on the same rules where they apply.

## Decisions

1. **The root rows, as the other paged lists.** The root list is ordered by address, so its range ends at the old last
   root address, at most 300 rows; the seam keeps old rows that sort after the new last address and drops any the new
   range holds, so the list stays sorted and unique (`root_index` searches it); the surviving cursor and a range with no
   more follow `inspector-soft-refresh`. The list's generation is not bumped.
2. **Children by the same merge.** A parent's children are ordered by address too: its re-read replaces its first page
   by the same address-based merge (old children that sort after the new page's last address are kept, those the new
   page holds dropped), and its cursor follows the same rule (the new first page's cursor survives; a first page with no
   more drops the old children past it). Every children page, the re-read one and later ones from "more…", merges by
   address into a sorted unique list, so a page that brings a child already held (one kept past the re-read first page)
   does not add it twice. A child that a re-read shows gone is removed, and with it everything under its path: its
   cached children, its open state, its "more…" and Retry rows, and any read in flight for it (dropped by identity).
   Only then is the selection checked: kept by its path when that row still exists, else cleared with an empty pane, as
   a manual Refresh does.
3. **Which open rows, and the marks.** At the start the view reports, from the rendered outline, the open rows' paths in
   screen order (visible ones first) and the first visible row; the model keeps no scroll state. The refresh re-reads
   the first children page of the visible open rows first, then of the other open rows in outline order from the visible
   area outward (rows below before rows above at the same distance), at most 20 parents. Re-reads go depth by depth: an
   open path is scheduled only after every open ancestor chosen for a re-read has answered (or failed or missed the
   deadline), and is checked against its parent's new children first; one whose ancestor no longer lists it is pruned
   (decision 2) and not read. The visible-first ranking orders the paths within each depth. A parent not re-read,
   children past a re-read first page, and the cached children of closed rows keep their rows with a "not refreshed"
   mark, shown on the row's "more…" row or on reopening; a later re-read or a manual Refresh of that parent clears it.
4. **Paging, More and Retry.** A soft refresh starts only when no root page and no parent read is in flight; one started
   meanwhile is remembered once. While it runs, a root page or a "more…" the user asks for waits for it to end (it holds
   the per-list and per-parent markers of `inspector-children`), and a Retry the user presses cancels the soft refresh
   and runs. Failed root paging skips the refresh; a parent whose last read failed keeps its Retry row and is not
   re-read (it does not count toward the 20). A parent whose re-read fails or misses the deadline keeps its old
   children, marked, with the error on its "more…" row.
5. **One budget for every read.** Each running-turn sample carries a sequence number taken when it is asked; a sample
   applies only when no later one has applied, so an older sample never overwrites a newer status. The root pages, the
   running-turn sample (`turns` under the prefix, as the list's first page reads it), the children pages and the pane's
   five reads (`get_actor`, `turn`, the mailbox's first 50 rows, the watches as watcher and as exact target) share the
   4-at-a-time limit and the 10 s deadline; what has answered by then is applied, the rest left as it was and marked.
6. **The pane without a flash.** The pane's reads are made again under the pane's generation without resetting to
   "Loading"; a pane read already in flight for the same actor is not started twice. Each part keeps its old content
   until it answers; a part that fails keeps its old content with the error beside it. An actor that is gone shows "not
   found" and empties the rest, as today.
7. **Cost.** Each root page carries full actor states (`list_actors_with_state`); the 300-row bound caps a soft refresh
   at 3 such pages. The handoff names the cost so an operator with large states knows why the inspector reads them.

## Tests

The root range by the old last address and the sorted unique seam; children merged by address with their cursor rule; a
gone child removed with its cached children, open state, rows and reads in flight before the selection is checked; the
open rows ranked visible first then outward, at most 20; nested open paths scheduled only after their ancestors'
re-reads, depth by depth, and pruned without a read when gone; "more…" after a re-read not adding a kept child twice; an
older running-turn sample not overwriting a newer one; marks shown and cleared; a refresh waiting for a page or parent
read in flight and remembered once, a "more…" during it waiting, a Retry cancelling it; a failed parent not re-read and
not counted; a failed or late parent re-read keeping its children marked; the shared limit and deadline across root,
turns sample, children and pane; the pane re-read without "Loading", no doubled pane read, a failed part keeping its
content, "not found" as today. Tests on virtual time with a wall-clock bound, against an in-process server; the
outline's restore gets a manual check list in the tasks.

## Docs

The handoff says the Actors page keeps its open rows and selection across soft refreshes, names the cost of full states,
and leaves the user-facing description to `inspector-live`.
