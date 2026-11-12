//! Message causation: the store keeps `caused_by` with every message and returns it wherever the message is returned,
//! also in the envelope of its outcome.

use crate::address::ActorAddress;
use crate::store::{
    CommitRequest, Delivery, EnqueueResult, Message, MessageId, MessageStatus, OutcomeStatus, OutgoingMessage, Page,
    TurnOutcome,
};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::retention::Retention;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

fn caused(to: &ActorAddress, by: &MessageId, from: Option<&ActorAddress>) -> OutgoingMessage {
    OutgoingMessage {
        from: from.cloned(),
        caused_by: Some(by.clone()),
        ..OutgoingMessage::new(to.clone(), "Msg", json!({"x": 1}))
    }
}

/// chain - causedBy is stored with the message and returned everywhere, also in the outcome envelope
pub async fn chain_caused_by_is_stored_with_the_message_and_returned_everywhere_also_in_the_outcome_envelope<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let root = t.send(&a).await;
        assert_eq!(root.caused_by, None, "a send from outside has no cause");
        let c = t.claim_one().await;
        assert_eq!(c.message.caused_by, None);
        let child = caused(&b, &root.id, Some(&a));
        let committed = t.commit_ok(CommitRequest { sends: vec![child.clone()], ..done(&c) }).await;
        assert_eq!(committed.sent[0].caused_by.as_ref(), Some(&root.id));
        assert_eq!(committed.sent.len(), 1);
        let status = store.status(&child.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Pending(m)) => m).caused_by.as_ref(), Some(&root.id));
        assert_eq!(single_message(t.pending(&b).await).caused_by.as_ref(), Some(&root.id));
        let listed = store.list_pending("", None, Page::<Message>::DEFAULT_SIZE).await.unwrap().items;
        let listed: Vec<&Message> = listed.iter().filter(|m| m.id == child.id).collect();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].caused_by.as_ref(), Some(&root.id));

        // Enqueued directly with a cause (the store does not judge who sets it).
        let direct = expect_matches!(
            store.enqueue(caused(&a, &root.id, None)).await.unwrap(),
            EnqueueResult::Enqueued(m) => m
        );
        assert_eq!(direct.caused_by.as_ref(), Some(&root.id));

        let c2 = t.claim_one_types(&[&OTHER]).await;
        assert_eq!(c2.message.caused_by.as_ref(), Some(&root.id));
        t.commit_ok(ending(&c2, TurnOutcome::Failed("boom".into()))).await;
        assert_eq!(
            outcome_record(&*store, &child.id).await.expect("an outcome").envelope.caused_by.as_ref(),
            Some(&root.id),
            "retained with the outcome"
        );
        let failures = store
            .outcomes(Some(OutcomeStatus::Failed), "", DateTime::<Utc>::MIN_UTC, None, Page::<Message>::DEFAULT_SIZE)
            .await
            .unwrap();
        assert_eq!(failures.items.len(), 1);
        assert_eq!(failures.items[0].envelope.caused_by.as_ref(), Some(&root.id));
    })
    .await
}

fn single_message(mut messages: Vec<Message>) -> Message {
    assert_eq!(messages.len(), 1, "expected exactly one message, got {messages:?}");
    messages.remove(0)
}

/// chain - causedBy survives reschedule and reopen
pub async fn chain_caused_by_survives_reschedule_and_reopen<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let root = t.send(&a).await;
        let child = caused(&b, &root.id, Some(&a));
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest { sends: vec![child.clone()], ..done(&c) }).await;
        let c = t.claim_one_types(&[&OTHER]).await;
        t.commit_ok(ending(&c, TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "e".into() })).await;
        let status = t.store().status(&child.id).await.unwrap();
        assert_eq!(
            expect_matches!(status, Some(MessageStatus::Pending(m)) => m).caused_by.as_ref(),
            Some(&root.id),
            "kept by the reschedule"
        );
        if t.fixture.supports_reopen() {
            t.reopen().await;
        }
        let status = t.store().status(&child.id).await.unwrap();
        assert_eq!(
            expect_matches!(status, Some(MessageStatus::Pending(m)) => m).caused_by.as_ref(),
            Some(&root.id),
            "kept across a reopen"
        );
        assert_eq!(single_message(t.pending(&b).await).caused_by.as_ref(), Some(&root.id));
        let c = t.claim_one_types(&[&OTHER]).await;
        assert_eq!(c.message.caused_by.as_ref(), Some(&root.id));
        t.commit_ok(ending(&c, TurnOutcome::Failed("boom".into()))).await;
        if t.fixture.supports_reopen() {
            t.reopen().await;
        }
        let outcome = outcome_record(&*t.store(), &child.id).await.expect("an outcome");
        assert_eq!(outcome.envelope.caused_by.as_ref(), Some(&root.id), "kept with the outcome");
        let failures = t
            .store()
            .outcomes(Some(OutcomeStatus::Failed), "", DateTime::<Utc>::MIN_UTC, None, Page::<Message>::DEFAULT_SIZE)
            .await
            .unwrap();
        assert_eq!(failures.items.len(), 1);
        assert_eq!(failures.items[0].envelope.caused_by.as_ref(), Some(&root.id));
    })
    .await
}

