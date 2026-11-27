# Proposal

## Why

The Messages item of the inspector's sidebar still says "Coming soon". It was part of `inspector-actors` in the owner's
proposal, then of `inspector-tables`, whose design review split it again: all three pages did not fit five tasks
(2026-11-27).

## What Changes

- Messages: the pending messages under a prefix (`/messages`), paged as the Actors table, each once, with a pane for
  the selected message (its envelope and payload).
- Show Actor: from a message to its actor on the Actors page, recorded in back and forward.
- The Actors list's root paging is factored into a pager the two pages share.
- Read only, as `inspector-actors`.

## Order

After `inspector-children`, before `inspector-types-watches`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the pager, the Messages page and Show Actor). Tests against an in-process server; a manual
check list. Docs handoff: the app guide.
