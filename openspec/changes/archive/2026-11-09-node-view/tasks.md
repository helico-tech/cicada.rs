# Tasks

## 1. Store

- [x] 1.1 `nodes` table (version 1 baseline) and `Store::heartbeat_node`, `list_nodes`, `remove_node` in all three
      stores; a heartbeat removes rows older than a day. Verify: conformance cases
      `nodes_a_heartbeat_upserts_the_row_with_store_time_and_nodes_are_listed_by_id` and
      `nodes_any_heartbeat_removes_rows_last_seen_more_than_a_day_ago` on in-memory, SQLite and PostgreSQL.

## 2. Runtime

- [x] 2.1 `ActorSystem::start` heartbeats at once and every 10 s (send-only nodes too; failures logged and retried);
      `stop` removes the row after the task ended. `ActorSystem::nodes` and `node`: status from store time, running
      turns. Verify: `a_started_node_is_listed_alive_goes_suspect_and_dead_and_its_row_goes_on_stop` and
      `a_node_heartbeats_every_10_seconds_also_when_it_hosts_nothing` (virtual time).

## 3. HTTP

- [x] 3.1 `GET /nodes`, `GET /nodes/{node}` (`404 no-such-node`), read routes, `ActorClient::nodes` and `node`. Verify:
      `nodes_lists_this_node_alive_and_a_detail_lists_its_running_turns` (axum), the client test, the OpenAPI test, the
      read-router test.

## 4. Docs

- [x] 4.1 `docs/operations.md` paragraph, README table, CHANGELOG, ADR 0015. Verify: doctests of `cicada-examples`.

## 5. Finish

- [x] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate node-view --strict`.
