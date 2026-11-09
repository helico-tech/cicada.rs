# Tasks

## 1. Store

- [x] 1.1 `actors.paused`; `Store::set_paused` and `Store::purge`; `claim_due` and `next_due_at` skip paused actors,
      with `actors_due` excluding them in its partial predicate. Verify: conformance cases
      `pause_a_paused_actor_is_not_claimed_and_keeps_queueing_until_resumed`,
      `pause_a_running_turn_of_a_paused_actor_finishes_and_other_actors_are_unaffected`,
      `pause_and_purge_of_an_unknown_instance_report_it`,
      `purge_gives_purged_to_pending_messages_keeps_the_running_one_and_the_instance`,
      `purge_of_a_message_whose_lease_expired_fences_the_stale_turn`; EXPLAIN on both databases shows `actors_due` used
      by the claim query.

## 2. Runtime and HTTP

- [x] 2.1 `GET .../mailbox` (replaces `GET .../messages`, items with `state`), `ActorSystem::mailbox`, read route; `POST
      .../mailbox/pause`, `/resume`, `/purge`, write routes; resume wakes the dispatch loop locally and on PostgreSQL
      sends the `NOTIFY`; `ActorView.paused`; client methods. Verify:
      `the_mailbox_marks_running_and_can_be_paused_resumed_and_purged` (axum), runtime tests
      `a_paused_mailbox_runs_nothing_until_resumed_and_the_snapshot_says_so`,
      `the_mailbox_marks_the_running_message_and_purge_spares_it`, the client test, the OpenAPI test.

## 3. Docs

- [x] 3.1 S8 exception in `docs/architecture.md`, ADR 0015, README, CHANGELOG. Verify: doctests.

## 4. Finish

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
      with PostgreSQL, `openspec validate mailbox-management --strict`.