/// chain - caused_by lists one level of pending and retained visible messages in id order across pages
pub async fn chain_caused_by_lists_one_level_of_pending_and_retained_visible_messages_in_id_order_across_pages<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test_with(backend, backend.tick() * 10, DEFAULT_TIMEOUT, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let root = t.send(&a).await;
        let kid = |n: &str, retention| OutgoingMessage {
            id: MessageId::new(format!("kid-{n}")).unwrap(),
            retention,
            ..caused(&b, &root.id, Some(&a))
        };
        // Sent in an order other than the id order, one of them removed at once when it is done.
        let sends = vec![kid("3", None), kid("1", None), kid("2", Some(Retention::REMOVE_ON_DONE))];
        t.commit_ok(CommitRequest { sends, ..done(&t.claim_one().await) }).await;
        let all = async || {
            let mut out = Vec::new();
            let mut cursor = None;
            loop {
                let page = store.caused_by(&root.id, cursor.as_ref(), 2).await.unwrap();
                assert!(page.items.len() <= 2);
                out.extend(page.items);
                match page.next {
                    Some(next) => cursor = Some(next),
                    None => return out,
                }
            }
        };
        let listed = |statuses: &[MessageStatus]| -> Vec<String> {
            statuses.iter().map(|s| s.envelope().id.to_string()).collect()
        };
        let before = all().await;
        assert_eq!(listed(&before), ["kid-1", "kid-2", "kid-3"]);
        assert!(before.iter().all(|s| matches!(s, MessageStatus::Pending(_))));
        assert_eq!(store.caused_by(&root.id, None, 10).await.unwrap().next, None);

        // One level only, and only what the message caused: a grandchild and a stranger are not listed.
        let grandchild = caused(&a, &MessageId::new("kid-1").unwrap(), Some(&b));
        enqueue_ok(&*store, grandchild.clone()).await;
        t.send(&a).await;
        assert_eq!(listed(&all().await), ["kid-1", "kid-2", "kid-3"]);
        let grandchildren = store.caused_by(&MessageId::new("kid-1").unwrap(), None, 10).await.unwrap().items;
        assert_eq!(listed(&grandchildren), [grandchild.id.to_string()]);
        assert!(store.caused_by(&MessageId::random(), None, 10).await.unwrap().items.is_empty());

        // kid-3 is handled first (the mailbox is in send order), then kid-1 is done, kid-2 removed at once.
        let running = single(t.claim_with(&[&OTHER], &NODE_A, LONG_LEASE, 10).await);
        assert_eq!(running.message.id.as_str(), "kid-3");
        let states = all().await;
        assert!(matches!(&states[2], MessageStatus::Running { .. }), "{states:?}");
        t.commit_ok(ending(&running, TurnOutcome::Failed("boom".into()))).await;
        for _ in 0..2 {
            t.commit_ok(done(&t.claim_one_types(&[&OTHER]).await)).await;
        }
        let after = all().await;
        assert_eq!(listed(&after), ["kid-1", "kid-3"], "the removed record stays out");
        assert!(matches!(&after[0], MessageStatus::Finished(o) if o.status == OutcomeStatus::Done));
        assert!(matches!(&after[1], MessageStatus::Finished(o) if o.status == OutcomeStatus::Failed));

        // Past retention the outcomes go; the grandchild is still pending.
        t.advance_ticks(11).await;
        store.sweep_expired().await.unwrap();
        assert!(all().await.is_empty());
        assert_eq!(store.caused_by(&MessageId::new("kid-1").unwrap(), None, 10).await.unwrap().items.len(), 1);
    })
    .await
}
