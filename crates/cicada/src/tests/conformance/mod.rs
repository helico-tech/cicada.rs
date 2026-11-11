//! The S1–S10 store conformance suite every backend runs: the in-memory store (on a virtual and on the real clock),
//! SQLite (likewise) and, when `CICADA_PG_URL` is set, PostgreSQL.
//!
//! A backend implements [`FixtureFactory`] (a fresh [`StoreFixture`] per case, and the suite's unit of time) and runs
//! every case with one macro call in its module:
//!
//! ```ignore
//! struct Sqlite;
//!
//! #[async_trait::async_trait]
//! impl FixtureFactory for Sqlite {
//!     type Fixture = SqliteFixture;
//!     async fn create(&self, retention: Duration) -> SqliteFixture { SqliteFixture::open(retention).await }
//! }
//!
//! store_conformance_tests!(Sqlite);
//! ```
//!
//! The macro expands to one `#[test]` per case, named after the case; to run the suite twice for one backend (e.g. on
//! a virtual and on the real clock), call it in two modules. Each test runs on its own multi-threaded tokio runtime,
//! so the concurrency cases run their peers in parallel. Every case is also an `async fn` in [`cases`], generic over
//! the factory, for running a single case by hand. Cases that need a capability the fixture lacks (a virtual clock,
//! reopening, stepping the clock back) print `skipped: <reason>` and pass.

#![allow(dead_code)]

/// Asserts that `$value` matches `$pattern`.
macro_rules! assert_matches {
    ($value:expr, $pattern:pat $(, $($message:tt)+)?) => {{
        let value = $value;
        assert!(
            matches!(value, $pattern),
            "expected {}, got {:?}{}",
            stringify!($pattern),
            value,
            String::new() $(+ ": " + &format!($($message)+))?
        );
    }};
}

/// Asserts that `$value` matches `$pattern` and evaluates to `$binding` (bound by the pattern).
macro_rules! expect_matches {
    ($value:expr, $pattern:pat => $binding:expr $(, $($message:tt)+)?) => {{
        match $value {
            $pattern => $binding,
            value => panic!(
                "expected {}, got {:?}{}",
                stringify!($pattern),
                value,
                String::new() $(+ ": " + &format!($($message)+))?
            ),
        }
    }};
}

mod chain;
mod env;
mod fixture;
mod operator;
mod reschedule;
mod retention;
mod robustness;
mod s1_to_s10;
mod turn_cancel;

pub use env::*;
pub use fixture::{FixtureFactory, StoreFixture};

/// Every case of the suite, as an `async fn`, one module per suite.
pub mod cases {
    pub use super::chain::*;
    pub use super::operator::*;
    pub use super::reschedule::*;
    pub use super::retention::*;
    pub use super::robustness::*;
    pub use super::s1_to_s10::*;
    pub use super::turn_cancel::*;
}

/// The runtime of one generated test.
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a tokio runtime")
}

