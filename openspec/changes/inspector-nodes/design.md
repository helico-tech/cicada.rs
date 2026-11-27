# Design

The Running Turns and Nodes items of the sidebar still say "Coming soon". They were part of the owner's proposal for
`inspector-outcomes` and split off from it. This change gives both pages, on the pager, panes and Show Actor of the
earlier inspector changes.

## Decisions

1. **Running Turns.** `turns` (no node filter) through the pager under the search prefix (100 a time, the server's
   cursor passed back unchanged): actor, message id, node, lease until, and "cancel asked" when an operator asked to
   cancel. Every store pages turns by an exclusive actor-address cursor, so an actor never appears on two pages and no
   de-duplication is needed. Selecting a turn shows its row in the pane with Show Actor; nothing more is read (the
   actor's pane on the Actors page has the rest).
2. **Nodes, cached per page.** `nodes` is read on the first visit, on the Nodes page's own Refresh (⌘R), and after a
   context switch: at once when the page is visible, else at its next visit. It does not follow the Overview's refresh.
   The API answers all nodes: id, status (alive, suspect, dead, computed by the server in the store's clock), label,
   host name, hosted types, running turns, and `last_seen` as a time. The answer carries no store time to measure an age
   against, so no age is computed on the Mac's clock; the footer says "as of the last read" with the Mac's time of that
   read in its tooltip. The search field filters the cached list by id prefix locally. A failed read shows its error
   with Retry and keeps no stale list.
3. **A node's pane.** Selecting a node reads `node(id)` under the pane's generation: the node's fields and its running
   turns (actor, message id, lease until, cancel asked), each turn with Show Actor. The read is dropped when the
   selection changes, on Refresh and on a context switch; Retry reads the selected node again. A node that is gone says
   "not found". Refresh clears the selection, as on the other pages.
4. **Show Actor and back.** Show Actor from a turn row or a node's turn uses the exact-address jump of
   `inspector-messages`. A Running Turns place keeps its prefix and the selected turn as read; back restores the prefix
   and selects that actor's row when a page brings it, showing its turn as it is now (which may be another message), and
   when the actor no longer runs a turn the pane shows the saved row marked "no longer running". A Nodes place keeps the
   node id; back selects it and reads its pane fresh.
5. **Read only, and where it lives.** No cancel until `inspector-actions`. Model code in `nodes.rs` and `turns.rs` with
   tests against an in-process server; the `appkit` module gains the two pages, `NSTableView`s sharing the search field
   (each page keeping its own filter text and placeholder), footer and pane toggle.

## Tests

Running Turns: paging by the unchanged cursor while leases begin and end between pages, the pane from the row, Show
Actor. Nodes: a read on the first visit, on its Refresh, and after a context switch at once when visible and else at the
next visit, never on the Overview's refresh; the local id filter; a failed read with Retry and no stale list; the status
and `last_seen` as the server gave them. A node's pane: its turns, Show Actor from a turn, "not found", a failed read
with Retry on the selected node, a read dropped on a new selection, Refresh or context switch. Back to a Running Turns
place whose turn has ended or moved to another message, and to a Nodes place. The window gets a manual check list in the
tasks. Tests keep a wall-clock bound and do not depend on machine speed.

## Docs

The handoff gives the app guide's Inspector section the two pages; the changelog's 0.26.0 section names them.
