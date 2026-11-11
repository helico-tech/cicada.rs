# Design

Written on 2026-11-11, against the code before steps 1 to 4 of ADR 0018. This change comes after them (proposal.md,
Order): check the names below against the code before starting. A store that is gone by then needs no work, and the
migration takes the next free version.

## Context

See proposal.md, Why, and ADR 0019.

- `turn.create` buffers an `ActorCreate { address, state }`; the commit applies creates before sends, idempotently.
- `Store::delete` and a commit with `StateChange::Stop` remove one row and purge its mailbox. A turn that runs on a
  deleted actor gets `LeaseLost` at commit.
- The stores differ in how they serialise writes: the in-memory store has one lock, SQLite one writer, PostgreSQL row
  locks at `READ COMMITTED` with a re-run on deadlock. Clock readings in PostgreSQL come after all row locks.
- A rejected commit must not have written anything (S2). The in-memory and SQLite stores check before they write;
  PostgreSQL rolls back.

## Goals / Non-Goals

**Goals:**

- A child cannot outlive its parent, whoever removes the parent.
- The same behaviour in all three stores, pinned by the conformance suite.
- No new concept beyond "parent" in the public API.

**Non-Goals:**

- See proposal.md, Out of scope.

## Decisions

### 1. `ActorCreate.child: bool`, not a parent address

A child is always a child of the committing actor, so the store derives the parent from `CommitRequest.address`. A
free parent address would need a check that the parent exists and, on PostgreSQL, a lock on it. With a flag the
invariant "a child's parent exists" follows from the lease: the committing actor holds one, so it exists and cannot
be removed until the commit ends.

`Store::create` (the create from outside a turn) keeps its signature and creates independent actors.

### 2. A child create that meets a foreign actor rejects the commit

| The address at commit | `create` | `create_child` |
| --- | --- | --- |
| absent | created, no parent | created, parent = committing actor |
| exists, parent = committing actor | untouched | untouched |
| exists, another parent or none | untouched | `CommitResult::Rejected`, the message ends `failed` |

"Left untouched" for the last cell would be less code, but then the creator believes it owns an actor that its
removal will leave behind: the gap this change closes. A loud failure fits the rule for sends to an absent actor.

Two creates of one address in one commit that disagree on `child` are a malformed request: `StoreError::Invalid`
from the store (one shared check in `store::support`), and `TurnError::Invalid` from the second `turn.create*` call,
so a handler sees it at once. This keeps the store check free of "what would the first create have done".

### 3. Removal walks the tree in the store

`delete` and `Stop` collect the descendants and then, for each, purge the mailbox and delete the row, in the
transaction that removes the root. `DeleteResult::Deleted.purged` is the sum over the tree; the enum does not change.

- **In memory:** a loop over the rows under the store lock.
- **SQLite:** one recursive query on `actors.parent`; the single writer makes it a consistent snapshot.
- **PostgreSQL:** level by level, `SELECT address FROM actors WHERE parent = ANY ($1) ORDER BY address FOR UPDATE`
  until a level is empty. Each statement gets a new snapshot. A child is created only in a commit that holds its
  parent's row lock, so once a level is locked no commit can add a child below it, and every child committed earlier
  is visible to the next statement. One recursive query would use one snapshot and could miss a child added by a
  commit it had to wait for. All locks are taken before the clock is read, as in every other write. The locks are not
  in address order, so a removal can deadlock with a commit that sends up or down the tree; the existing re-run of
  aborted transactions (8 times) covers that.

In a commit with `Stop`, the descendants are locked after the creates and before the validation, so children created
in the same commit are part of the tree and their messages are purged with it.

### 4. Listing children is a route below the actor

`GET /actors/{type}/{key}/children?cursor=&limit=` answers a page of addresses, or `404 no-such-instance`, like
`/mailbox`. The alternative, `GET /actors?parent=`, would put an address into a query value and raise the question of
how it combines with `prefix`. `Store::children(parent, cursor, limit)` serves it from the index
`actors_parent (parent, address)`, which also serves the removal.

`ActorView.parent` is always written, `null` for an independent actor, like every other response field.

### 5. Schema: an appended migration in both stores

The next version: `ALTER TABLE actors ADD COLUMN parent` (text, nullable; `COLLATE "C"` on PostgreSQL) and
`CREATE INDEX actors_parent ON actors (parent, address) WHERE parent IS NOT NULL`. No foreign key: the store removes
the tree itself, because every removed mailbox needs its `purged` outcomes (G3), which a database cascade cannot
write.

## Risks / Trade-offs

- A large tree is a large transaction → documented in `docs/operations.md`; a tree that large removes itself
  bottom-up in its own code.
- A delete is now stronger than before → documented; only actors created with `create_child` are affected.
- More deadlock re-runs on PostgreSQL when a tree is removed while its actors are busy → bounded by the existing
  retry; a removal that still fails reports `Unavailable` and can be repeated.

## Migration Plan

Stop all nodes of the previous version, start the new ones; the first one migrates the store. Rolling back the code
needs a database from before the migration, as for every schema version.
