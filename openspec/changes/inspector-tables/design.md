# Design

The Messages, Types and Watches items of the sidebar still say "Coming soon". They were part of the owner's proposal for
`inspector-actors` and split off from it. This change gives them their tables and panes, built on the Actors page's
paging, search and generations.

## Decisions

1. **One paged-list engine.** The Actors list's paging (the server's cursor passed back unchanged, at most one page in
   flight, Retry keeping the cursor, the list generation bumped at the first key press with the request 300 ms after
   the last, cancellation on a context switch or close) is factored into a generic list in the model, used by Actors and
   by Messages and Watches. Actors' behaviour and tests stay as they are; the factoring is checked by them.
2. **Messages.** `list_pending` under the search prefix, 100 a time: id, to, type, deliver at, attempt, and a "due"
   mark when deliver at is not after the store time of the read. Pages are not a snapshot, so rows are kept unique by
   message id within a list. Selecting a message fills the pane with the message itself from its row (id, to, from,
   type, deliver at, correlation id, headers, attempt, failures, payload as highlighted JSON with the Actors pane's
   formatter and its 256 KiB cut) and a "Show Actor" button that selects its actor in Actors. No extra read: the row
   holds the message.
3. **Types.** `types` once per refresh (the API answers them all, not paged): name, contract hash, and the number of
   messages; the search field filters by name prefix locally. Selecting a type fills the pane with its state schema and
   each message's schema as highlighted JSON, one section per message in name order, each cut at 256 KiB.
4. **Watches.** `watches` with neither watcher nor target, 100 a time: target, watcher. The search field filters the
   loaded rows by watcher prefix locally (the API filters by exact watcher or target only); the footer says so.
   Selecting a watch shows its target and watcher in the pane, with "Show Actor" for the watcher and, when the target
   is one actor, for it.
5. **Show Actor and navigation.** "Show Actor" selects Actors in the sidebar (recorded in back and forward), sets the
   search to the actor's exact address and selects its row once it is loaded; if it is not there, the pane shows "not
   found".
6. **Read only, and where it lives.** No action buttons until `inspector-actions`. Model code in `tables.rs` (the
   generic list, the three pages' rows and panes) with tests against an in-process server; the `appkit` module gains
   three `NSTableView` pages sharing the Actors page's search field, footer and pane toggle.

## Tests

The generic list keeps the Actors tests passing unchanged. Messages: paging, the "due" mark, a duplicate id across
pages kept once, the pane from the row, "Show Actor". Types: the list, the local name filter, the pane's schemas and
the cut. Watches: paging, the local watcher filter and its footer, the pane and both "Show Actor" cases. Show Actor
recorded in back and forward, and "not found" for a gone actor. The window gets a manual check list in the tasks.

## Docs

The handoff gives the app guide's Inspector section the three pages; the changelog's 0.26.0 section names them.
