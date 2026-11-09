# Tasks

## 1. Store

- [x] 1.1 `Store::stats(prefix)` and `StoreStats` in `InMemoryStore`, `SqliteStore` and `PostgresStore`, counting
      through the primary-key range of the prefix. Verify: the conformance case
      `stats_count_actors_and_pending_messages_under_a_prefix_and_find_the_oldest_due_one` on all three.
- [x] 1.2 `ActorSystem::stats`. Verify: the HTTP test below goes through it.

## 2. HTTP

- [x] 2.1 `GET /stats?prefix=` (read route), `StatsView`, `ActorClient::stats`, `docs/openapi.json`. Verify:
      `stats_count_actors_and_pending_messages_and_name_the_oldest_due_one` (axum), `stats_count_what_is_stored`
      (client), the read-router test, the OpenAPI test.

## 3. Docs

- [x] 3.1 README table, `docs/operations.md` (mailbox depth points to `/stats`, and what a call costs), `CHANGELOG.md`.
      Verify: the doctests of `cicada-examples`.

## 4. Finish

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate store-counts --strict`.
