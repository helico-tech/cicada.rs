# Design

A turn that watches one actor cannot learn that the actor is already gone: the watch lands on nothing, and no `Stopped`
ever comes (Factory issue 0014: a pool place stays taken for good). A new turn method sets the watch when the actor is
there at the commit, and otherwise puts a `Stopped` into the watcher's own mailbox in that commit.

```rust
// The pool gives a place to a run and must learn when the run goes, also when it went before this commit.
turn.watch_existing(run.address())?;     // run there at commit: a watch; else: a Stopped { actor, deleted, by: null }
```

## Decisions

1. **A new method; `watch` stays.** `Turn::watch_existing(target)` takes one actor (a typed ref, an address, or the
   text of one); a type prefix or `*` fails with `TurnError::Invalid`, as a type has no "absent". The same checks as
   `watch` (the family declares `Failed` and `Stopped`). `watch` keeps "the target need not exist", which a watch set
   before a create needs. Not taken: a send that tells its sender about a missing target (every send would change);
   an `exists` read in the turn (it races the commit: a delete between the read and the commit writes no `Stopped`,
   as the watch is not set yet).
2. **The `Stopped` reuses the cause `deleted` with `by` null.** The watcher gets `Stopped { actor, cause: deleted,
   by: null }`, the notification of a delete from outside a turn, so the schema of `Stopped`, which is part of every
   watcher's contract (ADR 0023), stays. Not taken: a new cause `absent` (it changes the contract hash of every
   watcher type, the ready-made ones included: a breaking 0.22.0 and "Changing a contract" steps for every watcher,
   for a difference Factory does not need). The rustdoc says that `deleted` with `by` null also means "not there when
   the watch was set", and that the `Stopped` names the absent actor.
3. **Absent: the pair holds no watch.** The watcher learned the actor is gone; a watch on nothing is the bug. A stored
   watch of the same watcher and target from an earlier turn is removed too, so the pair holds no watch after the
   change. A later re-create of that address is not watched. Present: a plain watch, with every rule of ADR 0023 (it
   survives a delete and re-create). In one turn, the watch changes apply in the order of the calls, as today, and a
   `watch_existing` is checked at its place: `watch_existing` then `unwatch` leaves no watch; `watch` then an absent
   `watch_existing` gives a `Stopped` and no watch; two `watch_existing` of one absent target give one `Stopped`, as one
   event writes one notification per watcher and actor (ADR 0023) (design review, blocking 3).
4. **Checked by the commit, at the place of watches.** The commit applies watches first (ADR 0024: watches, deletes,
   creates, purges, pauses and resumes, then sends), and checks the target inside the same transaction there. So a
   delete in the same turn finds the watch set and writes its own `Stopped`, as for `watch`; a create in the same turn
   comes after the check, so `watch_existing` of an actor the turn creates gives a `Stopped`: the rustdoc says to use
   `watch` there. A watch of the watcher itself finds it present. **Never neither:** the check must see a delete by
   another node either before it (a `Stopped` from the check) or after the watch is stored (a `Stopped` from the
   delete). On PostgreSQL at `READ COMMITTED` a plain read does not give that: a delete locks only the watchers it
   found, so it can commit between the check and the insert. So the commit adds each `watch_existing` target to its
   ordered row-lock set (`FOR UPDATE`, in address order with the rows it locks already), and "present" means that lock
   found the target's row, taken before the rows of the turn's creates exist. SQLite's commit holds the write lock for
   the whole transaction; the in-memory store holds its lock. A PostgreSQL test races a delete against the commit
   (design review, blocking 2).
5. **The notification is one of this commit's event.** It is marked `notification` (one that fails notifies nobody),
   is named after the commit's event id with its number among that event's notifications (ADR 0023), and wakes the
   watcher like any notification. Its `causedBy` is the turn's message.
6. **On the wire, a field `existing`; a breaking 0.22.0.** `WatchChange` gets `existing: bool`; the remote turn
   report's `WatchChangeView` carries `"existing": true`, left out when false, so every existing body and fixture
   stays as it is. Both structs are public and exhaustive, so code that builds them field by field stops compiling:
   the batch becomes 0.22.0, named **BREAKING** in the changelog, as 0.20.0 was for the same kind of change (design
   review, blocking 1). Decision 2 stands on its own reason (every watcher's contract hash). A body with
   `existing: true` on a type target or with `set: false` is refused as an invalid report (400) where the view becomes
   a `WatchChange`, which can fail from now on; a refused-body fixture pins it. A server that predates the field
   refuses it as an unknown field, so servers are upgraded before workers; the handoff says so.
   `docs/openapi.json` is regenerated; `fixtures/leases.json` gains one body with `existing` (ADR 0047). The
   in-memory store, SQLite and PostgreSQL implement the check; the conformance suite covers it.
7. **An ADR.** A new way to get a notification, and a change of the turn report: ADR 0063, "A watch of an actor that
   must exist", next to ADR 0023, which is not edited.

## Tests

- Conformance (`crates/cicada/src/tests/conformance`), every backend: present target, a watch is set and a later
  delete writes one `Stopped`; absent target, one `Stopped` (deleted, `by` null, `causedBy` the turn's message, named
  after the event) and no watch, an older stored watch of the pair removed; a type prefix refused in the turn; delete
  in the same turn; create in the same turn gives a `Stopped`; the watcher itself; the orders of decision 3 (with
  `unwatch`, after `watch`, twice); the notification ids with another notification of the same commit.
- PostgreSQL: a delete of the target racing the commit of a `watch_existing` gives exactly one `Stopped`, many rounds.
- Runtime, on virtual time with `TestActors`: Factory's case, a run deleted between the send that gives it a place and
  the pool's turn, frees the place.
- Wire: the report with `existing` round-trips; a body without it reads as false; `existing` on a type target or with
  `set: false` is refused; the new fixtures, one accepted and one refused.
- Remote worker: a turn with `watch_existing` on a worker over HTTP gives the same result (an existing remote-turn test
  extended).

## Docs

Nothing breaks: no doc block builds a `WatchChange`. The handoff asks for `watch_existing` next to `watch` in the
guide, the Watch and Notification entries of `docs/architecture.md`, the wording of G2 and the new conformance test
for the row of G4 in `docs/guarantees.md`, the upgrade order (servers before workers) and the 0.22.0 break. The
rustdoc of `Turn::watch` points to `watch_existing`.
