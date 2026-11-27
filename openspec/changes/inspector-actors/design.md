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
3. **Generations for every read.** One counter per kind of read (list pages, pane) is bumped on every new prefix,
   selection, Refresh and context switch; a result carries the number it was asked under and is dropped when it no
   longer matches. So a stale page never lands in a new list, and selecting A, then B, then A again shows only the
   last read of A.
4. **The pane.** Selecting an actor reads, under the pane's generation:
   - `get_actor`: state, version, created, parent, pinned node, paused;
   - `turn(address)`: the message id, node, lease until and whether a cancel was asked; the message type and attempt
     come from the same message in the mailbox read below, when it is in that page;
   - `mailbox` of the actor: the first 50 pending messages (id, type, deliver at, attempt);
   - `watches` with the actor as watcher, and with it as the exact stored target (the API lists exact targets only, so
     a type-wide watch on its type is not shown; the tab says so), the first 50 of each.
   Each part shows its own error. An actor that is gone shows "not found" and the rest of the pane empties.
5. **JSON off the main thread.** The model formats the state as pretty JSON, cuts the text at 256 KiB with a notice,
   and tokenizes it (keys, strings, numbers, literals, punctuation) into at most 20,000 style runs; past that the rest
   is plain. The view applies the runs in one batch (`beginEditing`/`endEditing` on the text storage) with the system's
   label and accent colours, so light and dark both work and a dense state stays responsive.
6. **Read only, and the pane toggle.** No action buttons until `inspector-actions`. The toolbar gains a button that
   shows and hides the pane (the split view's collapse); Refresh (⌘R) reloads the list from the top and the pane.
7. **Where it lives.** Portable model code (`actors.rs`: the list, the turns sample, generations, the pane reads, the
   formatter) with tests against an in-process server; the `appkit` module renders an `NSTableView` with columns
   (flat; `inspector-children` turns it into an outline), the search field in the toolbar (prefix set 300 ms after the
   last key), and the pane as an `NSTabView`.

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
