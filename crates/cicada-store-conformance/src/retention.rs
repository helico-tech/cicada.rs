//! The sender's retention policy: how long a finished message's record stays visible,
//! while a minimal record keeps dedup and chains working for the store retention.

use std::collections::BTreeMap;

use cicada::address::ActorAddress;
use cicada::retention::{Keep, Retention};
use cicada::store::{
    CancelResult, ChainEntry, CommitRequest, CommitResult, Delivery, EnqueueResult, Message, MessageStatus,
    OutcomeStatus, OutgoingMessage, Page, Store, TurnOutcome,
};
use cicada::time::Timestamp;
use serde_json::{Value, json};

use crate::env::*;
use crate::fixture::{FixtureFactory, StoreFixture};

async fn send_kept<F: StoreFixture>(
    t: &Env<F>,
    to: &ActorAddress,
    retention: Option<Retention>,
    delivery: Delivery,
) -> Message {
    let message = OutgoingMessage {
        delivery,
        correlation_id: Some("corr".into()),
        headers: BTreeMap::from([("h".to_owned(), "v".to_owned())]),
        retention,
        ..OutgoingMessage::new(to.clone(), "Msg", json!({"secret": 1}))
    };
    enqueue_ok(&*t.store(), message).await
}

/// Claims `message` (the due head of its instance) and commits it with `outcome`.
async fn finish<F: StoreFixture>(t: &Env<F>, message: &Message, outcome: TurnOutcome) {
    let c = t.claim_one_types(&[&TYPE, &OTHER]).await;
    assert_eq!(c.message.id, message.id);
    t.commit_ok(ending(&c, outcome)).await;
}

async fn is_duplicate(store: &dyn Store, message: &Message) -> bool {
    let again =
        OutgoingMessage { id: message.id.clone(), ..OutgoingMessage::new(message.to.clone(), "Msg", json!({"x": 2})) };
    store.enqueue(again).await.unwrap() == EnqueueResult::Duplicate(message.id.clone())
}

async fn failure_ids(store: &dyn Store, since: Timestamp) -> Vec<cicada::store::MessageId> {
    let page = store.failures("", since, None, Page::<Message>::DEFAULT_SIZE).await.unwrap();
    page.items.into_iter().map(|o| o.message_id).collect()
}

/// retention - the policy is stored with the message and kept in its envelope, across reschedule and reopen
pub async fn retention_the_policy_is_stored_with_the_message_and_kept_in_its_envelope_across_reschedule_and_reopen<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, None).await;
        let policy = Retention { done: Some(Keep::For { age: t.ticks(100) }), failed: Some(Keep::Last { count: 3 }) };
        let m = send_kept(t, &a, Some(policy.clone()), Delivery::Now).await;
        assert_eq!(m.retention.as_ref(), Some(&policy));
        let status = t.store().status(&m.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Pending(p)) => p).retention.as_ref(), Some(&policy));
        let pending = t.pending(&a).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].retention.as_ref(), Some(&policy));
        let c = t.claim_one().await;
        t.commit_ok(ending(&c, TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "e".into() })).await;
        if t.fixture.supports_reopen() {
            t.fixture.reopen().await;
        }
        let c = t.claim_one().await;
        assert_eq!(c.message.retention.as_ref(), Some(&policy));
        t.commit_ok(done(&c)).await;
        if t.fixture.supports_reopen() {
            t.fixture.reopen().await;
        }
        let outcome = t.store().outcome(&m.id).await.unwrap().expect("an outcome");
        assert_eq!(outcome.envelope.retention.as_ref(), Some(&policy));
        assert_eq!(t.send(&a).await.retention, None, "no policy: null");
    })
    .await
}

/// retention - remove on done hides the record at once, drops its content and keeps it a duplicate
pub async fn retention_remove_on_done_hides_the_record_at_once_drops_its_content_and_keeps_it_a_duplicate<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let removed = send_kept(t, &a, Some(Retention::REMOVE_ON_DONE), Delivery::Now).await;
        finish(t, &removed, TurnOutcome::Done).await;
        assert_eq!(store.status(&removed.id).await.unwrap(), None, "gone from status");
        assert_eq!(store.outcome(&removed.id).await.unwrap(), None);
        assert_eq!(store.cancel(&removed.id).await.unwrap(), CancelResult::NotFound);
        assert!(is_duplicate(&*store, &removed).await, "still a duplicate within the dedup window");
        assert_eq!(t.pending(&a).await, vec![], "nothing was enqueued again");
    })
    .await
}

