//! The change feed (ADR 0034) on every store: which committed writes report their actors, once per transaction and
//! after the commit (a read after a hint sees it, S3), and which report nothing: mailbox, lease, watch, registry, node
//! and sweep work, and the effects of a commit that was refused, rolled back or fenced (S2). An absence is shown with a
//! later marker, never by waiting.

use std::time::Duration;

use futures::StreamExt;
use serde_json::json;

use crate::address::{ActorAddress, TypeName};
use crate::change::{ActorChange, ActorChanges};
use crate::store::{
    ActorCreate, CommitRequest, CommitResult, Control, ControlEffect, CreateResult, DeleteResult, Delivery, MessageId,
    OutgoingMessage, SetWatchResult, StateChange, StoreError, TurnOutcome,
};
use crate::watch::{WatchChange, WatchTarget};

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

/// The address of `key` of [`TYPE`].
fn at(key: &str) -> ActorAddress {
    ActorAddress::of(&TYPE, key).expect("a valid address")
}

/// The address of `key` of [`OTHER`].
fn other(key: &str) -> ActorAddress {
    ActorAddress::of(&OTHER, key).expect("a valid address")
}

fn changed(address: &ActorAddress) -> ActorChange {
    ActorChange::changed(address.clone())
}

fn control(effect: ControlEffect, actor: &ActorAddress) -> Control {
    Control { effect, actor: actor.clone() }
}

fn child(address: &ActorAddress) -> ActorCreate {
    ActorCreate { address: address.clone(), state: json!({}), child: true }
}

fn independent(address: &ActorAddress, state: serde_json::Value) -> ActorCreate {
    ActorCreate { address: address.clone(), state, child: false }
}

fn to(address: &ActorAddress) -> OutgoingMessage {
    OutgoingMessage::new(address.clone(), "Msg", json!({}))
}

/// A stream of every address of the store under test, its first `Resync` taken.
async fn subscribe<F: StoreFixture>(t: &Env<F>) -> ActorChanges {
    t.register_type(&TYPE).await;
    t.register_type(&OTHER).await;
    let mut changes = ActorChanges::new(t.store().changes(), "");
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync));
    changes
}

async fn next(changes: &mut ActorChanges) -> Option<ActorChange> {
    tokio::time::timeout(Duration::from_secs(30), changes.next()).await.expect("an item in time")
}

/// Creates a marker actor and returns the items before its hint: what the writes before it reported.
async fn until_marker<F: StoreFixture>(t: &Env<F>, changes: &mut ActorChanges) -> Vec<ActorChange> {
    let marker = t.new_actor(&TYPE, json!({})).await;
    let mut seen = Vec::new();
    loop {
        match next(changes).await {
            Some(ActorChange::Changed { address, .. }) if address == marker => return seen,
            Some(change) => seen.push(change),
            None => panic!("the stream ended after {seen:?}"),
        }
    }
}

/// Sends a message to `actor`, the only due instance of its type, and claims it.
async fn turn_of<F: StoreFixture>(t: &Env<F>, actor: &ActorAddress) -> crate::store::Claim {
    t.send(actor).await;
    let claim = t.claim_one_types(&[actor.type_name()]).await;
    assert_eq!(&claim.actor, actor, "the only due instance of its type");
    claim
}

/// Creates `address` from outside and takes its hint.
async fn create_seen<F: StoreFixture>(t: &Env<F>, changes: &mut ActorChanges, address: &ActorAddress) {
    assert_eq!(t.store().create(address, json!({})).await.unwrap(), CreateResult::Created);
    assert_eq!(next(changes).await, Some(changed(address)));
}

pub async fn changes_creates_pauses_resumes_and_deletes_from_outside_are_reported_after_they_commit<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let mut changes = subscribe(t).await;
        let store = t.store();
        let a = at("a");
        assert_eq!(store.create(&a, json!({"n": 1})).await.unwrap(), CreateResult::Created);
        assert_eq!(next(&mut changes).await, Some(changed(&a)));
        assert_eq!(store.get(&a).await.unwrap().expect("committed before the hint").state, json!({"n": 1}));
        assert!(matches!(store.create(&a, json!({"n": 2})).await.unwrap(), CreateResult::Exists(_)));
        assert!(store.set_paused(&a, true).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&a)));
        assert!(store.get(&a).await.unwrap().unwrap().paused);
        assert!(store.set_paused(&a, false).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&a)));
        assert!(!store.get(&a).await.unwrap().unwrap().paused);
        assert!(!store.set_paused(&at("absent"), true).await.unwrap());
        assert_eq!(until_marker(t, &mut changes).await, vec![], "an idempotent create and an absent actor");

        // A tree made by a commit, removed by an operator delete: every actor of it, once.
        let p = at("p");
        create_seen(t, &mut changes, &p).await;
        let claim = turn_of(t, &p).await;
        let mut commit = done(&claim);
        commit.creates = vec![child(&at("p-1")), child(&at("p-2"))];
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&at("p-1"))));
        assert_eq!(next(&mut changes).await, Some(changed(&at("p-2"))));
        assert!(matches!(store.delete(&p, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. }));
        for removed in [&p, &at("p-1"), &at("p-2")] {
            assert_eq!(next(&mut changes).await, Some(changed(removed)));
        }
        assert_eq!(store.get(&at("p-2")).await.unwrap(), None);
        assert_eq!(store.delete(&p, &MessageId::random()).await.unwrap(), DeleteResult::Absent);
        assert_eq!(until_marker(t, &mut changes).await, vec![], "a delete of an absent actor");
    })
    .await;
}

