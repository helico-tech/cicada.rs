# Proposal

## Why

The Running Turns and Nodes items of the inspector's sidebar still say "Coming soon". They were part of
`inspector-outcomes` in the owner's proposal and split off in its design, as all of it did not fit five tasks
(2026-11-27).

## What Changes

- Running Turns: the running turns (`/turns`, paged), with actor, message, node and lease end; Show Actor.
- Nodes: the nodes (`/nodes`), with id, status, hosted types, label, host and last heartbeat, and the running turns of
  the selected node in the pane (`/nodes/{node}`).
- Read only.

Decided in the design: a node's pane reads its turns with `/nodes/{node}`, and the last heartbeat shows as a time next
to the server's status, with no age measured on the Mac's clock.

## Order

After `inspector-outcomes`, before `inspector-live`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (two pages). Tests against an in-process server; a manual check list. Docs handoff: the app
guide.
