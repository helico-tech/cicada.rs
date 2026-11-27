# Design

The Actors table of `inspector-actors` is flat. A parent row should fold open to show the actors its turns created as
children (owner's proposal). `GET /actors/{path}/children` answers addresses only, in address order, a page at a time.

## Decisions

1. **Row identities are paths.** A row's identity is its path from the root list: the root row's address, then each
   child's address on the way down. An actor shown at the root and again as a child of another row has two rows with
   different identities, so selection and fold state stay apart. The view keeps one retained Objective-C object per
   identity (a map from the path's key to the object), as `NSOutlineView` compares items by object.
2. **Children, read on opening, paged per parent.** Opening a row reads its first 100 children with its own cursor, and
   a "more…" row loads the next 100 by that cursor, passed back unchanged; a failed read keeps its cursor and shows a
   "Retry" row under that parent. Each parent's reads carry the list's generation and the parent's identity, so a
   reload, a new prefix or a context switch drops them, as the root pages are dropped. A parent that is gone answers
   "not found" under it.
3. **Child details for the visible rows.** A child comes as an address: its type is the address's type part, and its
   running status comes from the list's turns sample (or says nothing when the sample did not cover it). Its version,
   created time and paused flag are read with `get_actor` for the child rows that become visible, at most 8 at a time,
   and kept until the list reloads; a child row shows "…" until its read answers, and a failed read shows "?" with the
   error in the pane when selected. Selecting a child fills the pane as for any actor.
4. **Fold state across a reload.** The view keeps the set of open identities. A reload (Refresh, or the same prefix
   again) re-opens the open rows that are still there, reading their children again when they become visible; an open
   identity whose row is gone is forgotten. A new prefix or a context switch starts with everything folded.
5. **The outline.** The `NSTableView` of `inspector-actors` becomes an `NSOutlineView` with the same columns; the
   address column shows the child's own address, indented. Keyboard folding (← and →) works as in any outline. The
   model stays portable (`actors.rs` gains the children, the identities, the detail reads and the fold state) with
   tests against an in-process server.

## Tests

Model, against an in-process server: opening a row reads its first 100 children and "more…" the next by the unchanged
cursor; a failed page kept for Retry under its parent; a reload drops a parent's stale page; an actor at the root and
as a child keeps two identities with their own fold state; child details read for visible rows only, at most 8 at a
time, with "…" and "?" states; the fold state kept across a reload and forgotten for a gone row; a new prefix folds
everything. The window gets a manual check list in the tasks. Tests on the paused clock keep a wall-clock bound.

## Docs

The handoff gives the app guide's Actors section the folding; the changelog's 0.26.0 section names it.
