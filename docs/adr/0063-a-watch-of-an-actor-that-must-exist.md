# 0063. A watch of an actor that must exist

Date: 2026-11-25

## Status

Accepted for watch-absent-target. Builds on ADR 0023 and ADR 0024.

## Context

A watch's target is never checked (ADR 0023), so a watch can be set before its target is created. A turn that needs
to learn when an actor goes away, and is not sure it is still there, cannot tell: when the actor went before the
commit, the watch lands on nothing and no `Stopped` comes. Factory's pool keeps a place for such a run for good
(Factory issue 0014). An `exists` read inside the turn does not help: a delete between the read and the commit writes
no `Stopped`, as the watch is not set yet.

## Decision

- `Turn::watch_existing(target)` takes one actor. At the commit, where watches are applied (ADR 0024), it sets a watch
  when the actor exists and otherwise puts one `Stopped` into the watcher's mailbox in the same transaction, and sets
  no watch. A type prefix or `*` is refused in the turn. `Turn::watch` is unchanged.
- That `Stopped` has the cause `deleted` and `by` null, as for a delete from outside a turn, so the schema of
  `Stopped`, part of every watcher's contract, does not change. `deleted` with `by` null therefore also means "not
  there when the watch was set".
- It is a notification of the commit's event: marked as one, named after the event id with its number, with the
  turn's message as `causedBy`. One event writes one per watcher and actor, so two calls give one.
- Absent leaves the pair without a watch, an older stored one included. Watch changes apply in the order of the
  calls, each `watch_existing` checked at its place.
- The check sees every delete by another transaction either before it or after the watch is stored: on PostgreSQL the
  commit locks the target's row (`FOR UPDATE`, in its ordered lock set) and "present" means the lock found it.
- The remote turn report carries `"existing": true` on such a watch change, left out otherwise; with a type target or
  `set: false` it is refused. `WatchChange` gains the field `existing`, a break of a public struct: 0.22.0.

## Consequences

A watcher learns of an actor's end exactly once whether the actor goes before or after its watch: the check and the
watch are one transaction. An actor created in the same turn is not there yet at the check; such a turn uses `watch`.
A watcher cannot tell "never there" from "deleted from outside" by the notification alone. The HTTP API and the
portable lease fixtures gain one optional field; servers are upgraded before workers, as an older server refuses it.
No schema migration and no contract hash changes.
