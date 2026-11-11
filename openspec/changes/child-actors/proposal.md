# Proposal

## Why

An actor can create other actors in a turn, but nothing records who created what. Deleting or stopping an actor leaves
the actors it created behind: in the `fanout` example an orphaned item fails its reply to the missing batch and never
stops, and Chorus never removes the tasks of a workflow. A tool cannot ask what belongs to an actor, because a key
prefix finds children of one type only. An operator delete runs no handler, so no code above the core can clean up at
that moment. ADR 0019 decides on a parent link.

## What Changes

- An actor may have a **parent**: the actor whose turn created it as a child. `turn.create` keeps creating an
  independent actor. The new `turn.create_child` creates a child of the actor that runs the turn. A create from
  outside a turn is always independent. A parent never changes.
- **BREAKING** Deleting an actor, or stopping it with `turn.stop()`, also removes every descendant in the same
  transaction. Their pending messages end `purged`; a turn running on one of them commits nothing. This changes the
  guarantee S2.
- A child create of an actor that exists with another parent, or with none, fails the turn's message.
- **BREAKING** An actor's view has `parent`, so the public types `ActorRecord`, `ActorSnapshot` and `ActorView` gain
  a field. `GET /actors/{type}/{key}/children` lists the direct children, as do `ActorSystem::list_children`,
  `ActorClient::children` and `cicada get children`.
- **BREAKING** An appended migration adds the nullable column `actors.parent` and an index on it, in both stores.
  A node of the previous version that opens the migrated database refuses it. One that is still running would
  delete a parent without its children, so every old node is stopped before the first new one starts.
- The `fanout` example creates its items as children.

The simplest version: one nullable column and one rule for delete and stop. Addresses stay `type/key`.

## Capabilities

### New Capabilities

- `child-actors`: an actor can own the actors it creates; they are listed by their parent and removed with it.

### Modified Capabilities

None. The requirement texts of `sqlite-store` and `management-api` do not change: the migration is appended and the
OpenAPI document is regenerated, as those specs already ask.

## Out of scope

- Addresses that show the tree (ADR 0013 stays).
- Supervision: a parent is not told when a child fails. That is a watch (ADR 0018).
- Setting a parent from outside a turn, changing a parent, or adopting an existing actor.
- Pause, resume and purge of a whole tree, and bulk actions by parent.
- Listing all descendants in one call, or only the actors without a parent. A client walks the tree level by level.
- A tree view in the terminal UI.
- A background or batched removal for very large trees.

## Order

ADR 0019. After `tidy-the-core` (step 4 of ADR 0018), before `watches` (step 5).

- After steps 1 and 2: every actor has state by then, so there is no stateless variant of `create_child`, and the
  store trait is private, so its new method and fields break nobody.
- Before steps 5 and 6: watches and control effects must know that removing an actor removes its children. Built in
  this order, they cover it from the start.

## Impact

- `crates/cicada` (`store`, `turn`, `runtime`, `api`), the stores and their migrations, the store conformance suite,
  the randomised comparison of the stores,
  `cicada-axum`, `cicada-client`, `cicada-cli`, the `fanout` example, `README.md`, `docs/architecture.md`,
  `docs/operations.md`, `docs/openapi.json` and `CHANGELOG.md`.
- S2 changes, so ADR 0019.
- Breaking for code that builds `ActorRecord`, `ActorSnapshot` or `ActorView` by hand: each gains a field. Callers
  of `create`, `delete` and `stop` compile unchanged.
- The branch `feat/child-actors` holds a first, unfinished version of the store code, written before steps 1 to 4.
  It is deleted before the builder starts; the builder starts from the design (`design.md`, decision 7).
- A delete now removes the children too. Actors created before this change have no parent, so nothing changes for
  them as long as their creator keeps using `create`. A host that switches a type to `create_child` does so for new
  parents only: a parent that creates an existing independent actor again as a child fails its message.
- All nodes are upgraded together: the old ones are stopped first.
