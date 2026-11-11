//! A turn cancelling pending messages of its own instance: the commit's `cancels`,
//! applied atomically with the rest of the commit.

use crate::store::{
    CancelResult, CommitRequest, CommitResult, Delivery, MessageId, MessageStatus, OutcomeStatus, OutgoingMessage,
    StateChange, StoreError,
};
use serde_json::json;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::FixtureFactory;

async fn outcome_of(store: &dyn crate::store::Store, id: &MessageId) -> Option<OutcomeStatus> {
    outcome_record(store, id).await.map(|o| o.status)
}

/// turn cancel - pending messages of the instance are cancelled in the commit, others are ignored
pub async fn turn_cancel_pending_messages_of_the_instance_are_cancelled_in_the_commit_others_are_ignored<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let head = t.send(&a).await;
        let later = t.send_at(&a, Delivery::After(t.ticks(50))).await;
        let later_still = t.send_at(&a, Delivery::After(t.ticks(60))).await;
        let elsewhere = t.send(&b).await;
        let finished = t.send_at(&a, Delivery::After(t.ticks(70))).await;
        assert_matches!(store.cancel(&finished.id).await.unwrap(), CancelResult::Cancelled(_));
        let c = t.claim_one().await;
        assert_eq!(c.message.id, head.id);

        let unknown = MessageId::random();
        let committed = t
            .commit_ok(CommitRequest {
                cancels: vec![later.id.clone(), elsewhere.id.clone(), unknown, finished.id.clone(), later.id.clone()],
                ..done(&c)
            })
            .await;
        assert_eq!(committed.cancelled, vec![later.id.clone()], "only this instance's pending messages, each once");
        assert_eq!(outcome_of(&*store, &later.id).await, Some(OutcomeStatus::Cancelled));
        assert_eq!(ids(&t.all_pending(&a, 3).await), vec![later_still.id]);
        assert_matches!(
            store.status(&elsewhere.id).await.unwrap(),
            Some(MessageStatus::Pending(_)),
            "another instance's message is untouched"
        );
        assert_eq!(outcome_of(&*store, &head.id).await, Some(OutcomeStatus::Done));
    })
    .await
}

/// turn cancel - a cancelled due message is no longer claimed
pub async fn turn_cancel_a_cancelled_due_message_is_no_longer_claimed<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let first = t.send(&a).await;
        let due = t.send(&a).await;
        let third = t.send(&a).await;
        let c = t.claim_one().await;
        assert_eq!(c.message.id, first.id);
        t.commit_ok(CommitRequest { cancels: vec![due.id], ..done(&c) }).await;
        assert_eq!(t.claim_one().await.message.id, third.id, "the cancelled message is skipped");
    })
    .await
}

/// turn cancel - the turn's own message cannot be cancelled, and nothing is applied
pub async fn turn_cancel_the_turns_own_message_cannot_be_cancelled_and_nothing_is_applied<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let other = t.send(&a).await;
        let c = t.claim_one().await;
        assert_matches!(
            store.commit(CommitRequest { cancels: vec![other.id.clone(), m.id.clone()], ..done(&c) }).await,
            Err(StoreError::Invalid(_))
        );
        assert_matches!(store.status(&other.id).await.unwrap(), Some(MessageStatus::Pending(_)));
        t.commit_ok(done(&c)).await; // the lease is still held
    })
    .await
}

/// turn cancel - a rejected or fenced commit cancels nothing
pub async fn turn_cancel_a_rejected_or_fenced_commit_cancels_nothing<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let keep = t.send_at(&a, Delivery::After(t.ticks(50))).await;
        let c = t.claim_one().await;
        let absent = address("other/absent");
        let rejected = t
            .commit(CommitRequest {
                sends: vec![OutgoingMessage::new(absent, "T", json!(1))],
                cancels: vec![keep.id.clone()],
                ..done(&c)
            })
            .await;
        assert_matches!(rejected, CommitResult::Rejected(_));
        assert_matches!(store.status(&keep.id).await.unwrap(), Some(MessageStatus::Pending(_)));
        t.expire_lease().await;
        t.claim_one_node(&NODE_B).await;
        assert_eq!(
            t.commit(CommitRequest { cancels: vec![keep.id.clone()], ..done(&c) }).await,
            CommitResult::LeaseLost
        );
        assert_matches!(store.status(&keep.id).await.unwrap(), Some(MessageStatus::Pending(_)));
    })
    .await
}

/// turn cancel - with a stop, the cancelled message is cancelled and the rest purged
pub async fn turn_cancel_with_a_stop_the_cancelled_message_is_cancelled_and_the_rest_purged<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let cancelled = t.send_at(&a, Delivery::After(t.ticks(50))).await;
        let purged = t.send_at(&a, Delivery::After(t.ticks(60))).await;
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest { state: StateChange::Stop, cancels: vec![cancelled.id.clone()], ..done(&c) }).await;
        assert_eq!(outcome_of(&*store, &cancelled.id).await, Some(OutcomeStatus::Cancelled));
        assert_eq!(outcome_of(&*store, &purged.id).await, Some(OutcomeStatus::Purged));
        assert!(store.get(&a).await.unwrap().is_none());
        assert!(outcome_record(&*store, &cancelled.id).await.is_some());
    })
    .await
}
