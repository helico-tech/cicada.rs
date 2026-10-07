//! Message causation (Kotlin `StoreChainSuite`): the store keeps `caused_by` with every message and its outcome, and
//! finds messages by id and by cause (`chain_entries`, `chain_children`).

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use cicada::address::ActorAddress;
use cicada::store::{
    ChainEntry, CommitRequest, Delivery, EnqueueResult, Message, MessageId, MessageStatus, OutgoingMessage, Page,
    TurnOutcome,
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

fn live_envelope(entry: &ChainEntry) -> &Message {
    expect_matches!(entry, ChainEntry::Live(status) => status.envelope())
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
            store.outcome(&child.id).await.unwrap().expect("an outcome").envelope.caused_by.as_ref(),
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

/// chain - entries by id are pending, running or finished, and unknown ids are left out
pub async fn chain_entries_by_id_are_pending_running_or_finished_and_unknown_ids_are_left_out<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let b = t.new_actor(&OTHER, None).await;
        let done_msg = t.send(&a).await;
        t.commit_ok(done(&t.claim_one().await)).await;
        let running = t.send(&b).await;
        t.claim_one_types(&[&OTHER]).await;
        let pending = t.send(&a).await;
        let unknown = MessageId::random();

        let all = store
            .chain_entries(&[done_msg.id.clone(), running.id.clone(), pending.id.clone(), unknown, done_msg.id.clone()])
            .await
            .unwrap();
        assert_eq!(all.len(), 3, "each id once: {all:?}");
        let entries: HashMap<MessageId, ChainEntry> = all.into_iter().map(|e| (e.id().clone(), e)).collect();
        assert_eq!(
            entries.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([done_msg.id.clone(), running.id.clone(), pending.id.clone()]),
            "each id once, the unknown one left out"
        );
        assert_matches!(&entries[&done_msg.id], ChainEntry::Live(MessageStatus::Finished(_)));
        assert_matches!(&entries[&running.id], ChainEntry::Live(MessageStatus::Running { .. }));
        assert_matches!(&entries[&pending.id], ChainEntry::Live(MessageStatus::Pending(_)));
        assert_eq!(store.chain_entries(&[]).await.unwrap(), vec![]);
    })
    .await
}

/// chain - children are found across pending messages and outcomes, up to the limit
pub async fn chain_children_are_found_across_pending_messages_and_outcomes_up_to_the_limit<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let b = t.new_actor(&OTHER, None).await;
        let root = t.send(&a).await;
        let other = t.send(&a).await;
        let c = t.claim_one().await;
        assert_eq!(c.message.id, root.id);
        let kids: Vec<OutgoingMessage> = (0..4).map(|_| caused(&b, &root.id, Some(&a))).collect();
        t.commit_ok(CommitRequest { sends: kids.clone(), ..done(&c) }).await;
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest { sends: vec![caused(&b, &other.id, Some(&a))], ..done(&c) }).await;
        // Two of root's children finish (one done, one failed); two stay pending.
        let k0 = t.claim_one_types(&[&OTHER]).await;
        assert_eq!(k0.message.id, kids[0].id);
        t.commit_ok(done(&k0)).await;
        let k1 = t.claim_one_types(&[&OTHER]).await;
        assert_eq!(k1.message.id, kids[1].id);
        t.commit_ok(ending(&k1, TurnOutcome::Failed("x".into()))).await;

        let children = store.chain_children(std::slice::from_ref(&root.id), 10).await.unwrap();
        assert_eq!(
            children.iter().map(|c| c.id().clone()).collect::<BTreeSet<_>>(),
            kids.iter().map(|k| k.id.clone()).collect::<BTreeSet<_>>()
        );
        assert!(children.iter().all(|c| c.caused_by() == Some(&root.id)));
        let statuses: HashMap<MessageId, MessageStatus> =
            children.into_iter().map(|c| (c.id().clone(), expect_matches!(c, ChainEntry::Live(s) => s))).collect();
        assert_matches!(&statuses[&kids[0].id], MessageStatus::Finished(_));
        assert_matches!(&statuses[&kids[1].id], MessageStatus::Finished(_));
        assert_matches!(&statuses[&kids[2].id], MessageStatus::Pending(_));

        assert_eq!(
            store.chain_children(&[root.id.clone(), other.id.clone()], 10).await.unwrap().len(),
            5,
            "several parents at once"
        );
        assert_eq!(
            store.chain_children(std::slice::from_ref(&root.id), 3).await.unwrap().len(),
            3,
            "at most the limit"
        );
        assert_eq!(store.chain_children(&[MessageId::random()], 10).await.unwrap(), vec![]);
        assert_eq!(store.chain_children(&[], 10).await.unwrap(), vec![]);
    })
    .await
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
        let entries = t.store().chain_entries(std::slice::from_ref(&child.id)).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].caused_by(), Some(&root.id));
        let children = t.store().chain_children(std::slice::from_ref(&root.id), 10).await.unwrap();
        assert_eq!(children.iter().map(|c| c.id().clone()).collect::<Vec<_>>(), vec![child.id.clone()]);
        let c = t.claim_one_types(&[&OTHER]).await;
        t.commit_ok(done(&c)).await;
        if t.fixture.supports_reopen() {
            t.fixture.reopen().await;
        }
        let entries = t.store().chain_entries(std::slice::from_ref(&child.id)).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(live_envelope(&entries[0]).caused_by.as_ref(), Some(&root.id));
        let children = t.store().chain_children(std::slice::from_ref(&root.id), 10).await.unwrap();
        assert_eq!(children.iter().map(|c| c.id().clone()).collect::<Vec<_>>(), vec![child.id.clone()]);
    })
    .await
}

/// chain - outcomes past retention are not part of a chain
pub async fn chain_outcomes_past_retention_are_not_part_of_a_chain<B: FixtureFactory>(backend: &B) {
    store_test_with(backend, backend.tick() * 10, DEFAULT_TIMEOUT, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, None).await;
        let b = t.new_actor(&OTHER, None).await;
        let root = t.send(&a).await;
        let child = caused(&b, &root.id, Some(&a));
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest { sends: vec![child.clone()], ..done(&c) }).await;
        let c = t.claim_one_types(&[&OTHER]).await;
        t.commit_ok(done(&c)).await;
        t.advance_ticks(11).await;
        assert_eq!(store.chain_entries(&[root.id.clone(), child.id.clone()]).await.unwrap(), vec![]);
        assert_eq!(store.chain_children(std::slice::from_ref(&root.id), 10).await.unwrap(), vec![]);
    })
    .await
}
