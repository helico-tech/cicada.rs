# Proposal

## Why

The node table shows a node as dead once its heartbeat is 2 minutes old, but nothing tells anyone. An operator has to
look. With `pinned-actors`, a dead node also means that the actors pinned to it stop: their messages wait until the
node comes back. Hosts need to hear about it, the way watches tell a watcher that an actor failed or stopped.

## What Changes

- **A notification when a node goes down.** When a node's row turns dead, one message is sent to each configured
  target, once per death. It names the node, its details from `node-details`, and its last heartbeat.
- **Optionally a second message when it comes back**, so that a watcher can clear the alarm.
- Exactly one alive node sends the notification: the store decides which, so that two nodes that both see the dead
  row do not both send it.

To decide in the design:

- Where targets are configured: in `NodeConfig` (the same on every node), in a stored registration (like a watch,
  set over the API), or a ready-made actor in `cicada-actors` that hosts subscribe to.
- Whether "comes back" is part of the first version.
- What counts as down: the existing "dead" threshold, or a setting.
- What happens when the whole cluster restarts at once and many rows are old: one alarm per row, or none for rows of
  nodes that stopped cleanly (a clean stop already removes its row).
- How it relates to `heartbeat-watch`: a node could beat a heartbeat watch itself, which needs no core change but
  needs one watch per node.

## Out of scope

Alerts on suspect nodes. Alerts on workers, which have no node row. Paging or e-mail: the target is an actor, and the
host decides what it does. Moving the actors of a dead node.

## Order

After `pinned-process-example`. Before `sample-worker-mode`.

## Impact

Depends on the design: a store operation and a new message contract in `crates/cicada` with a conformance case, or a
new actor in `cicada-actors` with its contract in `docs/reference/contracts/`. A new notification is a new wire
format, so it needs an ADR. Handoffs for `docs/operations.md` and the guide of the layer it lands in.
