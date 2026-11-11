# Draft tasks

A draft, not agreed. This change is not ready: its `tasks.md` is written when its turn comes, after the design was
gone through and reviewed (ADR 0020). Written on 2026-11-11, against the code before steps 1 to 4 of ADR 0018: check
crate and type names against the code then, and drop a task whose store is gone.

## 1. Store

- [ ] 1.1 Model and trait: `ActorCreate.child`, `ActorRecord.parent`, `Store::children`, the shared check
      `support::check_creates`, and the new wording of S2 on `Store`. Verify: `cargo build -p cicada`.
- [ ] 1.2 `InMemoryStore`: the parent on the row, `children`, the removal of descendants in `delete` and `Stop`, the
      rejected child create. Verify: the conformance cases of 1.5 on the in-memory store.
- [ ] 1.3 `SqliteStore`: an appended migration (`actors.parent`, index `actors_parent`), the same behaviour with one
      recursive query. Verify: the conformance cases of 1.5, and
      `a_database_of_the_previous_version_is_migrated_and_its_actors_have_no_parent`.
- [ ] 1.4 `PostgresStore`: an appended migration, the same behaviour, locking the tree level by level before the clock
      is read. Verify: the conformance cases of 1.5 with `CICADA_PG_URL` set, and
      `a_schema_of_the_previous_version_is_migrated_and_its_actors_have_no_parent`.
- [ ] 1.5 Conformance cases in `children.rs`:
      `children_a_commit_creates_a_child_of_the_committing_instance_and_other_creates_have_no_parent`,
      `children_are_listed_by_parent_in_address_order_with_paging`,
      `children_delete_removes_every_descendant_and_purges_their_mailboxes`,
      `children_stop_removes_every_descendant_including_children_of_the_same_commit`,
      `children_stop_of_a_child_leaves_its_parent_and_siblings`,
      `children_a_child_create_of_a_foreign_instance_rejects_the_commit_and_keeps_the_lease`,
      `children_creates_of_one_instance_that_disagree_are_invalid`,
      `children_a_delete_racing_with_commits_that_create_children_leaves_no_orphan`. Verify: they pass on all three
      stores.

## 2. Runtime

- [ ] 2.1 `Turn::create_child`; a second create of one address that disagrees is `TurnError::Invalid`.
      `ActorSnapshot.parent`, `ActorSystem::list_children`. Verify (virtual time, `TestActors`):
      `a_child_knows_its_parent_and_is_listed_under_it`, `stopping_a_parent_removes_its_children`,
      `deleting_a_parent_removes_the_whole_tree`, `a_child_create_of_a_foreign_actor_fails_the_message`,
      `creating_one_address_both_ways_in_a_turn_is_refused`.

## 3. HTTP, client and command line

- [ ] 3.1 `ActorView.parent`; `GET /actors/{type}/{key}/children` as a read route; `ActorClient::children`;
      `docs/openapi.json`. Verify: `children_are_listed_below_their_parent_and_the_view_names_the_parent`, the
      read-router test, the OpenAPI test and the client test `children_of_an_actor_are_listed`.
- [ ] 3.2 `cicada get children ADDR`, and `PARENT` in the wide table of an actor. Verify: the `tests/cli.rs` case for
      `get children` and the unit test of the columns.

## 4. Example and docs

- [ ] 4.1 The `fanout` example creates its items with `create_child`. Verify: `deleting_a_batch_removes_its_items`.
- [ ] 4.2 ADR 0019; `docs/architecture.md` (domain language, S2, turn semantics, schema); README (the turn API, the
      routes, the command line); `docs/operations.md` (what a delete removes, large trees, the upgrade); CHANGELOG.
      Verify: `cargo test -p cicada-examples --doc`.

## 5. Finish

- [ ] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with
      PostgreSQL, and `openspec validate child-actors --strict`. Verify: all pass.
