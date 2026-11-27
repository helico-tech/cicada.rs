# Design

The Messages item of the sidebar still says "Coming soon". This change gives it its table and pane and adds Show Actor,
from a message to its actor on the Actors page. Types and Watches follow in `inspector-types-watches` (design review:
all three pages, the shared pager and the navigation did not fit five tasks). It builds after `inspector-children`,
which changes the Actors list first.

## Decisions

1. **The root pager, factored out.** Only the Actors list's root paging is factored into a generic pager in the model:
   the server's cursor passed back unchanged, at most one page in flight, Retry keeping the cursor, the generation
   bumped at the first key press with the request 300 ms after the last, cancellation on a context switch or close.
   Actors (with the children of `inspector-children` on top) and Messages use it; the Actors tests stay as they are and
   check the factoring.
2. **Messages.** `list_pending` under the search prefix, 100 a time: id, to, type, deliver at, attempt. Pages are not a
   snapshot, so within a generation the first row per message id is kept and a later one dropped; the cursor advances
   even when a page brings only duplicates, so paging goes on. No "due" mark: the answer has no store time to compare
   with.
3. **The pane, on demand.** Selecting a message fills the pane from its row (id, to, from, type, deliver at, correlation
   id, headers, attempt, failures) and formats its payload as highlighted JSON only then, with the Actors pane's
   formatter, its 256 KiB cut and its cap on style runs. No extra read.
4. **Show Actor.** The pane's "Show Actor" button records a jump in back and forward (the Actors page with the message's
   actor as its target), selects Actors in the sidebar, sets the search to the actor's exact address and, once that
   first page has loaded successfully, selects the root row with exactly that address. If the page loaded and the row is
   not there, the pane says "not found"; if the read failed, the footer shows the error with Retry and no row is
   selected. Going back returns to the message.
5. **Read only, and where it lives.** No action buttons until `inspector-actions`. Model code in `messages.rs` and the
   pager in `pager.rs`, with tests against an in-process server; the `appkit` module gains the Messages page, an
   `NSTableView` sharing the Actors page's search field, footer and pane toggle.

## Tests

The Actors tests pass unchanged over the factored pager. Messages: paging; a duplicate id across pages kept once; a page
of only duplicates still advances the cursor; the pane from the row, its payload formatted on selection only; Show
Actor selecting the exact root row after a successful load, "not found" for a gone actor, and a failed read without a
selection; the jump in back and forward. The window gets a manual check list in the tasks.

## Docs

The handoff gives the app guide's Inspector section the Messages page and Show Actor; the changelog's 0.26.0 section
names them.
