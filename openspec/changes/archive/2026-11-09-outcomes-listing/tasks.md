# Tasks

## 1. Store

- [x] 1.1 `Store::outcomes(status, prefix, since, cursor, limit)` replaces `Store::failures` in `InMemoryStore`,
      `SqliteStore` and `PostgresStore`; `outcomes_finished` covers `(finished_at, message_id)` in both schemas. Verify:
      the conformance case `s10_outcomes_are_listed_by_status_prefix_and_since_with_paging_and_a_hidden_record_stays_out`
      and the changed failure cases pass on all three; `EXPLAIN` shows `outcomes_failures` with a status and
      `outcomes_finished` without (SQLite and PostgreSQL).
- [x] 1.2 `ActorSystem::outcomes`. Verify: `status_cancel_outcomes_types` in `cicada-test/tests/operations.rs`.

## 2. HTTP

- [x] 2.1 `GET /outcomes` replaces `GET /failures`; an invalid `status` is `400`; `ActorClient::outcomes`;
      `docs/openapi.json`. Verify: `outcomes_list_finished_messages_with_envelope_and_error` (axum), `outcomes_and_types`
      (client), the OpenAPI test.
- [x] 2.2 `FailureWatcher`, the e2e crate and the sample service ask for failed outcomes. Verify: `cargo test -p
      cicada-extras`, `cargo build --workspace --all-targets` (the Docker suite was not run).

## 3. Docs

- [x] 3.1 ADR 0014, README endpoint table, `docs/operations.md`, `CHANGELOG.md`. Verify: the doctests of
      `cicada-examples`.

## 4. Finish

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate outcomes-listing --strict`.
