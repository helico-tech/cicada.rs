//! The S1–S10 store conformance suite every Cicada store backend runs (the Kotlin `actors-store-conformance`).
//!
//! A backend implements [`FixtureFactory`] (a fresh [`StoreFixture`] per case, and the suite's unit of time) and runs
//! every case with one macro call in its tests:
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
//! cicada_store_conformance::store_conformance_tests!(Sqlite);
//! ```
//!
//! The macro expands to one `#[test]` per case, named after the case; to run the suite twice in one test crate (e.g.
//! on a virtual and on the real clock), call it in two modules. Each test runs on its own multi-threaded tokio
//! runtime, so the concurrency cases run their peers in parallel. Every case is also a public `async fn` in
//! [`cases`], generic over the factory, for running a single case by hand. Cases that need a capability the fixture
//! lacks (a virtual clock, reopening, stepping the clock back) print `skipped: <reason>` and pass, as JUnit's
//! `assumeTrue` skips them in Kotlin.

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
mod conformance;
mod env;
mod fixture;
mod reschedule;
mod retention;
mod robustness;
mod turn_cancel;

pub use env::*;
pub use fixture::{FixtureFactory, StoreFixture};

/// Every case of the suite, as a public `async fn` named after the Kotlin case in snake_case, one module per Kotlin
/// suite class.
pub mod cases {
    pub use crate::chain::*;
    pub use crate::conformance::*;
    pub use crate::reschedule::*;
    pub use crate::retention::*;
    pub use crate::robustness::*;
    pub use crate::turn_cancel::*;
}

#[doc(hidden)]
pub mod __private {
    /// The runtime of one generated test.
    pub fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a tokio runtime")
    }
}

/// Calls `$callback!($($args)* ; <every case name>)`. The one list of cases: the generated tests and [`CASES`] come
/// from it.
#[doc(hidden)]
#[macro_export]
macro_rules! __for_each_case {
    ($callback:ident ! ($($args:tt)*)) => {
        $crate::$callback!($($args)* ;
            // conformance.rs
            registry_register_is_idempotent_and_types_are_listed_by_template,
            registry_same_template_with_a_different_contract_hash_conflicts,
            registry_overlapping_templates_conflict,
            registry_templates_that_share_no_address_coexist_also_within_one_namespace,
            registry_nodes_registering_the_same_contracts_concurrently_all_succeed,
            registry_nodes_registering_overlapping_templates_concurrently_admit_exactly_one,
            registry_resolve_finds_the_one_matching_template_and_decodes_its_parameters,
            instances_create_then_get_returns_the_initial_snapshot,
            instances_create_is_idempotent_and_leaves_the_existing_instance_untouched,
            instances_list_by_prefix_in_uri_order_with_paging,
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
            retention_the_sweep_drops_the_content_of_hidden_records_and_keeps_visible_ones_intact,
            // chain.rs
            chain_caused_by_is_stored_with_the_message_and_returned_everywhere_also_in_the_outcome_envelope,
            chain_entries_by_id_are_pending_running_or_finished_and_unknown_ids_are_left_out,
            chain_children_are_found_across_pending_messages_and_outcomes_up_to_the_limit,
            chain_caused_by_survives_reschedule_and_reopen,
            chain_outcomes_past_retention_are_not_part_of_a_chain,
            // turn_cancel.rs
            turn_cancel_pending_messages_of_the_instance_are_cancelled_in_the_commit_others_are_ignored,
            turn_cancel_a_cancelled_due_message_is_no_longer_claimed,
            turn_cancel_the_turns_own_message_cannot_be_cancelled_and_nothing_is_applied,
            turn_cancel_a_rejected_or_fenced_commit_cancels_nothing,
            turn_cancel_with_a_stop_the_cancelled_message_is_cancelled_and_the_rest_purged,
        );
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __store_conformance_tests {
    ($factory:expr ; $($case:ident),* $(,)?) => {
        $(
            #[test]
            fn $case() {
                $crate::__private::runtime().block_on(async {
                    let backend = $factory;
                    if let Some(reason) = $crate::FixtureFactory::skip_reason(&backend) {
                        eprintln!("skipped: {reason}");
                        return;
                    }
                    $crate::cases::$case(&backend).await;
                });
            }
        )*
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __case_names {
    ( ; $($case:ident),* $(,)?) => {
        /// The names of every case, in suite order.
        pub const CASES: &[&str] = &[$(stringify!($case)),*];
    };
}

__for_each_case!(__case_names!());

/// Generates one `#[test]` per conformance case, each running the case against a fresh fixture of `$factory` (an
/// expression of a type implementing [`FixtureFactory`], evaluated once per test).
#[macro_export]
macro_rules! store_conformance_tests {
    ($factory:expr $(,)?) => {
        $crate::__for_each_case!(__store_conformance_tests!($factory));
    };
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    /// Every `pub async fn` of the case modules is listed in [`super::CASES`], and only those.
    #[test]
    fn every_case_is_generated() {
        let sources = [
            include_str!("chain.rs"),
            include_str!("conformance.rs"),
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
        let listed: BTreeSet<&str> = super::CASES.iter().copied().collect();
        assert_eq!(defined, listed);
        assert_eq!(listed.len(), super::CASES.len(), "a case is listed twice");
    }
}