/// retention - a removed id cannot be reused by a commit's send
pub async fn retention_a_removed_id_cannot_be_reused_by_a_commits_send<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, None).await;
        let removed = send_kept(t, &a, Some(Retention::REMOVE_ON_DONE), Delivery::Now).await;
        finish(t, &removed, TurnOutcome::Done).await;
        t.send(&a).await;
        let c = t.claim_one().await;
        let reuse =
            OutgoingMessage { id: removed.id.clone(), ..OutgoingMessage::new(a.clone(), "Msg", json!({"x": 1})) };
        assert_matches!(t.commit(CommitRequest { sends: vec![reuse], ..done(&c) }).await, CommitResult::Rejected(_));
    })
    .await
}

/// retention - remove on failed hides a failure from failures, and the other outcome keeps the default
pub async fn retention_remove_on_failed_hides_a_failure_from_failures_and_the_other_outcome_keeps_the_default<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let since = store.now().await.unwrap();
        let failed = send_kept(t, &a, Some(Retention::REMOVE_ON_FAILED), Delivery::Now).await;
        let done_msg = send_kept(t, &a, Some(Retention::REMOVE_ON_FAILED), Delivery::Now).await;
        let failed_kept = send_kept(t, &a, Some(Retention::REMOVE_ON_DONE), Delivery::Now).await;
        finish(t, &failed, TurnOutcome::Failed("boom".into())).await;
        finish(t, &done_msg, TurnOutcome::Done).await;
        finish(t, &failed_kept, TurnOutcome::Failed("kept".into())).await;
        assert_eq!(store.status(&failed.id).await.unwrap(), None);
        assert_eq!(failure_ids(&*store, since).await, vec![failed_kept.id.clone()]);
        let kept = store.outcome(&done_msg.id).await.unwrap().expect("an outcome");
        assert_eq!(kept.status, OutcomeStatus::Done);
        assert_eq!(kept.envelope.payload, json!({"secret": 1}), "a kept record keeps its content");
        let kept_failure = store.outcome(&failed_kept.id).await.unwrap().expect("an outcome");
        assert_eq!(kept_failure.error.as_deref(), Some("kept"));
    })
    .await
}

/// retention - cancelled and purged messages keep the store default
pub async fn retention_cancelled_and_purged_messages_keep_the_store_default<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let all = Retention { done: Some(Keep::Remove), failed: Some(Keep::Remove) };
        let cancelled = send_kept(t, &a, Some(all.clone()), Delivery::After(t.ticks(100))).await;
        assert_matches!(store.cancel(&cancelled.id).await.unwrap(), CancelResult::Cancelled(_));
        assert_eq!(store.outcome(&cancelled.id).await.unwrap().expect("an outcome").status, OutcomeStatus::Cancelled);
        let purged = send_kept(t, &a, Some(all), Delivery::After(t.ticks(100))).await;
        store.delete(&a).await.unwrap();
        assert_eq!(store.outcome(&purged.id).await.unwrap().expect("an outcome").status, OutcomeStatus::Purged);
    })
    .await
}

/// retention - keep for a time shorter than the store retention
pub async fn retention_keep_for_a_time_shorter_than_the_store_retention<B: FixtureFactory>(backend: &B) {
    store_test_with(backend, backend.tick() * 20, DEFAULT_TIMEOUT, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let m = send_kept(
            t,
            &a,
            Some(Retention { done: Some(Keep::For { age: t.ticks(5) }), failed: None }),
            Delivery::Now,
        )
        .await;
        finish(t, &m, TurnOutcome::Done).await;
        t.advance_ticks(4).await;
        assert_eq!(
            store.outcome(&m.id).await.unwrap().expect("an outcome").envelope.payload,
            json!({"secret": 1}),
            "visible within its age"
        );
        t.advance_ticks(2).await;
        assert_eq!(store.outcome(&m.id).await.unwrap(), None, "hidden after its age");
        assert!(is_duplicate(&*store, &m).await, "dedup lasts for the store retention");
        assert_eq!(store.sweep_expired().await.unwrap(), 0, "nothing to delete yet");
        let entries = store.chain_entries(std::slice::from_ref(&m.id)).await.unwrap();
        assert_eq!(entries.len(), 1);
        let payload = expect_matches!(&entries[0], ChainEntry::Removed { envelope, .. } => &envelope.payload);
        assert_eq!(payload, &Value::Null, "the minimal record has no content");
        t.advance_ticks(15).await;
        assert_eq!(store.sweep_expired().await.unwrap(), 1);
        assert_eq!(store.chain_entries(std::slice::from_ref(&m.id)).await.unwrap(), vec![]);
    })
    .await
}

