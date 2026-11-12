# Proposal

## Why

One actor cannot learn that another one failed or went away, except by polling outcomes, as `FailureWatcher` in
`cicada-extras` does on a timer. Supervision and dead letters need that signal in the same transaction as the event.
ADR 0018 adds watches to the core.

## What Changes

- A watch is a stored pair: a target and a watcher. The target is one actor, the types under a dotted prefix
  (`billing` matches `billing` and `billing.invoices`), or every type (`*`). The watcher is one actor.
- A watch on one actor tells of its failures and of its going away. A watch on a type prefix or on every type tells
  of failures only.
- A watch is set and removed as a turn effect (`watch`, `unwatch`), and from outside a turn through `ActorSystem`,
  HTTP (`/watches`) and the command line. From outside, the watcher must exist.
- **BREAKING** When a message to a watched actor ends `failed`, or a watched actor stops or is deleted, the same
  transaction puts one notification into each watcher's mailbox. A failed message can now cause sends, which changes
  the guarantees G2 and G4.
- A notification is a core message with a fixed type name and schema: `Failed` (the actor, the id, type, payload,
  correlation id and sender of the message, the error, the attempts) or `Stopped` (the actor, the cause: stopped, deleted,
  or removed with its parent, and the actor whose turn removed it). A watcher declares both as variants of its own
  message family, with the core's types; a watch is refused otherwise.
- An actor that is removed together with its parent (ADR 0019) counts as stopped and notifies its watchers too. A
  watcher that is removed in the same transaction gets nothing.
- A watcher that does not exist is skipped and its watch removed. It never fails the commit of the watched actor.
- A watch survives delete and re-create of its target. Removing a watcher removes its watches.
- A notification is marked by the store, and one that fails notifies nobody, so two actors that watch each other
  cannot loop. A successful message notifies nobody. The watched actor does not know its watchers.
- **BREAKING** An appended migration adds the table of watches and a mark on a message, in both stores. A node of
  the previous version that opens the migrated database refuses it. One that is still running would write no
  notifications, so every old node is stopped before the first new one starts.
- **BREAKING** A message and its view have the field `notification`, which only the store sets.
- The terminal UI lists the watches (`:watches`).
- A conformance case that failed once on a slow machine, in the release workflow of 0.7.0, no longer depends on a
  lease of 0.8 seconds holding (`design.md`, decision 14). It is a fix of a test, found on the way.

The simplest version: two notifications, no filter on them.

## Capabilities

### New Capabilities

- `watches`: an actor is told when a message to a watched actor fails, and when a watched actor stops or is deleted.

### Modified Capabilities

- `randomized-testing`: the simulation under faults has a watcher and checks that a notification is written once;
  the comparison of the stores sets watches.

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
  the randomised comparison of the stores, `cicada-axum`, `cicada-client`, `cicada-cli`, `README.md`, `docs/architecture.md` (G2, G4, S2, the rows on effects and on the commit, the table of
  how a turn ends, the domain language, the schema), `docs/operations.md` (the upgrade, the cost of a watcher of every type), `docs/openapi.json` and
  `CHANGELOG.md`.
- A guarantee and the wire format change; ADR 0018 covers both. The rules this design adds (the target for every
  type, the cause in `Stopped`) are in ADR 0023 (`design.md`, decision 11).
- All nodes are upgraded together: the old ones are stopped first.
