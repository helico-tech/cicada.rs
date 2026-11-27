# Design

The inspector shows only the Overview. Operators most often need one actor: its state, its mailbox and its running turn
(owner's proposal; mockup page Inspector, "Actors"). This change gives the Actors page and the inspector pane. Folding a
row open to its children is the next change, `inspector-children`, and the Messages, Types and Watches tables follow in
`inspector-tables` (design review: one change for all of it did not fit five tasks).

## Decisions

1. **The Actors table, paged.** The model's `ActorsList` reads `list_actors_with_state` under a prefix, 100 rows at a
   time, passing the server's cursor back unchanged: address, type, version, created, paused. The view asks for the next
   page when the visible rows reach within 20 of the end, and checks again after rows are appended or the window is
   resized; at most one page is in flight. A page that fails keeps its cursor and shows a "Retry" row at the end.
2. **Status from one sample of running turns.** Each reload of the list (a new prefix, Refresh, a context switch) reads
   `turns` under the same prefix once, its first 200: a row is "running" when its address is among them, "paused" from
   the row itself. When the sample has a next cursor, the footer says the running status covers the first 200 running
   turns; when the read fails, the footer says the status is unknown and no row is marked running.
3. **Generations for every read.** The list has a generation, bumped at once on every key press in the search field, on
   Refresh and on a context switch; the request for the new prefix then goes out 300 ms after the last key, so nothing
   stale lands while the user types. The turns sample is read and kept under the list's generation. The pane has its own
   generation, bumped on every selection (and with the list's on Refresh and a context switch): a selection change
   invalidates only the pane. A result carries the number it was asked under and is dropped when it no longer matches,
   so selecting A, then B, then A again shows only the last read of A. A context switch and a closed window also abort
   the pending reads.
4. **The pane.** Selecting an actor reads, under the pane's generation:
   - `get_actor`: state, version, created, parent, pinned node, paused;
   - `turn(address)`: the message id, node, lease until and whether a cancel was asked;
   - `mailbox` of the actor, its first 50 rows in delivery order, each marked running or pending: the turn's message id
     is matched against those rows for its type and attempt (as that read saw them; a reclaim between the two reads can
     show an older attempt), and the pending rows among the 50 are listed (id, type, deliver at, attempt);
   - `watches` with the actor as watcher, and with it as the exact stored target (the API lists exact targets only, so
     a type-wide watch on its type is not shown; the tab says so), the first 50 of each.
   Each part shows its own error. An actor that is gone shows "not found" and the rest of the pane empties; an actor
   with no running turn shows "idle", which is not an error. Paused and running show together when both hold. The
   list keeps only the rows' metadata (address, type, version, created, paused) from `list_actors_with_state`, and drops
   their state once the row is built: the pane reads the state of the one selected actor. (`list_actors` gives addresses
   only, so the version, created time and paused flag need the listing with state; 100 rows bound what one page
   carries.)
5. **JSON off the main thread.** The model formats the state as pretty JSON, cuts the text at 256 KiB, on a UTF-8
   character boundary, with a notice, and tokenizes it (keys, strings, numbers, literals, punctuation) into at most
   20,000 style runs; past that the rest is plain. The view converts the runs to UTF-16 ranges and applies them in one
   batch (`beginEditing`/`endEditing` on the text storage) with the system's label and accent colours, so light and dark
   both work and a dense state stays responsive.
6. **Read only, and the pane toggle.** No action buttons until `inspector-actions`. The toolbar gains a button that
   shows and hides the pane (the split view's collapse); Refresh (⌘R) reloads the list from the top and the pane.
7. **Where it lives.** Portable model code (`actors.rs`: the list, the turns sample, generations, the pane reads, the
   formatter) with tests against an in-process server; the `appkit` module renders an `NSTableView` with columns (flat;
   `inspector-children` turns it into an outline), the search field in the toolbar (prefix set 300 ms after the last
   key), and the pane as an `NSTabView`.

## Tests

Model, against an in-process server: the first page and the next by the unchanged cursor; at most one page in flight; a
failed page kept for Retry; the running status from one sample, the footer's "first 200" and "unknown" cases; a stale
page and a stale pane read dropped, including A, then B, then A; the pane's parts, with a turn's type and attempt from
the mailbox and without them when the message is not in the first page; a missing actor; one part failing alone; the
formatter's cut at 256 KiB and its cap of 20,000 runs. The window gets a manual check list in the tasks. Tests on the
paused clock keep a wall-clock bound.

## Docs

The handoff gives the app guide's Inspector section the Actors page and the pane; the changelog's 0.26.0 section names
them.