/// retention - keep for longer than the store retention
pub async fn retention_keep_for_longer_than_the_store_retention<B: FixtureFactory>(backend: &B) {
    store_test_with(backend, backend.tick() * 5, DEFAULT_TIMEOUT, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let m = send_kept(
            t,
            &a,
            Some(Retention { done: None, failed: Some(Keep::For { age: t.ticks(20) }) }),
            Delivery::Now,
        )
        .await;
        let plain = send_kept(t, &a, None, Delivery::Now).await;
        let since = store.now().await.unwrap();
        finish(t, &m, TurnOutcome::Failed("boom".into())).await;
        finish(t, &plain, TurnOutcome::Failed("plain".into())).await;
        t.advance_ticks(10).await;
        assert_eq!(store.sweep_expired().await.unwrap(), 1, "only the default record expired");
        assert_eq!(store.outcome(&m.id).await.unwrap().expect("an outcome").error.as_deref(), Some("boom"));
        assert_eq!(failure_ids(&*store, since).await, vec![m.id.clone()]);
        assert!(is_duplicate(&*store, &m).await, "a visible record is a duplicate too");
        assert_matches!(store.cancel(&m.id).await.unwrap(), CancelResult::AlreadyFinished(_));
        t.advance_ticks(11).await;
        assert_eq!(store.outcome(&m.id).await.unwrap(), None);
        assert_eq!(store.sweep_expired().await.unwrap(), 1);
    })
    .await
}

/// retention - keep last N per instance and outcome, leaving other policies and instances alone
pub async fn retention_keep_last_n_per_instance_and_outcome_leaving_other_policies_and_instances_alone<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let b = t.new_actor(&OTHER, None).await;
        let last2 = Retention { done: Some(Keep::Last { count: 2 }), failed: Some(Keep::Last { count: 1 }) };
        let plain = send_kept(t, &a, None, Delivery::Now).await;
        finish(t, &plain, TurnOutcome::Done).await;
        let mut kept = Vec::new();
        for _ in 0..4 {
            let m = send_kept(t, &a, Some(last2.clone()), Delivery::Now).await;
            finish(t, &m, TurnOutcome::Done).await;
            kept.push(m);
        }
        let mut failed = Vec::new();
        for i in 0..2 {
            let m = send_kept(t, &a, Some(last2.clone()), Delivery::Now).await;
            finish(t, &m, TurnOutcome::Failed(format!("f{i}"))).await;
            failed.push(m);
        }
        let other = send_kept(t, &b, Some(last2.clone()), Delivery::Now).await;
        finish(t, &other, TurnOutcome::Done).await;

        let mut visible = Vec::new();
        for m in &kept {
            visible.push(store.outcome(&m.id).await.unwrap().is_some());
        }
        assert_eq!(visible, vec![false, false, true, true], "the newest two");
        let mut visible = Vec::new();
        for m in &failed {
            visible.push(store.outcome(&m.id).await.unwrap().is_some());
        }
        assert_eq!(visible, vec![false, true], "failed counted apart");
        assert!(store.outcome(&plain.id).await.unwrap().is_some(), "another policy is never trimmed");
        assert!(store.outcome(&other.id).await.unwrap().is_some(), "another instance is never trimmed");
        for m in &kept {
            assert!(is_duplicate(&*store, m).await, "trimmed records stay duplicates");
        }
        let entries = store.chain_entries(std::slice::from_ref(&kept[0].id)).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_matches!(&entries[0], ChainEntry::Removed { .. });
    })
    .await
}

/// retention - the sweep drops the content of hidden records and keeps visible ones intact
pub async fn retention_the_sweep_drops_the_content_of_hidden_records_and_keeps_visible_ones_intact<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test_with(backend, backend.tick() * 20, DEFAULT_TIMEOUT, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let short =
            send_kept(t, &a, Some(Retention { done: Some(Keep::For { age: t.tick }), failed: None }), Delivery::Now)
                .await;
        let plain = send_kept(t, &a, None, Delivery::Now).await;
        finish(t, &short, TurnOutcome::Done).await;
        finish(t, &plain, TurnOutcome::Done).await;
        t.advance_ticks(2).await;
        assert_eq!(store.sweep_expired().await.unwrap(), 0);
        let entries = store.chain_entries(std::slice::from_ref(&short.id)).await.unwrap();
        assert_eq!(entries.len(), 1);
        let (status, envelope) =
            expect_matches!(&entries[0], ChainEntry::Removed { status, envelope, .. } => (status, envelope));
        assert_eq!(envelope.payload, Value::Null);
        assert_eq!(envelope.headers, BTreeMap::new());
        assert_eq!(envelope.correlation_id, None);
        assert_eq!(envelope.message_type, "Msg", "the minimal record keeps type and target");
        assert_eq!(envelope.to, a);
        assert_eq!(*status, OutcomeStatus::Done);
        let visible = store.outcome(&plain.id).await.unwrap().expect("an outcome");
        assert_eq!(visible.envelope.headers, BTreeMap::from([("h".to_owned(), "v".to_owned())]));
    })
    .await
}
