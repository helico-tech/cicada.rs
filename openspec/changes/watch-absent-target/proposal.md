# Proposal

## Why

A turn cannot learn that the actor it watches is already gone (Factory issue 0014, a real bug, request from the
Factory session, 2026-11-25). Factory's Pool gives a place to a Run and watches it; when the Run is deleted before the
Pool's turn commits, the watch lands on an absent target, no `Stopped` ever comes, and the place stays taken for good.
`turn.watch` documents "the target need not exist", which is right for a watch set before a create, so the plain watch
cannot simply change.

## What Changes

Some way for a turn to learn that a watched instance is absent. Options for the design:

- a watch variant (such as `turn.watch_existing(target)`) that puts a `Stopped` into the watcher's mailbox at once
  when the instance is absent at the commit (Factory's preference);
- a send to a missing instance that tells the sender, with a notice like `Stopped` or `Failed`;
- a cheap "exists" read inside a turn, which still races the commit.

To decide in the design: the variant's name and target kinds (an instance only; a type prefix or every type has no
"absent"), how the commit checks existence atomically with setting the watch on both backends, the remote turn routes
and the HTTP API if a watch crosses them, and whether it needs an ADR (it adds a notification path: likely yes).

## Out of scope

Changing `turn.watch` itself. Notices for sends to missing instances, unless the design picks that option.

## Order

After `docker-context`, before `container-job-max-turn`.

## Impact

`crates/cicada` (turn, watch, commit on SQLite and PostgreSQL, the conformance suite), perhaps the remote turn wire
model and `cicada-client`. Additive (a new method): a patch.
