# Tasks

## 1. Store

- [x] 1.1 `actors.cancel_epoch`; `Store::running_turns`, `running_turn`, `request_cancel`; `RenewResult::CancelRequested`;
      `TurnOutcome::Cancelled` in all three stores; a partial index `actors_running`. Verify: conformance cases
      `turns_running_turns_are_valid_leases_by_node_and_prefix_in_address_order_across_pages`,
      `turns_a_cancel_request_is_recorded_only_under_a_valid_lease_and_reported_only_for_its_epoch`,
      `turns_a_request_made_for_an_earlier_epoch_has_no_effect_on_a_later_turn`,
      `turns_a_cancelled_commit_is_fenced_by_its_epoch_and_moves_the_mailbox_on`.

## 2. Runtime

- [x] 2.1 Keeper on `CancelRequested`: an atomic `HandlerPhase` decides between handler end and cancel request; abort
      *work*, commit `Cancelled` fenced by epoch; ignore a request after the handler ended (G6). Verify (virtual time):
      `cancel_takes_effect_within_one_heartbeat_with_no_effects_and_on_outcome_only`,
      `a_handler_that_already_returned_is_not_cancelled_and_commits_done`,
      `a_request_from_an_earlier_epoch_does_not_cancel_a_later_turn`,
      `a_cancelled_turn_with_retry_left_is_not_retried`; real time:
      `blocking_code_runs_until_it_returns_and_then_the_commit_is_cancelled`.

## 3. HTTP

- [x] 3.1 `GET /turns`, `GET /actors/{type}/{key}/turn` (`404 no-running-turn`), read routes; `POST
      /actors/{type}/{key}/turn/cancel` (`202`, `404`), write route; client methods. Verify:
      `turns_lists_running_turns_filtered_and_one_actors_turn_can_be_read_and_cancelled`, the client test, the OpenAPI
      test, the read-router test.

## 4. Docs

- [x] 4.1 Turn semantics row in `docs/architecture.md`, ADR 0015, README, CHANGELOG. Verify: doctests.

## 5. Finish

- [x] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate turn-management --strict`.
