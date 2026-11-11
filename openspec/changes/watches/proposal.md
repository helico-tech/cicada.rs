# Proposal

## Why

One actor cannot learn that another one failed or went away, except by polling outcomes, as `FailureWatcher` in
`cicada-extras` does on a timer. Supervision and dead letters need that signal in the same transaction as the event.
ADR 0018 adds watches to the core.

## What Changes

- A watch is a stored pair: a target and a watcher. The target is one actor, one type, or a dotted type prefix at a
  segment boundary (`billing` matches `billing.invoices`). The watcher is one actor.
- A watch is set and removed as a turn effect (`watch`, `unwatch`), and from outside a turn through `ActorSystem` and
  HTTP.
- When a message to a watched actor ends `failed`, or a watched actor stops or is deleted, the same transaction puts
  one notification into each watcher's mailbox. The notification is a core message with a fixed type name and schema:
  `Failed` (message id, error, attempts) or `Stopped`.
- An actor that is removed together with its parent (ADR 0019) counts as stopped and notifies its watchers too.
- A watcher that does not exist is skipped and its watch removed. It never fails the commit of the watched actor.
- A watch survives delete and re-create of its target. Deleting a watcher removes its watches.
- A notification that itself fails notifies nobody, so two actors that watch each other cannot loop. A successful
  message notifies nobody. The watched actor does not know its watchers.
- An appended migration adds the table of watches, in both stores.
- A failed message can now cause sends. G2 and G4 get a row for that in `docs/architecture.md`.

The simplest version: two notifications, no filter on them.

To decide in the design:

- How a watcher's message family takes a notification.
- Whether a watcher that is removed in the same tree as its target still gets a notification.
- The HTTP routes that list, set and remove watches.

## Capabilities

### New Capabilities

- `watches`: an actor is told when a message to a watched actor fails, and when a watched actor stops or is deleted.

### Modified Capabilities

None.

## Out of scope

- A watch that covers only the messages its watcher sent (ADR 0018: not built until needed).
- Crash loops. They produce no `failed` outcome; `turn-observer` gives them one.
- Notifications for `done`, `cancelled` or `purged` messages.
- The supervisor and dead letters built on watches; that is `cicada-actors`.

## Order

Step 5 of ADR 0018. After `child-actors`, before `control-effects`. It comes after `child-actors` so that the removal
of a tree is covered from the start, not added later.

## Impact

- `crates/cicada` (`store`, `turn`, `runtime`, `api`), both stores and their migrations, the store conformance suite,
  `cicada-axum`, `cicada-client`, `cicada-cli`, `README.md`, `docs/architecture.md` (G2, G4, the domain language, the
  schema), `docs/openapi.json` and `CHANGELOG.md`.
- A guarantee and the wire format change; ADR 0018 covers both.
- Nodes of the previous version refuse the migrated database, so all nodes are upgraded together.
