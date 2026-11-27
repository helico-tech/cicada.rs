# Docs handoff for inspector-messages

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The inspector's Messages item has a page. Its table lists the pending messages to the actors under the search prefix,
100 at a time as it scrolls, each message once. Selecting a message fills a read-only pane with its envelope and its
payload as highlighted JSON. The pane's Show Actor button goes to the message's actor on the Actors page and is
recorded in back and forward. No public item changed: `Inspector` and `Embedded` open the same window.

The tests in `crates/cicada-inspector/src/tests/messages.rs` cover paging, a message shown once when two pages hold
it, a page of only such messages still moving on, the pane from the row with its payload formatted only on selection,
Show Actor selecting the exact row, "not found", a failed read with Retry, cancellation, and Back and Forward. The
Actors tests in `tests/actors.rs` pass unchanged over the shared pager. The window has a manual check list in
`tasks.md`.

## 1. The Messages page and Show Actor

**Where.** `docs/guide/app.md`, `Inspector on macOS`: after the Actors description (from the `inspector-actors` and
`inspector-children` handoffs), and the sentence that says which Browse items have a page.

**Text.** Messages lists the pending messages, delayed ones included, to the actors whose address starts with the text
in the search field. Actors and Messages share the field, and each page keeps its own prefix. The table loads 100
messages at a time as you scroll and shows each message's id, target, type, delivery time and attempt. A message that
is retried while you scroll can appear on two pages of the server's answer; the table shows it once, as first read.
The footer shows how many messages are loaded, that none are pending under the prefix, or why the next page failed,
with **Retry**.

Select a message to see its envelope in the pane: id, target, sender, type, delivery time, attempt and failures,
correlation id and headers, and its payload as highlighted JSON. A payload longer than 256 KiB shows its start with a
notice. The pane shows what the table read and makes no request of its own. ⌘R reloads the list from the top and
clears the selection.

**Show Actor** opens the Actors page with the message's target address in the search field and selects that actor's
row once the first page has loaded. If the actor no longer exists, the pane says it is not found. If the page fails,
the list shows the error with **Retry**, and Retry still selects the actor. Back (⌘[) returns to Messages with its
prefix, the pages it had loaded and the message selected; Forward (⌘]) goes to the actor again. Leaving the Actors page
or typing in the search field before the page loads cancels the selection.

**Caveats.** The table is not a snapshot: messages handled or retried after a page was read stay as read until ⌘R.
The page shows no "due" mark, as the server's answer has no store time to compare with. It is read only: no cancel,
retry or deliver-now yet.

## Blocking

None.

## Status

Proposed app guide text. The inspector built and passed Clippy and its 45 model tests. Workspace formatting, Clippy
with `-D warnings`, 3,911 workspace tests with PostgreSQL, the 11 documentation tests, the 54 doctests and strict
OpenSpec validation passed. The change loop runs the gate at landing.

An uncommitted scratch script drove the real window by code against an in-process server: Messages picked in the
sidebar, 120 messages loaded while scrolling, a message selected, Show Actor selecting its actor's row with the search
field showing the address, Back restoring the message's row and the empty prefix, Forward selecting the actor again,
and ⌘R's action clearing the selection. Nobody saw the window, so these count as run by code, not seen. The whole
manual check list, light and dark, a gone actor, a stopped server, keyboard and VoiceOver included, still needs the
owner's pass before 0.26.0 is released.
