//! A commit that reschedules the turn's message in place: the same message goes back
//! to pending with a new delivery time and `seq`, counts the failure, and gets exactly one outcome later. Also the
//! message's retry policy: stored with the message, kept across reschedules and reopen, retained with the outcome.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::retry::{Backoff, RetryPolicy};
use crate::store::{
    ActorCreate, Backend, CancelResult, CommitRequest, CommitResult, DeleteResult, Delivery, EnqueueResult, Message,
    MessageId, MessageStatus, OutcomeStatus, OutgoingMessage, Page, StateChange, TurnOutcome,
};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

fn rescheduled(error: &str, delivery: Delivery) -> TurnOutcome {
    TurnOutcome::Rescheduled { delivery, error: error.to_owned() }
}

async fn pending_message(store: &dyn Backend, id: &MessageId) -> Message {
    let status = store.status(id).await.unwrap();
    expect_matches!(status, Some(MessageStatus::Pending(m)) => m, "message {id} is pending")
}

/// Asserts that `after` is the message `before` again: same envelope, only delivery, seq and failure fields differ.
fn assert_same_message(before: &Message, after: &Message) {
    let expected = Message {
        deliver_at: after.deliver_at,
        seq: after.seq,
        failures: after.failures,
        last_error: after.last_error.clone(),
        ..before.clone()
    };
    assert_eq!(&expected, after);
}

fn policy() -> RetryPolicy {
    let backoff = Backoff::exponential(Duration::from_secs(1), 3.0, Duration::from_secs(60)).expect("a valid backoff");
    RetryPolicy::new(4, backoff).expect("a valid policy")
}

async fn failure_ids(store: &dyn Backend, since: chrono::DateTime<Utc>) -> Vec<MessageId> {
    let page =
        store.outcomes(Some(OutcomeStatus::Failed), "", since, None, Page::<Message>::DEFAULT_SIZE).await.unwrap();
    page.items.into_iter().map(|o| o.message_id).collect()
}

/// reschedule - the same message stays pending with its failure recorded and no outcome
pub async fn reschedule_the_same_message_stays_pending_with_its_failure_recorded_and_no_outcome<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let from = t.new_actor(&OTHER, json!({})).await;
        let sent = enqueue_ok(
            &*store,
            OutgoingMessage {
                from: Some(from),
                correlation_id: Some("corr".into()),
                headers: BTreeMap::from([("trace".to_owned(), "t1".to_owned())]),
                ..OutgoingMessage::new(a.clone(), "Msg", json!({"x": 1}))
            },
        )
        .await;
        let c = t.claim_one().await;
        assert_eq!(c.message.failures, 0);
        assert_eq!(c.message.last_error, None);
        let before = store.now().await.unwrap();
        let committed = t.commit_ok(ending(&c, rescheduled("boom", Delivery::After(t.ticks(2))))).await;
        let after = store.now().await.unwrap();
        assert_eq!(committed.sent, vec![]);

        assert_eq!(outcome_record(&*store, &sent.id).await, None, "no outcome while rescheduled");
        let again = pending_message(&*store, &sent.id).await;
        assert_same_message(&c.message, &again);
        assert_eq!(again.failures, 1);
        assert_eq!(again.last_error.as_deref(), Some("boom"));
        assert_eq!(again.attempt, 1, "attempt counts claims and is kept");
        assert!(again.seq > sent.seq, "a new seq");
        assert!(
            again.deliver_at >= plus(before, t.ticks(2)) && again.deliver_at <= plus(after, t.ticks(2)),
            "due at store now + delay"
        );
        assert_eq!(t.pending(&a).await, vec![again]);
        assert!(!failure_ids(&*store, before).await.contains(&sent.id), "not a failure");
        assert_eq!(store.get(&a).await.unwrap().expect("the instance").version, 0, "state unchanged");
    })
    .await
}

