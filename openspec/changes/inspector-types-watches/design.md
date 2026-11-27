# Design

The Types and Watches items of the sidebar still say "Coming soon". This change gives them their tables and panes, on
the pager of `inspector-messages` and with its Show Actor.

## Decisions

1. **Types, cached and filtered locally.** Each Refresh (and the first visit, and a context switch) reads `types` once:
   the API answers them all, not paged. The table shows name, contract hash (shortened, the full hash in its tooltip)
   and the number of message types. The search field filters the cached list by name prefix without another request. A
   failed read shows its error in the footer with Retry and keeps no stale list. Refresh clears the selected type and
   its chosen schema, as Messages does; a context switch reads again at the next visit, as Messages does.
2. **A type's pane, one schema at a time.** The pane lists the type's schemas (State, then each message type in name
   order) in a pop-up; only the chosen schema is formatted, with the Actors pane's formatter, its 256 KiB cut and its
   cap on style runs, and a formatting that finishes after another choice or selection is dropped. So a type with many
   or large schemas costs one formatting at a time.
3. **Watches, paged, filtered locally.** `watches` with neither watcher nor target, through the pager (100 a time, the
   server's cursor passed back unchanged; the order is the stores' `(target, watcher)` order, so no duplicate check is
   needed): target, watcher. The API filters by exact watcher or target only, so the search field filters the loaded
   rows locally by prefix, matching the watcher or the target ("who watches X" as well as "what X watches"). With no
   filter, pages load on scroll as on Messages. While a filter is typed, nothing loads by itself: the footer says the
   filter covers the loaded rows and offers "Load more", one page per click, and Retry after a failed page, also when no
   row is visible. So a filter that hides most rows never reads the whole table unasked. Refresh starts paging again
   from the top and clears the selection.
4. **A watch's pane and Show Actor.** The pane shows the target and the watcher; "Show Actor" for the watcher and, when
   the target is one actor, for the target, with `inspector-messages`' jump: recorded in back and forward, the exact
   root row selected after a successful load, "not found" and a failed read apart.
5. **The shared search field.** Each page keeps its own filter text and its own placeholder (Actors and Messages
   "Address prefix", Types "Type name prefix", Watches "Watcher or target prefix") when the user moves between pages.
6. **Read only, and where it lives.** No action buttons until `inspector-actions`. Model code in `types.rs` and
   `watches.rs` with tests against an in-process server; the `appkit` module gains the two pages, `NSTableView`s sharing
   the search field, footer and pane toggle.

## Tests

Types: one read per refresh, the local name filter without a request, a failed read with Retry and no stale list,
Refresh clearing the selection and schema, the pane's pop-up formatting only the chosen schema, a stale formatting
dropped. Watches: paging on scroll by the unchanged cursor with no filter; with a filter that hides every row, no page
asked by itself, one page per Load more, Retry after a failed page with no row visible; the filter matching watcher or
target; Refresh starting again; both Show Actor cases. Each page keeping its own filter text. The window gets a manual
check list in the tasks. Tests on the paused clock keep a wall-clock bound.

## Docs

The handoff gives the app guide's Inspector section the two pages; the changelog's 0.26.0 section names them.