/// Calls `$callback!($($args)* ; <every case name>)`. The one list of cases: the generated tests and [`CASES`] come
/// from it.
macro_rules! for_each_case {
    ($callback:ident ! ($($args:tt)*)) => {
        $callback!($($args)* ;
            // s1_to_s10.rs
            registry_register_is_idempotent_and_types_are_listed_by_name,
            registry_same_name_with_a_different_contract_hash_conflicts,
            registry_nodes_registering_the_same_contracts_concurrently_all_succeed,
            registry_nodes_registering_one_name_with_different_contracts_concurrently_admit_exactly_one,
            registry_remove_type_removes_an_unused_type_refuses_one_with_instances_and_reports_an_absent_one,
            registry_nodes_removing_one_type_concurrently_remove_it_once_and_a_racing_create_is_not_lost,
            instances_create_then_get_returns_the_initial_snapshot,
            instances_create_is_idempotent_and_leaves_the_existing_instance_untouched,
            instances_list_by_prefix_in_address_order_with_paging,
            instances_delete_of_an_absent_instance_reports_absent,
            s1_acknowledged_writes_survive_a_restart,
            s2_commit_applies_state_creates_sends_and_outcome_together,
            s2_a_send_to_an_absent_instance_rejects_the_whole_commit_and_keeps_the_lease,
            s2_creates_are_applied_before_sends_in_one_commit,
            s2_a_send_reusing_a_message_id_rejects_the_commit,
            s2_delete_removes_the_instance_and_purges_its_whole_mailbox,
            s2_stop_removes_the_instance_and_purges_the_rest_of_its_mailbox,
            s2_a_failed_outcome_keeps_state_and_records_the_error_and_envelope,
            s3_every_read_sees_the_preceding_acknowledged_write,
            s3_concurrent_enqueues_to_one_instance_get_distinct_seqs_in_one_order,
            s3_concurrent_creates_of_one_instance_create_it_once,
            s4_a_valid_lease_excludes_other_claims,
            s4_concurrent_claims_from_many_nodes_never_share_an_instance,
            s4_concurrent_claims_of_one_instance_yield_exactly_one_lease,
            s4_an_expired_lease_is_claimed_again_with_a_higher_epoch_and_attempt,
            s4_epochs_increase_with_every_claim_of_an_instance,
            s4_epochs_stay_monotonic_across_delete_and_re_create,
            s5_a_stale_epoch_cannot_commit_after_a_re_claim,
            s5_renew_succeeds_only_with_the_current_held_lease,
            s5_commit_after_release_or_a_second_commit_loses_the_lease,
            s5_racing_stale_and_current_commits_produce_exactly_one_outcome,
            s5_committing_a_message_other_than_the_claimed_one_is_a_programming_error,
            s6_only_the_head_message_is_claimed_one_turn_at_a_time,
            s6_equal_deliver_at_keeps_enqueue_order,
            s6_sends_from_one_turn_with_equal_deliver_at_keep_send_order,
            s6_a_message_is_never_claimed_before_its_deliver_at,
            s6_due_exactly_at_deliver_at_and_not_one_microsecond_earlier,
            s6_an_earlier_scheduled_message_overtakes_a_later_pending_one,
            s6_a_past_deliver_at_is_clamped_to_now_and_does_not_jump_the_queue,
            s6_due_ness_follows_the_new_head_after_commit_cancel_and_enqueue,
            s6_pending_lists_the_mailbox_in_delivery_order_across_pages,
            s6_pending_of_all_instances_by_prefix_instance_by_instance_in_delivery_order_across_pages,
            s7_message_ids_are_unique_across_pending_messages_and_retained_outcomes,
            s7_concurrent_enqueues_of_one_id_accept_exactly_one,
            s7_a_send_to_an_absent_instance_is_refused_and_does_not_consume_the_id,
            s7_cancel_ends_only_pending_messages,
            s7_cancelling_a_claimed_message_whose_lease_expired_fences_the_stale_turn,
            s7_release_expiry_and_lost_commits_keep_the_message_pending,
            s7_concurrent_workers_process_every_message_exactly_once_serially_and_in_order,
            s8_a_released_lease_is_claimable_again_at_once,
            s8_every_due_instance_is_eventually_claimed_under_small_limits,
            s8_claim_due_only_returns_the_requested_types,
            s8_next_due_at_reports_when_an_instance_becomes_claimable,
            s9_due_ness_delays_and_leases_are_judged_by_the_store_clock,
            s9_store_time_never_goes_backwards,
            s10_outcomes_are_retained_for_the_retention_period_then_removed,
            s10_a_message_id_can_be_reused_once_its_outcome_expired,
            s10_failures_are_listed_by_prefix_and_since_with_paging_within_retention,
            s10_outcomes_are_listed_by_status_prefix_and_since_with_paging_and_a_hidden_record_stays_out,
            stats_count_actors_and_pending_messages_under_a_prefix_and_find_the_oldest_due_one,
            // robustness.rs
            time_a_delivery_time_beyond_the_supported_range_is_refused_and_nothing_is_enqueued,
            time_the_latest_supported_delivery_time_is_stored_exactly_and_never_due,
            time_a_sub_microsecond_delivery_time_is_rounded_up_never_down,
            time_store_time_stays_monotonic_when_the_underlying_clock_steps_back,
            s4_a_lease_is_valid_strictly_before_lease_until_and_expired_at_it,
            s5_renew_and_commit_still_work_after_expiry_as_long_as_nobody_re_claimed,
            commit_repeating_a_commit_that_already_applied_changes_nothing,
            s6_equal_deliver_at_keeps_enqueue_order_on_any_clock,
            g5_a_message_is_never_claimed_before_its_deliver_at_on_any_clock,
            s7_a_commit_and_a_concurrent_enqueue_racing_for_one_id_accept_exactly_one,
            s7_two_commits_racing_for_one_send_id_accept_exactly_one,
            s7_cancel_racing_a_commit_that_sends_the_same_id_never_loses_the_message,
            s3_a_commit_that_creates_and_sends_to_a_child_is_serialised_with_concurrent_writes_to_that_child,
            closed_after_close_every_operation_fails_as_unavailable,
            // reschedule.rs
            reschedule_the_same_message_stays_pending_with_its_failure_recorded_and_no_outcome,
            reschedule_the_message_queues_again_behind_due_messages_and_is_not_due_before_its_delay,
            reschedule_the_message_gets_exactly_one_final_outcome_carrying_its_failures,
            reschedule_is_fenced_by_epoch_like_any_commit,
            reschedule_is_atomic_with_the_commits_sends_and_creates_and_a_rejected_commit_applies_nothing,
            reschedule_a_rescheduled_message_can_be_cancelled,
            reschedule_delete_and_stop_purge_a_rescheduled_message,
            reschedule_the_id_stays_in_use_while_the_message_is_rescheduled,
            reschedule_failures_and_last_error_are_durable,
            retry_policy_stored_with_the_message_and_returned_everywhere_retained_in_the_outcome_envelope,
            retry_policy_survives_reschedule_and_reopen,
            // retention.rs
            retention_the_policy_is_stored_with_the_message_and_kept_in_its_envelope_across_reschedule_and_reopen,
            retention_remove_on_done_hides_the_record_at_once_drops_its_content_and_keeps_it_a_duplicate,
            retention_a_removed_id_cannot_be_reused_by_a_commits_send,
            retention_remove_on_failed_hides_a_failure_from_failures_and_the_other_outcome_keeps_the_default,
            retention_cancelled_and_purged_messages_keep_the_store_default,
            retention_keep_for_a_time_shorter_than_the_store_retention,
            retention_keep_for_longer_than_the_store_retention,
            retention_keep_last_n_per_instance_and_outcome_leaving_other_policies_and_instances_alone,
            retention_the_sweep_keeps_hidden_records_as_duplicates_and_visible_ones_intact,
            // chain.rs
            chain_caused_by_is_stored_with_the_message_and_returned_everywhere_also_in_the_outcome_envelope,
            chain_caused_by_survives_reschedule_and_reopen,
            chain_caused_by_lists_one_level_of_pending_and_retained_visible_messages_in_id_order_across_pages,
            // turn_cancel.rs
            turn_cancel_pending_messages_of_the_instance_are_cancelled_in_the_commit_others_are_ignored,
            turn_cancel_a_cancelled_due_message_is_no_longer_claimed,
            turn_cancel_the_turns_own_message_cannot_be_cancelled_and_nothing_is_applied,
            turn_cancel_a_rejected_or_fenced_commit_cancels_nothing,
            turn_cancel_with_a_stop_the_cancelled_message_is_cancelled_and_the_rest_purged,
            // operator.rs
            registry_a_create_of_an_unregistered_type_is_refused_and_creates_nothing,
            registry_a_commit_creating_an_unregistered_type_is_rejected_applies_nothing_and_keeps_the_lease,
            registry_removing_a_type_racing_a_commit_that_creates_an_instance_of_it_leaves_no_orphan,
            nodes_a_heartbeat_upserts_the_row_with_store_time_and_nodes_are_listed_by_id,
            nodes_any_heartbeat_removes_rows_last_seen_more_than_a_day_ago,
            turns_running_turns_are_valid_leases_by_node_and_prefix_in_address_order_across_pages,
            turns_a_cancel_request_is_recorded_only_under_a_valid_lease_and_reported_only_for_its_epoch,
            turns_a_request_made_for_an_earlier_epoch_has_no_effect_on_a_later_turn,
            turns_a_cancelled_commit_is_fenced_by_its_epoch_and_moves_the_mailbox_on,
            pause_a_paused_actor_is_not_claimed_and_keeps_queueing_until_resumed,
            pause_a_running_turn_of_a_paused_actor_finishes_and_other_actors_are_unaffected,
            pause_and_purge_of_an_unknown_instance_report_it,
            purge_gives_purged_to_pending_messages_keeps_the_running_one_and_the_instance,
            purge_of_a_message_whose_lease_expired_fences_the_stale_turn,
            deliver_now_makes_a_delayed_message_due_at_once_with_a_new_seq,
            deliver_now_refuses_running_finished_and_unknown_messages,
            deliver_now_keeps_the_message_order_after_an_enqueue_following_it,
        );
    };
}

