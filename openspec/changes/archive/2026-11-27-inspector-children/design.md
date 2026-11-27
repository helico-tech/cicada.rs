# Design

The Actors table of `inspector-actors` is flat. A parent row should fold open to show the actors its turns created as
children (owner's proposal). `GET /actors/{path}/children` answers addresses only, in address order, a page at a time.

## Decisions

1. **Row identities are paths.** A row's identity is its path from the root list: the root row's address, then each
   child's address on the way down. An actor shown at the root and again as a child of another row has two rows with
   different identities, so selection and fold state stay apart; the selection becomes a path (re-selecting by address
   would pick the root row). Paths stay finite: a parent is fixed for life and an actor cannot become its own
   descendant, and each level is one click; reads taken at different times can still show an actor under a former
   descendant, which is harmless. The view keeps one retained Objective-C object per identity, as `NSOutlineView`
   compares items by object and does not keep them alive: an entry leaves the map only after `reloadData` (or the
   removal of its rows) has replaced it.
2. **Children, read on opening, one read per parent.** Opening a row reads its first 100 children with its own cursor; a
   "more…" row loads the next 100 by that cursor, passed back unchanged, and is disabled while that parent's read is in
   flight (one request marker per parent, as the root list has). A failed read keeps its cursor and shows a "Retry" row
   under that parent; an empty read shows "No children." Closing a row keeps its loaded children, and reopening shows
   them without a new read; Refresh reads them again. Each parent's reads carry the list's generation and the parent's
   identity, so a reload, a new prefix or a context switch drops them. A parent that is gone answers "not found" under
   it.
3. **What a child row shows.** A child comes as an address: its row shows the address, its type (the address's type
   part) and its running status from the list's turns sample. A child outside the sample's prefix shows "—" (not
   covered), not the blank of idle. Its version, created time and paused flag are read only when it is selected, by the
   pane's `get_actor` as for any actor; no read per child row.
4. **Fold state across a reload.** The view keeps the set of open identities. A reload (Refresh, or a debounced prefix
   equal to the one the set was opened under, so typing a letter and deleting it keeps it) re-opens an open row as soon
   as its row is loaded, with `expandItem` under the existing guard that keeps the expand callback from reading again;
   it then reads that row's children. An open identity is forgotten once a read page has passed its address without it,
   or its list (root or parent) has ended without it. A different prefix or a context switch starts with everything
   folded.
5. **The outline.** The `NSTableView` of `inspector-actors` becomes an `NSOutlineView` with the same columns; the
   address column shows the child's own address, indented. Paging on scroll counts root rows (`wants_more` takes the
   index of the last visible root row, not the outline's row number). Keyboard folding (← and →) works as in any
   outline. The model stays portable (`actors.rs` gains the children, the identities, the per-parent markers and the
   fold state) with tests against an in-process server.

## Tests

Model, against an in-process server: opening a row reads its first 100 children and "more…" the next by the unchanged
cursor; "more…" twice, and open, close and reopen, never show a child twice; a failed page kept for Retry under its
parent; "No children." for an empty read; a reload drops a parent's stale page; an actor at the root and as a child
keeps two identities with their own fold state and selection; a child outside the sample shows "—"; the fold state kept
across a reload, an open row re-opened when it is loaded and forgotten once a page passed it or its list ended; typing a
letter and deleting it keeps the fold state, a different prefix folds everything; paging counts root rows. The window
gets a manual check list in the tasks. Tests on the paused clock keep a wall-clock bound.

## Docs

The handoff gives the app guide's Actors section the folding; the changelog's 0.26.0 section names it.
