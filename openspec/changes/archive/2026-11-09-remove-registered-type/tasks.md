# Tasks

## 1. Store and runtime

- [x] 1.1 `Store::remove_type(name)` and `RemoveTypeResult` in `InMemoryStore`, `SqliteStore` (one write transaction)
      and `PostgresStore` (row lock `FOR UPDATE` on the registration; `create` takes `FOR SHARE` on it). Verify: the
      conformance cases `registry_remove_type_removes_an_unused_type_refuses_one_with_instances_and_reports_an_absent_one`
      and `registry_nodes_removing_one_type_concurrently_remove_it_once_and_a_racing_create_is_not_lost` on all three.
- [x] 1.2 `ActorSystem::remove_type`; `create_actor` looks the type up again after the insert and takes the actor back
      when the type is gone. Verify: `a_type_is_removed_only_while_no_actor_of_it_exists_and_is_not_served_afterwards`
      (cicada-test) and `removing_a_type_races_with_creates_without_leaving_an_actor_of_a_removed_type` (PostgreSQL, 40
      races with spread start times).

## 2. HTTP

- [x] 2.1 `DELETE /types/{name}` (write route only), problem type `type-in-use`, `ActorClient::remove_type`,
      `docs/openapi.json`. Verify: `delete_a_type_answers_204_404_and_409_while_actors_of_it_exist` (axum),
      `remove_type_answers_removed_absent_and_in_use` (client), the read-router test (the read router answers 405), the
      OpenAPI test.

## 3. Docs

- [x] 3.1 `docs/operations.md` "Changing a contract" (stop the nodes that host the type, delete its actors, call
      `DELETE /types/{name}`), README table, `CHANGELOG.md`. Verify: the doctests of `cicada-examples`.

## 4. Finish

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate remove-registered-type --strict`.