/// reschedule - the message queues again behind due messages and is not due before its delay
pub async fn reschedule_the_message_queues_again_behind_due_messages_and_is_not_due_before_its_delay<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        let behind = t.send(&a).await; // due now, enqueued while m was running
        t.commit_ok(ending(&c, rescheduled("first", Delivery::Now))).await;
        let pending = t.all_pending(&a, 3).await;
        assert_eq!(
            ids(&pending),
            vec![behind.id.clone(), m.id.clone()],
            "rescheduled at the same time: new seq, so behind"
        );
        assert_delivery_order(&pending);

        let c2 = t.claim_one().await;
        assert_eq!(c2.message.id, behind.id, "the due message behind it runs first");
        t.commit_ok(done(&c2)).await;
        let c3 = t.claim_one().await;
        assert_eq!(c3.message.id, m.id);
        assert_eq!(c3.message.attempt, 2);
        assert_eq!(c3.message.failures, 1);
        assert_eq!(c3.message.last_error.as_deref(), Some("first"));

        // With a delay: never due early, ordered by its new deliverAt among later sends.
        let now = store.now().await.unwrap();
        t.commit_ok(ending(&c3, rescheduled("second", Delivery::After(t.ticks(3))))).await;
        let later = t.send_at(&a, Delivery::After(t.ticks(2))).await;
        let last = t.send_at(&a, Delivery::After(t.ticks(3))).await;
        assert_eq!(ids(&t.all_pending(&a, 3).await), vec![later.id.clone(), m.id.clone(), last.id.clone()]);
        assert_eq!(pending_message(&*store, &m.id).await.deliver_at, plus(now, t.ticks(3)));
        assert_eq!(store.next_due_at(&type_set(&[&TYPE])).await.unwrap(), Some(plus(now, t.ticks(2))));
        t.advance_ticks(2).await;
        let c = t.claim_one().await;
        assert_eq!(c.message.id, later.id);
        t.commit_ok(done(&c)).await;
        assert_eq!(t.claim().await, vec![], "not due before its delay");
        t.advance_ticks(1).await;
        let c4 = t.claim_one().await;
        assert_eq!(c4.message.id, m.id);
        assert_eq!(c4.message.attempt, 3);
        assert_eq!(c4.message.failures, 2);
        assert_eq!(c4.message.last_error.as_deref(), Some("second"));
    })
    .await
}

/// reschedule - the message gets exactly one final outcome, carrying its failures
pub async fn reschedule_the_message_gets_exactly_one_final_outcome_carrying_its_failures<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let since = store.now().await.unwrap();
        let m = t.send(&a).await;
        t.commit_ok(ending(&t.claim_one().await, rescheduled("e1", Delivery::Now))).await;
        t.commit_ok(ending(&t.claim_one().await, rescheduled("e2", Delivery::Now))).await;
        let last = t.claim_one().await;
        assert_eq!(last.message.attempt, 3);
        t.commit_ok(ending(&last, TurnOutcome::Failed("final".into()))).await;
        let outcome = outcome_record(&*store, &m.id).await.expect("an outcome");
        assert_eq!(outcome.status, OutcomeStatus::Failed);
        assert_eq!(outcome.error.as_deref(), Some("final"));
        assert_eq!(outcome.envelope.failures, 2);
        assert_eq!(outcome.envelope.last_error.as_deref(), Some("e2"));
        assert_eq!(outcome.envelope.attempt, 3);
        assert_eq!(failure_ids(&*store, since).await, vec![m.id.clone()], "listed once, when final");
        assert_eq!(t.pending(&a).await, vec![]);
        assert_eq!(t.claim().await, vec![]);

        // Success on a later attempt: done.
        let n = t.send(&a).await;
        t.commit_ok(ending(&t.claim_one().await, rescheduled("e1", Delivery::Now))).await;
        t.commit_ok(done(&t.claim_one().await)).await;
        let done_outcome = outcome_record(&*store, &n.id).await.expect("an outcome");
        assert_eq!(done_outcome.status, OutcomeStatus::Done);
        assert_eq!(done_outcome.error, None);
        assert_eq!(done_outcome.envelope.failures, 1);
        assert_eq!(failure_ids(&*store, since).await, vec![m.id.clone()]);
    })
    .await
}

/// reschedule - is fenced by epoch like any commit
pub async fn reschedule_is_fenced_by_epoch_like_any_commit<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let stale = t.claim_one().await;
        t.expire_lease().await;
        let current = t.claim_one_long_node(&NODE_B).await;
        assert_eq!(t.commit(ending(&stale, rescheduled("stale", Delivery::Now))).await, CommitResult::LeaseLost);
        let status = store.status(&m.id).await.unwrap();
        let running = expect_matches!(status, Some(MessageStatus::Running { message, .. }) => message);
        assert_eq!(running.failures, 0, "nothing applied");
        assert_eq!(running.seq, current.message.seq);

        t.commit_ok(ending(&current, rescheduled("current", Delivery::Now))).await;
        assert_eq!(
            t.commit(ending(&current, rescheduled("repeated", Delivery::Now))).await,
            CommitResult::LeaseLost,
            "a repeat changes nothing"
        );
        assert_eq!(t.commit(ending(&stale, rescheduled("stale", Delivery::Now))).await, CommitResult::LeaseLost);
        let pending = pending_message(&*store, &m.id).await;
        assert_eq!(pending.failures, 1);
        assert_eq!(pending.last_error.as_deref(), Some("current"));
    })
    .await
}

