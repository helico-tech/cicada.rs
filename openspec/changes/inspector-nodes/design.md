# Design

The Running Turns and Nodes items of the sidebar still say "Coming soon". They were part of the owner's proposal for
`inspector-outcomes` and split off from it. This change gives both pages, on the pager, panes and Show Actor of the
earlier inspector changes.

## Decisions

1. **Running Turns.** `turns` (no node filter) through the pager under the search prefix (100 a time, the server's
   cursor passed back unchanged): actor, message id, node, lease until, and "cancel asked" when an operator asked to
   cancel. Pages are not a snapshot: within a generation the first row per actor is kept and paging goes on past a page
   of only duplicates. Selecting a turn shows its row in the pane with Show Actor; nothing more is read (the actor's
   pane on the Actors page has the rest).
2. **Nodes.** `nodes` once per refresh, first visit and next visit after a context switch (the API answers them all):
   id, status (alive, suspect, dead, from the server's view in the store's clock), label, host name, hosted types,
   running turns, and the last heartbeat. The answer carries no store time to measure an age against, so the page
   shows `last_seen` as a time next to the status the server computed, and computes no age on the Mac's clock (which
   can differ from the store's). The search field filters the cached list by id prefix locally. A failed read
   shows its error with Retry and keeps no stale list.
3. **A node's pane.** Selecting a node reads `node(id)` under the pane's generation: the node's fields and its running
   turns (actor, message id, lease until, cancel asked), each turn with Show Actor. A node that is gone says "not found";
   a failed read shows its error with Retry. Refresh clears the selection, as on the other pages.
4. **Read only, and where it lives.** No cancel until `inspector-actions`. Model code in `nodes.rs` and `turns.rs` with
   tests against an in-process server; the `appkit` module gains the two pages, `NSTableView`s sharing the search
   field (each page keeping its own filter text and placeholder), footer and pane toggle.

## Tests

Running Turns: paging by the unchanged cursor, a duplicate actor kept once and paging past a page of only duplicates,
the pane from the row, Show Actor. Nodes: one read per refresh, first visit and next visit after a context switch, the
local id filter, a failed read with Retry and no stale list, the status and `last_seen` as the server gave them. A
node's pane: its turns, Show Actor from a turn, "not found", a failed read with Retry, a stale read dropped. The window
gets a manual check list in the tasks. Tests keep a wall-clock bound and do not depend on machine speed.

## Docs

The handoff gives the app guide's Inspector section the two pages; the changelog's 0.26.0 section names them.
