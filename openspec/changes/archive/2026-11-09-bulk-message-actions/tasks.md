# Tasks

## 1. Runtime

- [x] 1.1 `ActorSystem::bulk_cancel` and `bulk_retry` on the single operations and the listings `list_pending` and
      `outcomes` with status failed; no new store methods; at most 1000 per call; a retry filter skips messages already
      retried. Verify: runtime tests `bulk_cancel_by_filter_and_ids_counts_only_what_it_cancelled`,
      `bulk_retry_by_filter_skips_already_retried_messages`, `bulk_filter_caps_at_a_thousand_and_reports_more`,
      `bulk_retry_by_filter_caps_at_a_thousand_and_does_not_repeat_on_the_next_call`.

## 2. HTTP

- [x] 2.1 `POST /messages/bulk-cancel` and `/bulk-retry` with a strict body (`ids` or a filter, else `400`, at most 1000
      ids), answer `200 {affected, more}`; client methods. Verify:
      `bulk_cancel_by_filter_or_ids_answers_affected_and_more`, `bulk_bodies_need_exactly_ids_or_a_filter`,
      `bulk_retry_skips_ineligible_messages_without_counting_them` (axum), the client test, the OpenAPI test.

## 3. Docs

- [x] 3.1 Impact line of the proposal, ADR 0015, README, CHANGELOG. Verify: doctests.

## 4. Finish

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate bulk-message-actions --strict`.
