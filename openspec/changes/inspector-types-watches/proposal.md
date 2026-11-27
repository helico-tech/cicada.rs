# Proposal

## Why

The Types and Watches items of the inspector's sidebar still say "Coming soon". They were split from `inspector-tables`
(now `inspector-messages`) by its design review (2026-11-27).

## What Changes

- Types: the registered types (`/types`, answered whole) with name and contract hash, filtered by name from the cached
  list without another request; the pane formats the chosen schema (the state's, or one message's) only when it is
  shown.
- Watches: the watches (`/watches`, paged), with watcher and target; a local filter by watcher, with Load more and Retry
  still offered when the filter leaves no row visible; Show Actor for the watcher and, for a one-actor target, for it.
- Read only.

To decide in the design: how the pane picks among a type's schemas, and the filter's footer.

## Order

After `inspector-messages`, before `inspector-outcomes`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (two pages on the shared pager). Tests against an in-process server; a manual check list.
Docs handoff: the app guide.