/// reschedule - is atomic with the commit's sends and creates, and a rejected commit applies nothing
pub async fn reschedule_is_atomic_with_the_commits_sends_and_creates_and_a_rejected_commit_applies_nothing<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let other = t.new_actor(&OTHER, json!({})).await;
        let child = address("other/reschedule-child");
        let m = t.send(&a).await;
        let c = t.claim_one_long().await;
        let rejected = t
            .commit(CommitRequest {
                sends: vec![OutgoingMessage::new(address("other/absent"), "T", json!(1))],
                ..ending(&c, rescheduled("boom", Delivery::Now))
            })
            .await;
        assert_matches!(rejected, CommitResult::Rejected(_));
        let status = store.status(&m.id).await.unwrap();
        let running =
            expect_matches!(status, Some(MessageStatus::Running { message, .. }) => message, "still held, nothing applied");
        assert_eq!(running.failures, 0);

        let committed = t
            .commit_ok(CommitRequest {
                creates: vec![ActorCreate { address: child.clone(), state: json!({}), child: false }],
                sends: vec![
                    OutgoingMessage::new(other.clone(), "T", json!(1)),
                    OutgoingMessage::new(child.clone(), "T", json!(2)),
                ],
                ..ending(&c, rescheduled("boom", Delivery::Now))
            })
            .await;
        assert_eq!(committed.sent.len(), 2);
        assert_eq!(t.pending(&other).await.len(), 1);
        assert_eq!(t.pending(&child).await.len(), 1);
        let pending = pending_message(&*store, &m.id).await;
        assert_eq!(pending.failures, 1);
        assert!(committed.sent.iter().all(|s| s.seq > pending.seq), "the rescheduled message is queued before the sends");
    })
    .await
}

/// reschedule - a rescheduled message can be cancelled
pub async fn reschedule_a_rescheduled_message_can_be_cancelled<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        t.commit_ok(ending(&c, rescheduled("e1", Delivery::After(Duration::from_secs(86_400))))).await;
        let cancelled = expect_matches!(store.cancel(&m.id).await.unwrap(), CancelResult::Cancelled(o) => o);
        assert_eq!(cancelled.status, OutcomeStatus::Cancelled);
        assert_eq!(cancelled.envelope.failures, 1);
        assert_eq!(cancelled.envelope.last_error.as_deref(), Some("e1"));
        assert_eq!(outcome_record(&*store, &m.id).await.map(|o| o.status), Some(OutcomeStatus::Cancelled));
        assert_eq!(t.pending(&a).await, vec![]);
        assert_eq!(store.next_due_at(&type_set(&[&TYPE])).await.unwrap(), None);
        assert_matches!(store.cancel(&m.id).await.unwrap(), CancelResult::AlreadyFinished(_));
    })
    .await
}

/// reschedule - delete and stop purge a rescheduled message
pub async fn reschedule_delete_and_stop_purge_a_rescheduled_message<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        t.commit_ok(ending(&c, rescheduled("e1", Delivery::After(Duration::from_secs(86_400))))).await;
        assert_eq!(store.delete(&a, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { purged: 1 });
        let purged = outcome_record(&*store, &m.id).await.expect("an outcome");
        assert_eq!(purged.status, OutcomeStatus::Purged);
        assert_eq!(purged.envelope.failures, 1);

        // A rescheduling commit that also stops the instance purges the message it rescheduled.
        let b = t.new_actor(&TYPE, json!({})).await;
        let n = t.send(&b).await;
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest { state: StateChange::Stop, ..ending(&c, rescheduled("boom", Delivery::Now)) }).await;
        assert_eq!(store.get(&b).await.unwrap(), None);
        assert_eq!(outcome_record(&*store, &n.id).await.map(|o| o.status), Some(OutcomeStatus::Purged));
    })
    .await
}

/// reschedule - the id stays in use while the message is rescheduled
pub async fn reschedule_the_id_stays_in_use_while_the_message_is_rescheduled<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        t.commit_ok(ending(&c, rescheduled("e1", Delivery::After(Duration::from_secs(86_400))))).await;
        let reuse = OutgoingMessage { id: m.id.clone(), ..OutgoingMessage::new(a.clone(), "Msg", json!(9)) };
        assert_eq!(store.enqueue(reuse.clone()).await.unwrap(), EnqueueResult::Duplicate(m.id.clone()));
        let sender = t.new_actor(&OTHER, json!({})).await;
        t.send(&sender).await;
        let c = t.claim_one_types(&[&OTHER]).await;
        assert_matches!(t.commit(CommitRequest { sends: vec![reuse], ..done(&c) }).await, CommitResult::Rejected(_));
        assert_eq!(ids(&t.pending(&a).await), vec![m.id.clone()]);
        assert_eq!(pending_message(&*store, &m.id).await.failures, 1);
    })
    .await
}

