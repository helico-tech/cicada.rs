# Docs handoff for inspector-actors

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The inspector's Actors item has a page. No public item changed: `Inspector` and `Embedded` open the same window, which
now shows the Actors page and an inspector pane.

- The table lists the actors whose address starts with the text of the toolbar's search field, 100 at a time, by the
  server's cursor. The next page is read when the last visible row comes within 20 of the end, also after rows came in
  or the window was resized. Columns: address, type, version, created (UTC) and status.
- Status: `running` when the actor is among one read of the first 200 running turns under the prefix, taken once per
  reload of the list (a new prefix, Refresh, a context switch), not per page; `paused` from the actor; `running ·
  paused` when both hold. The footer says "The running status covers the first 200 running turns." when more turns
  run, or "The running status is unknown: …" when that read failed; then no row is marked running.
- A page that fails shows a row with its error and a Retry button; scrolling does not ask again.
- Typing in the search field empties the table at the first key press and reads the new prefix 300 ms after the last
  key.
- Selecting an actor fills the pane, read only: the address; type and version; created; parent ("none
  (independent)"); pinned node ("any node"); "Paused" when its mailbox is paused; its running turn ("Running <type> ·
  attempt <n>" when the turn's message is among the first 50 messages of the mailbox, then message id, node, lease and
  "cancel requested"), or "Idle: no turn is running." Tabs: State (the state as pretty JSON, colour-highlighted in light
  and dark, cut at 256 KiB with the notice "The state is longer than 256 KiB; this shows its first 256 KiB."), Mailbox
  (the pending messages among the first 50, with type, id, due time and attempt; the running one is matched only for
  the turn above) and Watches
  ("Watching", the targets it watches; "Watched by", the watchers whose stored target is exactly this actor).
- An actor that is gone shows "Not found: this actor no longer exists." and nothing else; each other part shows its own
  error. Each read has 10 s.
- When the mailbox cannot be read, the turn shows "Running (its message type is unknown: the mailbox could not be
  read)" with its message id, and the Mailbox tab shows the error.
- A search whose prefix leaves out the selected actor unselects it: the pane empties and its reads are dropped.
- The toolbar gains the search field and a "Show or Hide Pane" button; in the standalone app only
  (`Inspector::run`), the View menu gains Hide Pane / Show Pane; an embedded window has no such menu item. The
  pane shows only on the Actors page. ⌘R on the Actors page reloads the list from the top and the pane; on another
  page it refreshes the Overview as before.
- A read that no longer matches the current prefix, selection or context is dropped (also when the user selects A, then
  B, then A again); a context switch or a closed window cancels the pending reads.

Proved by the tests in `crates/cicada-inspector/src/tests/actors.rs` (paging, the server's cursor, one page at a time,
Retry, the sample and its footers, the search, cancellation at close and at a context switch, the pane's parts, a
missing actor, one part failing, A then B then A) and `crates/cicada-inspector/src/tests/json.rs` (the formatter). The
window has a manual check list in `tasks.md`.

## 1. The Actors page in the app guide

**Where.** `docs/guide/app.md`, `Inspector on macOS`: replace "only the Overview has a page so far, and the others say
"Coming soon"" and add a paragraph after the Overview's.

**Text.** The sidebar lists Overview and the Browse items (Actors, Messages, Running Turns, Outcomes, Nodes, Types,
Watches); Overview and Actors have pages, and the others say "Coming soon".

Actors lists the actors whose address starts with the text in the toolbar's search field, 100 at a time as the table
scrolls, with their type, version, created time and status. Typing empties the table at once and reads the new prefix
when you stop typing. The status says `running` when the actor is among the first 200 running turns under the prefix,
read once per reload, and `paused` when its mailbox is paused, or both; the footer says when the running status covers
only the first 200 running turns, or that it is unknown. A page that fails shows its error with a Retry button.
Selecting an actor fills the pane on the right: its type, version, created time, parent, pinned node and paused flag,
its running turn, and tabs for its state as highlighted JSON, its mailbox (the pending messages among its first 50)
and its watches. Everything is read only. The toolbar's pane button hides and shows the pane (in the standalone app also
View › Hide Pane); ⌘R
reloads the list from the top and the pane.

**Caveats.**

- The running status is a sample: with more than 200 running turns under the prefix, an actor past them shows no
  `running` even when it runs. Search a narrower prefix to see it.
- The turn's message type and attempt come from a separate read of the mailbox: a turn reclaimed between the two reads
  can show an older attempt, and a message past the mailbox's first 50 shows no type.
- Watches lists only watches whose stored target is exactly this actor: a watch on its type, or on every type (`*`),
  is not listed (the API lists exact targets only).
- A state longer than 256 KiB shows its first 256 KiB with a notice; the pane never shows more.
- Pages are not a snapshot: an actor created or deleted while you scroll can be missed or appear late; Refresh reads
  again from the top.

## Blocking

None.

## Status

Proposed text; the app guide is not edited. Ran for task 5: `cargo fmt --all --check` (clean), `cargo clippy
--workspace --all-targets -- -D warnings` (clean), `cargo test --workspace` with PostgreSQL (3,898 passed, 0 failed),
`cargo test -p cicada-examples --test docs` (11 passed), `--doc` (54 passed), `--test skill` (18 passed), and
`openspec validate inspector-actors --strict` (valid). The manual AppKit check list was not run: the builder had no
GUI session.
