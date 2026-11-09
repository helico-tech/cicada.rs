# Tasks

## 1. Store and runtime

- [x] 1.1 `Store::deliver_now` (`DeliverNowResult`) in all three stores; `ActorSystem::retry_message` (`RetryResult`) on
      `status` and `enqueue`; `ActorSystem::deliver_now`. Verify: conformance cases
      `deliver_now_makes_a_delayed_message_due_at_once_with_a_new_seq`,
      `deliver_now_refuses_running_finished_and_unknown_messages`,
      `deliver_now_keeps_the_message_order_after_an_enqueue_following_it`; runtime tests
      `retry_sends_a_failed_or_cancelled_message_again_as_a_new_message_and_refuses_the_rest` and
      `deliver_now_makes_a_delayed_message_run_now`.

## 2. HTTP

- [x] 2.1 `POST /messages/{id}/cancel` (replaces `DELETE /messages/{id}`), `POST /messages/{id}/retry`, problem type
      `message-not-retryable`, `POST /messages/{id}/deliver-now`; client methods. Verify:
      `retry_answers_202_with_a_new_message_and_the_refusals`,
      `deliver_now_answers_200_with_the_message_and_409_for_running_or_finished`,
      `delete_of_a_message_is_gone_and_cancel_is_a_post` (axum), the client test, the OpenAPI test.

## 3. Docs

- [x] 3.1 ADR 0015, README, CHANGELOG. Verify: doctests.

## 4. Finish

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate message-actions --strict`.
