# Docs handoff for inspector-types-watches

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The macOS inspector has read-only Types and Watches pages. The Types pane displays a chosen schema as highlighted
JSON. The Watches pane shows the watcher and target and links to the Actors page for either endpoint that is an
actor. `Inspector` and `Embedded` open the same window.

The in-process server tests in `crates/cicada-inspector/src/tests/types.rs` and `tests/watches.rs` cover local
filters, reloads and context changes, errors and Retry, paging with and without a filter, on-demand schema formatting
and stale results, and Show Actor with its history.

## 1. The Types page

**Where.** `docs/guide/app.md`, `Inspector on macOS`. Add after Messages and update the list of implemented Browse
items.

**Text.** Types lists the registered types by name. The table shows the shortened contract hash with the full hash
in its tooltip, and the number of message types. The search field filters the loaded list by type name prefix. The
first visit and each ⌘R read the whole list. Select a type and use the pane's pop-up to see its State schema or a
message schema. The message choices are in name order. The pane formats the selected schema as highlighted JSON and
shows a notice when it is cut at 256 KiB.

**Caveats.** ⌘R clears the type and schema selection. A failed read leaves the list empty and shows the error with
**Retry**. After a context switch, the next visit reads the list from the new context.

## 2. The Watches page and shared search

**Where.** `docs/guide/app.md`, `Inspector on macOS`. Add after Types and update the description of the search field
and pane toggle.

**Text.** Watches lists the target and watcher, ordered by target and then watcher, 100 rows at a time. Scroll to
load more when the search field is empty. A prefix matches either the watcher or the target and covers the rows
already loaded. While the field has text, **Load more** requests the next page, even when no row matches. **Retry**
requests a failed page again. Select a watch to see its target and watcher in the pane. **Show Actor** goes to the
watcher's exact root row on the Actors page. A target that names one actor has its own **Show Actor** button. Back
returns to the watch and filter, and Forward repeats the jump. A missing actor shows “not found” after a successful
read, and a failed read shows **Retry** on Actors.

Actors, Messages, Types and Watches share the search field and the pane toggle. Each page keeps its own filter text.
The search field's placeholder says which prefix to enter.

**Caveats.** Watches uses the server's cursor and keeps the rows as they were read until ⌘R. Refresh clears the watch
selection and starts from the first page. A context switch drops the pages and selection, and the next visit starts
from the first page of the new context. The pane shows no watch actions other than navigation.

## Blocking

None.

## Status

Proposed app guide text. The macOS inspector build, crate Clippy, 54 inspector unit tests and one doctest passed.
A temporary AppKit driver exercised the Types table, name filter, State and message schemas in both appearances,
watch paging by scroll and **Load more**, both kinds of watch filter, both kinds of **Show Actor** with back and
forward, the four search fields' saved text, the Refresh action, stopped-server errors and Down-arrow selection.
These checks ran by code and were not seen. The ⌘R key equivalent and VoiceOver still need a manual pass.