/// reschedule - failures and lastError are durable
pub async fn reschedule_failures_and_last_error_are_durable<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        t.commit_ok(ending(&c, rescheduled("persisted", Delivery::After(Duration::from_secs(86_400))))).await;
        if t.fixture.supports_reopen() {
            t.fixture.reopen().await;
            let again = pending_message(&*t.store(), &m.id).await;
            assert_eq!(again.failures, 1);
            assert_eq!(again.last_error.as_deref(), Some("persisted"));
            assert_eq!(t.pending(&a).await, vec![again]);
        }
    })
    .await
}

/// retry policy - stored with the message and returned everywhere, retained in the outcome envelope
pub async fn retry_policy_stored_with_the_message_and_returned_everywhere_retained_in_the_outcome_envelope<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let policy = policy();
        let a = t.new_actor(&TYPE, json!({})).await;
        let sent = enqueue_ok(
            &*store,
            OutgoingMessage { retry: Some(policy.clone()), ..OutgoingMessage::new(a.clone(), "Msg", json!({"x": 1})) },
        )
        .await;
        assert_eq!(sent.retry.as_ref(), Some(&policy));
        let plain = t.send(&a).await;
        assert_eq!(plain.retry, None, "no policy: null");
        assert_eq!(pending_message(&*store, &sent.id).await.retry.as_ref(), Some(&policy));
        assert_eq!(
            t.pending(&a).await.into_iter().map(|m| m.retry).collect::<Vec<_>>(),
            vec![Some(policy.clone()), None]
        );

        let c = t.claim_one().await;
        assert_eq!(c.message.id, sent.id);
        assert_eq!(c.message.retry.as_ref(), Some(&policy));
        // A commit's sends carry their policies too.
        let b = t.new_actor(&OTHER, json!({})).await;
        let forwarded = OutgoingMessage {
            from: Some(a.clone()),
            retry: Some(policy.clone()),
            ..OutgoingMessage::new(b.clone(), "Msg", json!({"y": 2}))
        };
        let committed = t.commit_ok(CommitRequest { sends: vec![forwarded.clone()], ..done(&c) }).await;
        assert_eq!(committed.sent.len(), 1);
        assert_eq!(committed.sent[0].retry.as_ref(), Some(&policy));
        assert_eq!(pending_message(&*store, &forwarded.id).await.retry.as_ref(), Some(&policy));
        assert_eq!(
            outcome_record(&*store, &sent.id).await.expect("an outcome").envelope.retry.as_ref(),
            Some(&policy),
            "retained with the outcome"
        );

        let c = t.claim_one().await;
        assert_eq!(c.message.id, plain.id);
        t.commit_ok(ending(&c, TurnOutcome::Failed("x".into()))).await;
        assert_eq!(outcome_record(&*store, &plain.id).await.expect("an outcome").envelope.retry, None);
    })
    .await
}

/// retry policy - survives reschedule and reopen
pub async fn retry_policy_survives_reschedule_and_reopen<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let policy = policy();
        let a = t.new_actor(&TYPE, json!({})).await;
        let sent = enqueue_ok(
            &*t.store(),
            OutgoingMessage { retry: Some(policy.clone()), ..OutgoingMessage::new(a.clone(), "Msg", json!({"x": 1})) },
        )
        .await;
        let c = t.claim_one().await;
        t.commit_ok(ending(&c, rescheduled("e1", Delivery::Now))).await;
        let again = pending_message(&*t.store(), &sent.id).await;
        assert_eq!(again.retry.as_ref(), Some(&policy), "kept by the reschedule");
        assert_eq!(again.failures, 1);
        if t.fixture.supports_reopen() {
            t.fixture.reopen().await;
            assert_eq!(pending_message(&*t.store(), &sent.id).await.retry.as_ref(), Some(&policy));
            let pending = t.pending(&a).await;
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].retry.as_ref(), Some(&policy));
        }
        let last = t.claim_one().await;
        assert_eq!(last.message.retry.as_ref(), Some(&policy));
        t.commit_ok(ending(&last, TurnOutcome::Failed("final".into()))).await;
        if t.fixture.supports_reopen() {
            t.fixture.reopen().await;
        }
        let store = t.store();
        let outcome = outcome_record(&*store, &sent.id).await.expect("an outcome");
        assert_eq!(outcome.envelope.retry.as_ref(), Some(&policy));
        assert_eq!(outcome.envelope.failures, 1);
        let failures = store
            .outcomes(Some(OutcomeStatus::Failed), "", DateTime::<Utc>::MIN_UTC, None, Page::<Message>::DEFAULT_SIZE)
            .await
            .unwrap();
        let failure: Vec<_> = failures.items.iter().filter(|o| o.message_id == sent.id).collect();
        assert_eq!(failure.len(), 1);
        assert_eq!(failure[0].envelope.retry.as_ref(), Some(&policy));
    })
    .await
}
