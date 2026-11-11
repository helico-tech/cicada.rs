# 0019. Child actors: a parent link

Date: 2026-11-11

## Status

Accepted

## Context

An actor can create other actors in a turn, but nothing records who created what. Three gaps follow:

- Deleting or stopping an actor leaves the actors it created behind. In the `fanout` example an orphaned item fails
  its reply to the missing batch, so its own `stop` never commits and it stays forever. Chorus never removes the tasks
  of a workflow.
- A tool cannot ask "what belongs to this actor". A key prefix finds children of one type only.
- An operator delete runs no handler, so no code above the core can clean up at that moment. Children that watch their
  parent (ADR 0018) could stop themselves later, but that is delayed, needs code in every child type and gives no
  query.

A full hierarchy (paths as addresses, a parent that supervises its children) was considered and dropped. ADR 0013
removed path addresses on purpose, and ADR 0018 puts supervision outside the core, on watches.

## Decision

An actor may have a **parent**: the actor whose turn created it as a child. The parent is stored on the actor's row.
It is the model of Kubernetes owner references: each object names its owner, and removing the owner removes what it
owns.

- **Chosen at create.** `turn.create` creates an independent actor, as before. `turn.create_child` creates a child of
  the actor that runs the turn. A create from outside a turn (`ActorSystem`, HTTP) is always independent.
- **Fixed for life.** A parent is never changed or removed. Because a child is created only by its parent's own
  commit, a child's parent always exists, and there are no cycles.
- **Addresses stay flat.** A child has an ordinary `type/key` address (ADR 0013). The parent is a column
  (`actors.parent`, null for an independent actor), not part of the address.
- **One removal for the whole tree.** Deleting an actor (`Store::delete`, `DELETE /actors/{type}/{key}`) or stopping
  it (`turn.stop()`) also removes every descendant, in the same transaction. For each removed actor the rules of a
  single delete hold: every pending message ends `purged`, a running turn loses its lease and commits nothing.
  `DeleteResult::Deleted.purged` counts the messages of the whole tree.
- **A child create never takes over an actor.** Creating a child that already exists as a child of the same parent
  leaves it untouched (idempotent, as every create). Creating a child that exists with another parent, or with none,
  rejects the commit: the message ends `failed`, as for a send to an absent actor. Creates of one address in one
  commit must agree on being a child or not; the turn refuses the second call (`TurnError::Invalid`), the store
  answers `StoreError::Invalid`.
- **Query.** An actor's view has `parent` (null when independent). `Store::children`, `ActorSystem::list_children`
  and `GET /actors/{type}/{key}/children` list the direct children in address order, paged. `cicada get children`
  does the same on the command line.
- **PostgreSQL locks the tree level by level.** A removal locks the actor, then its children, then theirs, each level
  with its own statement. A child is created only in a commit that holds its parent's row lock, so once a level is
  locked its set of children can no longer grow, and a new statement sees every child that was committed before. One
  recursive query would read a single snapshot and could miss a child that a concurrent commit adds. These locks are
  not taken in address order, so a removal can deadlock with a commit; PostgreSQL aborts one of them and the store
  re-runs it, as it already does for deadlocks.
- **No meaning beyond ownership.** A parent is not told about its children's failures (that is a watch), and pause,
  resume and purge do not cascade.

## Consequences

- S2 changes: `delete` removes the instance, its descendants and their mailboxes together. `docs/architecture.md` has
  the new wording, and the term *parent* in the domain language.
- The schema gets an appended migration in both stores: the nullable column `actors.parent` and the partial index
  `actors_parent (parent, address)`. Existing actors are independent. A node of an earlier version refuses a migrated
  database, as for every newer schema.
- The wire format grows: `ActorView.parent` and the `children` route. Nothing is removed.
- `Store` gains `children`; `ActorCreate` gains `child` and `ActorRecord` gains `parent`. The store trait is private
  by then (ADR 0018, step 2), so this breaks no caller.
- Removing a large tree is one large transaction: it writes one `purged` outcome per pending message of every
  descendant. On SQLite it holds the single writer for that time. Throughput is not a goal; a tree of many thousands
  of actors with full mailboxes should be removed bottom-up by its own code instead.
- An operator delete is now stronger: it removes everything the actor owns. The operations guide says so.
- The change `child-actors` is built after step 4 of ADR 0018 and before step 5. Steps 1 and 2 make it smaller: with
  state on every actor there is no stateless `create_child`, and the store trait is private. Steps 5 and 6 build on
  it: a watch sees each removed descendant stop, and a delete by a turn (control effects) removes a tree in the same
  way. Both are left to their own changes.
- The `fanout` example uses `create_child`, so deleting a batch removes its items.
