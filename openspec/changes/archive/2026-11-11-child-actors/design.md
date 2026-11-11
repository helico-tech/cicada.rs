# Design

Written on 2026-11-11, before steps 1 to 4 of ADR 0018, and gone through with the owner that day: the five decisions
below stand ("Agree with all"). Brought up to date on 2026-11-11 with the code of 0.6.0 and the design of
`tidy-the-core`, which is built before this change. Decisions 6 to 9, and what is marked "first review", were added
then by the main session; the owner left the open points of this change to it ("you have my approval to decide for
me"). Reviewed once.

## Context

See proposal.md, Why, and ADR 0019. What the code does today (0.6.0):

- The store trait is private to `cicada` and is named `Backend` (ADR 0018, step 2). The SQLite and PostgreSQL
  stores are in `crates/cicada/src/store`, the in-memory store in `crates/cicada/src/testing/memory.rs`, the
  conformance suite in `crates/cicada/src/tests/conformance`, the randomised comparison of the stores in
  `crates/cicada/src/tests/fuzz`.
- `turn.create` buffers an `ActorCreate { address, state }`; the commit applies creates before sends, idempotently.
- `Backend::delete` and a commit with `StateChange::Stop` remove one row and purge its mailbox. A turn that runs on
  a deleted actor gets `LeaseLost` at commit.
- The stores differ in how they serialise writes: the in-memory store has one lock, SQLite one writer, PostgreSQL row
  locks at `READ COMMITTED` with a re-run on deadlock (8 times). Clock readings in PostgreSQL come after all row
  locks.
- A rejected commit must not have written anything (S2). The in-memory and SQLite stores check before they write;
  PostgreSQL rolls back.
- A node checks the schema version only when it opens its store. A node that is already running does not notice a
  later migration.
- A PostgreSQL transaction is cut off after `socket_timeout`, 30 seconds unless the host sets another value; the
  call then answers `Unavailable`.
- Both SQL stores are at schema version 3. `ActorRecord` (store), `ActorSnapshot` (runtime) and `ActorView` (wire)
  are public types that hold an actor's address, state, version, creation time and whether it is paused.
- The branch `feat/child-actors` holds one unfinished commit (a7fb226) that edits the stores where they were before
  step 2.

## Goals / Non-Goals

**Goals:**

- A child cannot outlive its parent, whoever removes the parent.
- The same behaviour in all three stores, pinned by the conformance suite and by the randomised comparison.
- No new concept beyond "parent" in the public API.

**Non-Goals:**

- See proposal.md, Out of scope.

## Decisions

### 1. `ActorCreate.child: bool`, not a parent address

A child is always a child of the committing actor, so the store derives the parent from `CommitRequest.address`. A
free parent address would need a check that the parent exists and, on PostgreSQL, a lock on it. With a flag the
invariant "a child's parent exists" follows from the lease: the committing actor holds one, so it exists and cannot
be removed until the commit ends.

`Backend::create` (the create from outside a turn) keeps its signature and creates independent actors.

### 2. A child create that meets a foreign actor rejects the commit

| The address at commit | `create` | `create_child` |
| --- | --- | --- |
| absent | created, no parent | created, parent = committing actor |
| exists, parent = committing actor | untouched | untouched |
| exists, another parent or none | untouched | `CommitResult::Rejected`, the message ends `failed` |

"Left untouched" for the last cell would be less code, but then the creator believes it owns an actor that its
removal will leave behind: the gap this change closes. A loud failure fits the rule for sends to an absent actor.

An actor that names itself in `create_child` falls under the last row: it exists and is not its own child.

Two creates of one address in one commit that disagree on `child` are a malformed request: `StoreError::Invalid`
from the store (one shared check in `store::support`), and `TurnError::Invalid` from the second `turn.create*` call,
so a handler sees it at once. This keeps the store check free of "what would the first create have done". The check
is the first thing a commit does, before the store is read, so the answer does not depend on the lease (first
review).

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
  aborted transactions covers that.

In a commit with `Stop`, the descendants are locked after the creates and before the validation, so children created
in the same commit are part of the tree and their messages are purged with it.

Every walk visits an actor once, so it ends on any data (first review). Through this version no cycle can form. But
a node of 0.7.0 that keeps running on a migrated database deletes a parent without its children, and such an orphan
can then create its old parent's address as its own child: two rows that name each other. The recursive query of
SQLite uses `UNION`, not `UNION ALL`; a level of PostgreSQL leaves out the addresses that are locked already; the
in-memory store keeps a set of what it has seen.

### 4. Listing children is a route below the actor

`GET /actors/{type}/{key}/children?cursor=&limit=` answers a page of addresses, or `404 no-such-instance`, like
`/mailbox`. The alternative, `GET /actors?parent=`, would put an address into a query value and raise the question of
how it combines with `prefix`. `Backend::children(parent, cursor, limit)` serves it from the index
`actors_parent (parent, address)`, which also serves the removal. `ActorSystem::list_children` is the public call.

`ActorView.parent` is always written, `null` for an independent actor, like every other response field.

### 5. Schema: migration 4 in both stores

`ALTER TABLE actors ADD COLUMN parent` (text, nullable; `COLLATE "C"` on PostgreSQL) and
`CREATE INDEX actors_parent ON actors (parent, address) WHERE parent IS NOT NULL`. No foreign key: the store removes
the tree itself, because every removed mailbox needs its `purged` outcomes (G3), which a database cascade cannot
write.

The migration is not additive (first review). A node of 0.7.0 that opens a database of version 4 refuses it, as
every node refuses a newer schema. But one that is already running does not notice, and its delete and stop remove
one row and leave the children behind. So every node of the old version is stopped before the first new one starts.
The docs of both schema modules say so, and `docs/operations.md` names 0.8.0 as such a release.

### 6. The randomised comparison of the stores creates children

Decided by the main session.

The removal of a tree is written three times, once per store, and each walks the tree in its own way. That is where
stores drift apart. The comparison of `tests/fuzz` already runs random commits with creates on the in-memory store
and on SQLite. It gets three additions: a create in a commit may be a child create, the children of an actor are
listed as an operation of its own, and the parent is part of the state that is compared at the end.

The comparison does not run PostgreSQL, and PostgreSQL is the store with row locks. So the conformance suite gets
a case for the deadlock that decision 3 names: a child whose address sorts before its parent's commits a send to its
parent while the parent is deleted (first review).

Not chosen: the conformance cases alone (they test the trees we thought of; the comparison builds the ones we did
not).

### 7. What follows, with no choice to make

Decided by the main session.

- **`turn.create_child` mirrors `turn.create`**: the same arguments, the same checks of the initial state, the same
  errors.
- **The three public types of an actor gain `parent`**: `ActorRecord`, `ActorSnapshot` and `ActorView`. The value is
  an address, or absent for an independent actor.
- **The texts that say what a delete does change with it**: the description of the delete route in the OpenAPI
  document, the doc comments of `ActorSystem::delete_actor` and `Turn::stop`, the help of `cicada delete actor`, the
  question the terminal UI asks before a delete, and S2 in `docs/architecture.md`.
- **ADR 0019 is accepted and committed.** It names the store trait `Store` in places, as it was called then; the
  trait is `Backend` now. The ADR is not rewritten.
- **The main session deletes the branch `feat/child-actors` before it starts the builder** (owner, 2026-11-11). The
  branch is local only. The builder would otherwise continue on it; it starts from this design.

### 8. An actor that exists is not adopted

Decided by the main session (first review).

An actor that was created with `create` stays independent. When a host changes a parent type from `create` to
`create_child`, a parent that creates an existing actor again gets a rejected commit (decision 2), and its message
ends `failed`. A handler cannot test for that. So a host switches a type when no parent of it is left that created
actors the old way: it starts its data anew, or lets those parents finish first. Chorus and Karakuri start anew
(owner, 2026-11-11: their databases need not survive). The changelog and `docs/operations.md` say so.

Not chosen: a `create_child` that adopts an actor without a parent (it could take over an actor that another part
of the system created on purpose; ADR 0019 says a child create never takes over an actor).

### 9. A restart by another actor than the parent makes the child independent

Decided by the main session (first review).

`control-effects` lets any actor delete and create another actor in one turn. When an actor that is not the parent
restarts a child that way, the subtree of the child is gone, the new actor has no parent (or the restarter as its
parent), and the next `create_child` of it by the old parent is rejected. This is accepted: the actor that restarts
a child is its parent, which deletes it and creates it again as a child. The design of `control-effects` says so.

Not chosen: a parent address on a create, so that a restart could keep the parent (decision 1: it needs a check
that the parent exists and, on PostgreSQL, a lock on it).

### 10. The terminal UI shows the parent and the children of an actor

The owner, 2026-11-11, after the change was ready: "Terminal UI should show parent and maybe even a hierarchy if
possible". Which form, decided by the main session.

The screen of an actor gets a fact `Parent`, with the parent's address or "none", and a third tab `children` next to
`mailbox` and `outcomes`, with the direct children in address order. Enter on a child opens the child's screen, as
Enter on a row does everywhere. A key opens the parent's screen, as `a` opens the actor of a message. Esc goes back.
So a user walks the tree up and down, one level per screen.

| Option | What it adds | What it costs |
| --- | --- | --- |
| The parent and a tab of children (chosen) | The tree can be walked with the screens the UI has | One more request per refresh of an actor's screen |
| The parent only | The way up | No way down |
| The whole tree on one screen, with folding | The shape at a glance | A new kind of screen, and one request per actor of the tree on every refresh |

The whole tree on one screen is not chosen: the API lists direct children only (listing all descendants in one call
is out of scope), and the UI refreshes its screen every 2 seconds, so a tree of a thousand actors would be a
thousand requests each time.

## Risks / Trade-offs

- A large tree is a large transaction → documented in `docs/operations.md`; a tree that large removes itself
  bottom-up in its own code.
- On PostgreSQL a tree whose removal takes longer than `socket_timeout` cannot be removed in one call: every delete
  answers `Unavailable`, and a `stop()` of its root is handled again and again. → `docs/operations.md` says so,
  with the two ways out: remove the tree from the leaves up with the `children` listing, or raise `socket_timeout`.
- A node of 0.7.0 that keeps running leaves orphans → decision 5: all old nodes are stopped first.
- A host that switches a type to `create_child` on live data fails messages → decision 8.
- A delete is now stronger than before → documented; only actors created with `create_child` are affected.
- More deadlock re-runs on PostgreSQL when a tree is removed while its actors are busy → bounded by the existing
  retry; a removal that still fails reports `Unavailable` and can be repeated.

## Migration Plan

Stop all nodes of the previous version, then start the new ones; the first one migrates the store. Rolling back the
code needs a database from before the migration, as for every schema version. A host changes no code unless it
builds `ActorRecord`, `ActorSnapshot` or `ActorView` by hand. It uses `create_child` where it wants a child, and
switches an existing type only as decision 8 says.

The change is breaking (owner, 2026-11-11): the schema, the guarantee S2 and the new field of the public types. It
is released with a new minor number: 0.8.0 after 0.7.0.
