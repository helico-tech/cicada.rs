# Design

The inspector shows only the Overview. Operators most often need one actor: its state, its mailbox, its running turn
and its children (owner's proposal; mockup page Inspector, "Actors"). This change gives the Actors page and the
inspector pane; the Messages, Types and Watches tables follow in `inspector-tables` (one change for all four did not
fit five tasks).

## Decisions

1. **The Actors table, paged.** The model's `ActorsList` reads `GET /actors` with state
   (`list_actors_with_state`) under a prefix, 100 at a time with the server's cursor: address, type, version, created,
   paused. The view asks for the next page when the user scrolls within 20 rows of the end; at most one page is in
   flight. Status: "paused" from the row, "running" when the address is among the running turns of one `/turns` read
   per page (its first 200; past that the status says nothing, as the Overview does with "more").
2. **Children, folded.** A row folds open to show its children (`/actors/{type}/{key}/children`, first 100 and a
   "more…" row that loads the next 100), read when it is opened, not before; a folded row keeps what it loaded until
   the table is reloaded.
3. **Search by prefix.** The search field sets the prefix 300 ms after the last key; a new prefix starts a new list
   with a new generation, and a page that arrives for an older generation is dropped, as the Overview's refresh does.
4. **The inspector pane.** Selecting an actor reads `get_actor` (state, version, created, parent, pinned node, paused)
   and `turn(address)` (message type, attempt, claim time, node) and shows them, with the state as JSON highlighted by
   a small tokenizer in the model (keys, strings, numbers, literals, punctuation; colours from the system's label and
   accent colours in light and dark). A state over 256 KB of text is cut there with a notice. Two tabs: Mailbox
   (`mailbox`, first 50 pending messages: id, type, deliver at, attempts) and Watches (`watches` of the actor, first
   50). A pane read for an actor no longer selected is dropped. The toolbar gains a button that shows and hides the
   pane (the split view's collapse), and Refresh (⌘R) reloads the table from the top and the pane.
5. **Read only.** No action buttons until `inspector-actions`. Errors show in the table's footer or the pane, with
   credentials already removed by `context_client` (as on the Overview).
6. **Where it lives.** Portable model code (`actors.rs`: list, children, search generations, pane reads, the JSON
   tokenizer) with tests against an in-process server; the `appkit` module renders an `NSOutlineView` with columns, the
   search field in the toolbar, and the pane as an `NSTabView` next to it.

## Tests

Model, against an in-process server: the first page and the next by cursor; at most one page in flight; status from the
row and from `/turns`; children read on opening and their "more…"; a search that drops an older generation's page; the
pane's reads and dropping a stale one; the tokenizer on nested JSON, escapes and numbers; a state over 256 KB cut with
the notice. The window gets a manual check list in the tasks. Tests on the paused clock keep a wall-clock bound.

## Docs

The handoff gives the app guide's Inspector section the Actors page and the pane; the changelog's 0.26.0 section names
them.