pub async fn changes_a_turn_reports_its_new_state_even_an_equal_one_its_creates_controls_and_restarts_once_each<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let mut changes = subscribe(t).await;
        let store = t.store();
        let a = at("t");
        let free = other("free");
        assert_eq!(store.create(&a, json!({"n": 1})).await.unwrap(), CreateResult::Created);
        assert_eq!(next(&mut changes).await, Some(changed(&a)));

        // The same state, a child, an independent actor and a send to the child: three addresses, once each.
        let claim = turn_of(t, &a).await;
        let mut commit = done(&claim);
        commit.state = StateChange::NewState(json!({"n": 1}));
        commit.creates = vec![child(&other("kid")), independent(&free, json!({"life": 1})), child(&other("kid"))];
        commit.sends = vec![to(&other("kid"))];
        committed(store.commit(commit).await.unwrap());
        for address in [&a, &free, &other("kid")] {
            assert_eq!(next(&mut changes).await, Some(changed(address)), "in address order");
        }
        assert_eq!(store.get(&a).await.unwrap().unwrap().version, 1, "an equal state is a write");

        // A restart (delete and create of one address) is one address, read in its new life.
        let claim = turn_of(t, &a).await;
        let mut commit = done(&claim);
        commit.controls = vec![control(ControlEffect::Delete, &free)];
        commit.creates = vec![independent(&free, json!({"life": 2}))];
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&free)));
        let record = store.get(&free).await.unwrap().unwrap();
        assert_eq!((record.state, record.version), (json!({"life": 2}), 0));
        // A restart of an actor that is not there creates it; pauses and resumes of a commit count like the calls.
        let claim = turn_of(t, &a).await;
        let mut commit = done(&claim);
        commit.controls = vec![control(ControlEffect::Delete, &other("new")), control(ControlEffect::Pause, &free)];
        commit.creates = vec![independent(&other("new"), json!({}))];
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&free)));
        assert_eq!(next(&mut changes).await, Some(changed(&other("new"))));
        assert!(store.get(&free).await.unwrap().unwrap().paused);
        let claim = turn_of(t, &a).await;
        let mut commit = done(&claim);
        commit.controls = vec![control(ControlEffect::Resume, &free), control(ControlEffect::Pause, &other("gone"))];
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&free)));
        assert_eq!(until_marker(t, &mut changes).await, vec![], "a control of an absent actor");
    })
    .await;
}

pub async fn changes_a_stop_and_a_delete_by_a_commit_report_every_removed_actor_with_its_descendants<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let mut changes = subscribe(t).await;
        let store = t.store();
        let s = at("s");
        create_seen(t, &mut changes, &s).await;
        let claim = turn_of(t, &s).await;
        let mut commit = done(&claim);
        commit.creates = vec![child(&at("s-1")), child(&at("s-2"))];
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&at("s-1"))));
        assert_eq!(next(&mut changes).await, Some(changed(&at("s-2"))));
        let claim = turn_of(t, &at("s-1")).await;
        let mut commit = done(&claim);
        commit.creates = vec![child(&at("s-1-x"))];
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&at("s-1-x"))));

        let claim = turn_of(t, &s).await;
        let mut commit = done(&claim);
        commit.state = StateChange::Stop;
        committed(store.commit(commit).await.unwrap());
        for removed in ["s", "s-1", "s-1-x", "s-2"] {
            assert_eq!(next(&mut changes).await, Some(changed(&at(removed))));
        }
        assert_eq!(store.get(&at("s-1-x")).await.unwrap(), None);

        // A delete by a commit: the named actor and its descendants.
        let (d, killer) = (other("d"), at("killer"));
        create_seen(t, &mut changes, &d).await;
        create_seen(t, &mut changes, &killer).await;
        let claim = turn_of(t, &d).await;
        let mut commit = done(&claim);
        commit.creates = vec![child(&other("d-1"))];
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&other("d-1"))));
        let claim = turn_of(t, &killer).await;
        let mut commit = done(&claim);
        commit.controls = vec![control(ControlEffect::Delete, &d)];
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(changed(&d)));
        assert_eq!(next(&mut changes).await, Some(changed(&other("d-1"))));
        assert_eq!(until_marker(t, &mut changes).await, vec![]);
    })
    .await;
}

