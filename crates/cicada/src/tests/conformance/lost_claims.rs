//! The count of lost claims of a message (ADR 0025): a claim of an actor that still holds a lease that ran out, which
//! no commit, release, cancel or purge cleared, adds 1 to the `lost_claims` of the message of that lease. Nothing else
//! changes it, and an outcome keeps it.

use crate::store::{Backend, CancelResult, Delivery, Message, MessageId, MessageStatus, OutcomeStatus, TurnOutcome};
use serde_json::json;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

/// The message `id` while it is pending or running.
async fn waiting(store: &dyn Backend, id: &MessageId) -> Message {
    match store.status(id).await.unwrap() {
        Some(MessageStatus::Pending(m) | MessageStatus::Running { message: m, .. }) => m,
        other => panic!("expected {id} pending or running, got {other:?}"),
    }
}

/// lost claims - a lease that runs out and is replaced counts one lost claim, and every further one counts again
pub async fn lost_claims_a_lease_that_runs_out_and_is_replaced_counts_one_lost_claim<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        assert_eq!(m.lost_claims, 0, "a new message has none");
        let first = t.claim_one().await;
        assert_eq!((first.message.attempt, first.message.lost_claims), (1, 0));
        t.expire_lease().await;
        let second = t.claim_one_node(&NODE_B).await;
        assert_eq!(second.message.id, m.id);
        assert_eq!((second.message.attempt, second.message.lost_claims), (2, 1), "the claim it replaced is lost");
        assert_eq!(waiting(&*store, &m.id).await.lost_claims, 1, "the store holds the count");
        t.expire_lease().await;
        let third = t.claim_one().await;
        assert_eq!((third.message.attempt, third.message.lost_claims), (3, 2));
        assert_eq!(t.pending(&a).await[0].lost_claims, 2, "the mailbox shows it");
    })
    .await
}

/// lost claims - a lease that ran out and is not replaced counts nothing, also after a renewal
pub async fn lost_claims_a_lease_that_ran_out_and_is_not_replaced_counts_nothing<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        t.expire_lease().await;
        assert_eq!(waiting(&*store, &m.id).await.lost_claims, 0, "running out alone loses nothing");
        store.renew(&a, c.epoch, LONG_LEASE).await.unwrap();
        assert_eq!(waiting(&*store, &m.id).await.lost_claims, 0, "nor does a renewal after it");
        assert!(t.claim().await.is_empty(), "the renewed lease holds");
    })
    .await
}

/// lost claims - a release is not counted, and neither is a release with an old epoch or a second release
pub async fn lost_claims_a_release_is_not_counted_nor_a_release_with_an_old_epoch_or_a_second_one<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let first = t.claim_one().await;
        store.release(&a, first.epoch).await.unwrap();
        let second = t.claim_one().await;
        assert_eq!((second.message.attempt, second.message.lost_claims), (2, 0), "a released claim is not lost");
        store.release(&a, first.epoch).await.unwrap(); // an old epoch: changes nothing
        store.release(&a, second.epoch).await.unwrap();
        store.release(&a, second.epoch).await.unwrap(); // a second release: changes nothing
        let third = t.claim_one().await;
        assert_eq!((third.message.attempt, third.message.lost_claims), (3, 0));
        assert_eq!(waiting(&*store, &m.id).await.lost_claims, 0);
    })
    .await
}

/// lost claims - a retry that was committed is not counted, and the count stays over it
pub async fn lost_claims_a_committed_retry_is_not_counted_and_the_count_stays_over_it<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let retry = || TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "boom".to_owned() };
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let first = t.claim_one().await;
        t.commit_ok(ending(&first, retry())).await;
        let second = t.claim_one().await;
        assert_eq!(
            (second.message.attempt, second.message.failures, second.message.lost_claims),
            (2, 1, 0),
            "a committed retry is no lost claim"
        );
        t.expire_lease().await;
        let third = t.claim_one().await;
        assert_eq!((third.message.attempt, third.message.failures, third.message.lost_claims), (3, 1, 1));
        t.commit_ok(ending(&third, retry())).await;
        let fourth = t.claim_one().await;
        assert_eq!(
            (fourth.message.attempt, fourth.message.failures, fourth.message.lost_claims),
            (4, 2, 1),
            "the count never goes back, also not after a retry"
        );
    })
    .await
}