/// One `#[test]` per case named in `$case`, each running the case against a fresh fixture of `$factory`.
macro_rules! conformance_tests {
    ($factory:expr ; $($case:ident),* $(,)?) => {
        $(
            #[test]
            fn $case() {
                $crate::tests::conformance::runtime().block_on(async {
                    let backend = $factory;
                    if let Some(reason) = $crate::tests::conformance::FixtureFactory::skip_reason(&backend) {
                        eprintln!("skipped: {reason}");
                        return;
                    }
                    $crate::tests::conformance::cases::$case(&backend).await;
                });
            }
        )*
    };
}

macro_rules! case_names {
    ( ; $($case:ident),* $(,)?) => {
        /// The names of every case, in suite order.
        pub const CASES: &[&str] = &[$(stringify!($case)),*];
    };
}

for_each_case!(case_names!());

/// Generates one `#[test]` per conformance case, each running the case against a fresh fixture of `$factory` (an
/// expression of a type implementing [`FixtureFactory`], evaluated once per test).
macro_rules! store_conformance_tests {
    ($factory:expr $(,)?) => {
        for_each_case!(conformance_tests!($factory));
    };
}

// The backends. They come after the macros, which they use.
mod in_memory;
mod in_memory_real_time;
mod postgres;
mod sqlite;

/// Every `pub async fn` of the case modules is listed in [`CASES`], and only those.
#[test]
fn every_case_is_generated() {
    use std::collections::BTreeSet;

    let sources = [
        include_str!("chain.rs"),
        include_str!("s1_to_s10.rs"),
        include_str!("operator.rs"),
        include_str!("reschedule.rs"),
        include_str!("retention.rs"),
        include_str!("robustness.rs"),
        include_str!("turn_cancel.rs"),
    ];
    let defined: BTreeSet<&str> = sources
        .iter()
        .flat_map(|s| s.lines())
        .filter_map(|l| l.strip_prefix("pub async fn "))
        .map(|l| l.split(['<', '(']).next().unwrap_or_default())
        .collect();
    let listed: BTreeSet<&str> = CASES.iter().copied().collect();
    assert_eq!(defined, listed);
    assert_eq!(listed.len(), CASES.len(), "a case is listed twice");
}
