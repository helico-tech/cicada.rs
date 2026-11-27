# Proposal

## Why

The Messages, Types and Watches items of the inspector's sidebar still say "Coming soon". They were part of
`inspector-actors` in the owner's proposal and split off when its design showed that all four pages did not fit five
tasks (2026-11-27).

## What Changes

- Messages: the pending messages under a prefix (`/messages`), paged as the Actors table, with the same pane for the
  selected message's actor.
- Types: the registered types (`/types`) with their contract hash, and their state and message schemas in the pane.
- Watches: the watches (`/watches`), with watcher and target.
- Read only, as `inspector-actors`.

Decided in the design: one paged-list engine shared with Actors, the panes from the rows and schemas, local filters
where the API has none, and Show Actor.

## Order

After `inspector-children`, before `inspector-outcomes`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (three tables and their panes). Tests of the mapping against an in-process server; a manual
check list. Docs handoff: the app guide.