/// lost claims - a commit after the lease ran out succeeds while no other claim came, and its message has none
pub async fn lost_claims_a_commit_after_the_lease_ran_out_succeeds_and_counts_nothing<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        t.expire_lease().await;
        t.commit_ok(done(&c)).await;
        let outcome = outcome_record(&*store, &m.id).await.expect("an outcome");
        assert_eq!(outcome.status, OutcomeStatus::Done);
        assert_eq!(outcome.envelope.lost_claims, 0);
        t.send(&a).await;
        assert_eq!(t.claim_one().await.message.lost_claims, 0, "the commit cleared the lease");
    })
    .await
}

/// lost claims - the message of a lost claim is cancelled or purged: the next message of the actor is claimed with none
pub async fn lost_claims_the_message_of_a_lost_claim_is_cancelled_or_purged_and_the_next_one_has_none<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m1 = t.send(&a).await;
        let m2 = t.send(&a).await;
        let c = t.claim_one().await;
        assert_eq!(c.message.id, m1.id);
        t.expire_lease().await;
        assert_matches!(store.cancel(&m1.id).await.unwrap(), CancelResult::Cancelled(_));
        let next = t.claim_one().await;
        assert_eq!((next.message.id.clone(), next.message.lost_claims), (m2.id.clone(), 0), "after a cancel");
        assert_eq!(outcome_record(&*store, &m1.id).await.expect("cancelled").envelope.lost_claims, 0);
        t.commit_ok(done(&next)).await;

        let b = t.new_actor(&TYPE, json!({})).await;
        t.send(&b).await;
        let c = t.claim_one().await;
        t.expire_lease().await;
        assert_eq!(store.purge(&b).await.unwrap(), Some(1));
        let m3 = t.send(&b).await;
        let next = t.claim_one().await;
        assert_eq!((next.message.id, next.message.lost_claims), (m3.id, 0), "after a purge");
        assert_ne!(next.epoch, c.epoch);
    })
    .await
}

/// lost claims - the message in an outcome keeps its count, also after a reopen
pub async fn lost_claims_the_message_in_an_outcome_keeps_its_count<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let done_message = t.send(&a).await;
        t.claim_one().await;
        t.expire_lease().await;
        let c = t.claim_one().await;
        assert_eq!(c.message.lost_claims, 1);
        t.commit_ok(done(&c)).await;
        let failed_message = t.send(&a).await;
        t.claim_one().await;
        t.expire_lease().await;
        t.claim_one().await;
        t.expire_lease().await;
        let c = t.claim_one().await;
        assert_eq!(c.message.lost_claims, 2);
        t.commit_ok(ending(&c, TurnOutcome::Failed("boom".to_owned()))).await;
        if t.fixture.supports_reopen() {
            t.reopen().await;
        }
        let store = t.store();
        let done_outcome = outcome_record(&*store, &done_message.id).await.expect("done");
        assert_eq!((done_outcome.status, done_outcome.envelope.lost_claims), (OutcomeStatus::Done, 1));
        let failed_outcome = outcome_record(&*store, &failed_message.id).await.expect("failed");
        assert_eq!((failed_outcome.status, failed_outcome.envelope.lost_claims), (OutcomeStatus::Failed, 2));
        let listed =
            store.outcomes(Some(OutcomeStatus::Failed), "", chrono::DateTime::UNIX_EPOCH, None, 10).await.unwrap();
        assert_eq!(listed.items.iter().find(|o| o.message_id == failed_message.id).unwrap().envelope.lost_claims, 2);
    })
    .await
}
