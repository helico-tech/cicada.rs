# Tasks

## 1. Schema and stores

- [x] 1.1 Both baseline schemas: index `messages(caused_by)` (partial), column `outcomes.caused_by` and its partial index;
      the commit path writes the column from the envelope. Verify: the conformance case
      `chain_caused_by_lists_one_level_of_pending_and_retained_visible_messages_in_id_order_across_pages` finds an
      outcome by cause after the message left the mailbox, on all three stores.
- [x] 1.2 `Store::caused_by(id, cursor, limit)` in `InMemoryStore`, `SqliteStore` and `PostgresStore`. Verify: the same
      case (paging by 2, one level only, running, finished, removed at once, past retention).
- [x] 1.3 `ActorSystem::caused_by` (`None` when `id` is unknown). Verify: the HTTP 404 case below.

## 2. HTTP

- [x] 2.1 `GET /messages/{id}/caused` (read route), `ActorClient::caused_by`, `docs/openapi.json`. Verify:
      `caused_lists_the_messages_a_turn_sent_one_level_down_with_paging` (axum), `caused_by_lists_what_a_message_sent`
      (client), the OpenAPI test.

## 3. Docs

- [x] 3.1 README table and the section "Message causes", `CHANGELOG.md`. Verify: the doctests of `cicada-examples`.

## 4. Finish

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate messages-by-cause --strict`.
