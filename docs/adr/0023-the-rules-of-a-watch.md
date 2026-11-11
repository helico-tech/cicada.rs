# 0023. The rules of a watch

Date: 2026-11-11

## Status

Accepted

## Context

ADR 0018 adds watches to the core: a stored pair of a target and a watcher, and a notification, `Failed` or
`Stopped`, that the store puts into the watcher's mailbox in the transaction of the event. It leaves open what the
change `watches` had to settle to build it, and ADR 0019 added a way for an actor to go away: with its parent.

A notification makes a failed message cause a send, which changes the guarantees G2 and G4. The project rule wants a
change to a guarantee recorded.

## Decision

- **A notification is a message of the watcher's own family.** The core has two public types, `Failed` and
  `Stopped`. A watcher declares both as variants of its message family, under those type names, and its one handler
  handles them. A watch is refused when the contract of the watcher's type does not hold both, with the core's
  schemas.
- **A type target is a prefix at a dot, and `*` is every type.** A target with a `/` is one actor. A target without
  one matches the type of that name and every type whose name goes on with a dot: `billing` matches `billing` and
  `billing.invoices`, not `billings`. ADR 0018 lists one actor, one type and a type prefix; `*` is the same rule
  taken to its end, and a dead-letters actor needs it.
- **A type target tells of failures only.** A watch on one actor tells of its failures and of its going away. A
  watch on a type prefix or on every type tells of failures only. Otherwise the watcher of every type would get a
  message, run a turn and keep an outcome for every actor of the system that goes away, and every removal would
  lock that one row.
- **`Stopped` says why, and who:** the cause is `stopped` (the actor's own turn), `deleted` or `parent` (removed
  with its parent, ADR 0019), and `by` is the actor whose turn removed it, absent for a removal from outside a turn.
  `Failed` holds the actor, the id, type name, payload, correlation id and sender of the failed message, the error
  and the number of attempts. Both schemas are part of every watcher's contract, so they do not change lightly; a
  snapshot test pins them.
- **A `Failed` has no `causedBy`, and no notification is retried.** A `Failed` names the failed message in its
  payload: the messages a failed message caused are its retries. Because it holds that payload, it is kept as long
  as the failure is: the `failed` rule of the failed message's retention holds for both of its outcomes. A
  notification has no retry policy; one that fails ends `failed` at once.
- **A watch from outside a turn needs a watcher that exists.** In a turn the watcher is the actor itself. The target
  is never checked: a watch survives the delete and re-create of its target.
- **The store marks a notification, and names it after its event.** A message has a field `notification` that only
  the store sets. A notification that ends `failed` notifies nobody, so two actors that watch each other cannot
  loop. The runtime draws one id per commit and per delete; the store names the notifications of that event after it
  with a number, in the order of the actor's and then the watcher's address. One event writes one notification per
  watcher and actor.
- **A watch never fails the commit of the watched actor.** A watcher that does not exist, or that the same
  transaction removes, is skipped, and its watch is removed.
- **Watches follow their watcher.** Removing an actor removes the watches it holds as a watcher. The watches on it
  as a target stay.
- **G2 and G4 name the notifications.** The notifications of a failed message, and those of an actor that goes
  away, are written with that event or not at all, and each is written once.

## Consequences

- A message that ends `failed` can now cause sends. Before, a failed message had no effects.
- A watcher of a type prefix, or of every type, gets a `Failed` for every failure it matches. There is no filter. A
  watcher of every type is in the way of every failure of the system, not of its removals.
- One event can write many messages: an actor with many watchers, or a tree whose actors are each watched by their
  address. It is one transaction.
- The schema gets an appended migration in both stores: the table of watches and the mark on a message. A node of
  the earlier version that keeps running writes no notifications, so every old node is stopped first.
- The wire format grows: the routes of `/watches`, and `notification` in the view of a message.
- `Failed` and `Stopped` are type names a watcher's family must use for these two messages. A family that had a
  variant of that name for something else renames it before it watches.
- The watched actor does not know its watchers, and a successful message notifies nobody (ADR 0018).
