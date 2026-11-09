//! Message causation: the store keeps `caused_by` with every message and returns it wherever the message is returned,
//! also in the envelope of its outcome.

use chrono::{DateTime, Utc};
use cicada::address::ActorAddress;
use cicada::store::{
    CommitRequest, Delivery, EnqueueResult, Message, MessageId, MessageStatus, OutgoingMessage, Page, TurnOutcome,
};
use serde_json::json;

use crate::env::*;
use crate::fixture::{FixtureFactory, StoreFixture};

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
        let a = t.new_actor(&TYPE, None).await;
        let b = t.new_actor(&OTHER, None).await;
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
        let failures = store.failures("", DateTime::<Utc>::MIN_UTC, None, Page::<Message>::DEFAULT_SIZE).await.unwrap();
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
        let a = t.new_actor(&TYPE, None).await;
        let b = t.new_actor(&OTHER, None).await;
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
            t.fixture.reopen().await;
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
            t.fixture.reopen().await;
        }
        let outcome = outcome_record(&*t.store(), &child.id).await.expect("an outcome");
        assert_eq!(outcome.envelope.caused_by.as_ref(), Some(&root.id), "kept with the outcome");
        let failures =
            t.store().failures("", DateTime::<Utc>::MIN_UTC, None, Page::<Message>::DEFAULT_SIZE).await.unwrap();
        assert_eq!(failures.items.len(), 1);
        assert_eq!(failures.items[0].envelope.caused_by.as_ref(), Some(&root.id));
    })
    .await
}