pub async fn changes_mailbox_lease_watch_registry_node_and_sweep_work_and_a_turn_that_keeps_its_state_report_nothing<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let mut changes = subscribe(t).await;
        let store = t.store();
        let (a, w) = (at("m"), other("w"));
        create_seen(t, &mut changes, &a).await;
        create_seen(t, &mut changes, &w).await;
        t.register_type(&TypeName::parse("extra").unwrap()).await;
        let later = t.send_at(&a, Delivery::After(Duration::from_secs(3600))).await;
        store.deliver_now(&later.id).await.unwrap();
        let pending = t.send_at(&a, Delivery::After(Duration::from_secs(3600))).await;
        store.cancel(&pending.id).await.unwrap();
        let claim = t.claim_one_long().await;
        store.renew(&claim.actor, claim.epoch, LONG_LEASE).await.unwrap();
        assert!(store.request_cancel(&claim.actor).await.unwrap());
        store.release(&claim.actor, claim.epoch).await.unwrap();
        // A turn that keeps its state, sends, cancels and watches.
        let claim = t.claim_one_long().await;
        let cancelled = t.send_at(&a, Delivery::After(Duration::from_secs(3600))).await;
        let mut commit = done(&claim);
        commit.sends = vec![to(&w)];
        commit.cancels = vec![cancelled.id];
        commit.watches = vec![WatchChange { target: WatchTarget::parse("other/w").unwrap(), set: true }];
        committed(store.commit(commit).await.unwrap());
        // A rescheduled message, a purge, watches from outside, nodes and a sweep.
        t.send(&a).await;
        let claim = t.claim_one_long().await;
        let rescheduled = TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "again".into() };
        committed(store.commit(ending(&claim, rescheduled)).await.unwrap());
        t.send_at(&a, Delivery::After(Duration::from_secs(3600))).await;
        store.purge(&a).await.unwrap();
        let target = WatchTarget::parse("conformance/m").unwrap();
        store.watch(&target, &w).await.unwrap();
        store.unwatch(&target, &w).await.unwrap();
        store.heartbeat_node(&NODE_A, &type_set(&[&TYPE]), &Default::default()).await.unwrap();
        store.remove_node(&NODE_A).await.unwrap();
        store.sweep_expired().await.unwrap();
        assert_eq!(until_marker(t, &mut changes).await, vec![]);
    })
    .await;
}

pub async fn changes_refused_rolled_back_and_fenced_commits_and_a_failed_turn_report_nothing<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let mut changes = subscribe(t).await;
        let store = t.store();
        let (a, w) = (at("r"), other("w"));
        create_seen(t, &mut changes, &a).await;
        create_seen(t, &mut changes, &w).await;
        let target = WatchTarget::parse("conformance/r").unwrap();
        assert_eq!(store.watch(&target, &w).await.unwrap(), SetWatchResult::Created);
        let claim = turn_of(t, &a).await;
        let effects = |mut commit: CommitRequest| {
            commit.state = StateChange::NewState(json!({"n": 9}));
            commit.creates = vec![independent(&at("never"), json!({}))];
            commit.controls = vec![control(ControlEffect::Pause, &w)];
            commit
        };
        // Refused: a send to an absent actor.
        let mut refused = effects(done(&claim));
        refused.sends = vec![to(&at("absent"))];
        assert!(matches!(store.commit(refused).await.unwrap(), CommitResult::Rejected(_)));
        // Fenced: an epoch that is not the current one.
        let mut stale = effects(done(&claim));
        stale.epoch -= 1;
        assert_eq!(store.commit(stale).await.unwrap(), CommitResult::LeaseLost);
        // Invalid: a pause and a resume of one actor.
        let mut invalid = effects(done(&claim));
        invalid.controls.push(control(ControlEffect::Resume, &w));
        assert!(matches!(store.commit(invalid).await, Err(StoreError::Invalid(_))));
        // A failed turn commits no effect of its handler; the notification it writes for the watcher is mailbox work.
        committed(store.commit(ending(&claim, TurnOutcome::Failed("boom".into()))).await.unwrap());
        assert_eq!(t.pending(&w).await.len(), 1, "the watcher was told");
        assert_eq!(store.get(&at("never")).await.unwrap(), None);
        assert_eq!(until_marker(t, &mut changes).await, vec![]);
    })
    .await;
}

pub async fn changes_a_transaction_of_more_than_100_actors_is_one_resync<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let mut changes = subscribe(t).await;
        let store = t.store();
        let big = at("big");
        create_seen(t, &mut changes, &big).await;
        let claim = turn_of(t, &big).await;
        let mut commit = done(&claim);
        commit.creates = (0..101).map(|i| child(&at(&format!("big-{i:03}")))).collect();
        committed(store.commit(commit).await.unwrap());
        assert_eq!(next(&mut changes).await, Some(ActorChange::Resync), "101 creates");
        assert!(store.get(&at("big-100")).await.unwrap().is_some(), "committed before the resync");
        assert!(matches!(store.delete(&big, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. }));
        assert_eq!(next(&mut changes).await, Some(ActorChange::Resync), "102 removals");
        assert_eq!(until_marker(t, &mut changes).await, vec![]);
    })
    .await;
}
