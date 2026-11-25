//! A watch of an actor that must exist (ADR 0063): a commit checks the target of each `existing` watch change where it
//! applies the watches, sets the watch when the actor is there and otherwise writes one `Stopped` of it (cause
//! `deleted`, `by` null, the turn's message as `causedBy`) to the committing actor, and leaves the pair without a watch.

use crate::address::{ActorAddress, TypeName};
use crate::store::{
    ActorCreate, Backend, Claim, CommitRequest, Control, ControlEffect, DeleteResult, Message, MessageId,
    SetWatchResult, StateChange, StoreError,
};
use crate::watch::{StopCause, Stopped, WatchChange, WatchTarget};
use serde_json::json;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

fn event(id: &str) -> MessageId {
    MessageId::new(id).expect("a valid id")
}

fn target(text: &str) -> WatchTarget {
    WatchTarget::parse(text).expect("a valid target")
}

/// The change of `Turn::watch_existing(on)`.
fn existing(on: &ActorAddress) -> WatchChange {
    WatchChange { target: WatchTarget::Actor(on.clone()), set: true, existing: true }
}

fn set(on: &ActorAddress) -> WatchChange {
    WatchChange { target: WatchTarget::Actor(on.clone()), set: true, existing: false }
}

fn unset(on: &ActorAddress) -> WatchChange {
    WatchChange { target: WatchTarget::Actor(on.clone()), set: false, existing: false }
}

/// A new committing actor `key`, of a type of its own so that no other case's message is claimed for it, and a claim
/// of a message to it.
async fn turn<F: StoreFixture>(t: &Env<F>, key: &str) -> Claim {
    let ty = type_name(&format!("watcher.{key}"));
    let actor = t.new_actor_named(&ty, key, json!({})).await;
    again(t, &actor).await
}

/// A claim of a new message to `actor`, the only one of its type with a due message.
async fn again<F: StoreFixture>(t: &Env<F>, actor: &ActorAddress) -> Claim {
    t.send(actor).await;
    let ty: &TypeName = actor.type_name();
    let c = t.claim_one_types(&[ty]).await;
    assert_eq!(&c.actor, actor);
    c
}

/// The stored watches of `watcher` as target text.
async fn watches_of(store: &dyn Backend, watcher: &ActorAddress) -> Vec<String> {
    let page = store.watches(Some(watcher), None, None, 100).await.unwrap();
    page.items.into_iter().map(|w| w.target.to_string()).collect()
}

/// The notifications pending at `address`.
async fn notifications<F: StoreFixture>(t: &Env<F>, address: &ActorAddress) -> Vec<Message> {
    t.pending(address).await.into_iter().filter(|m| m.notification).collect()
}

fn stopped_of(m: &Message) -> Stopped {
    assert_eq!(m.message_type, "Stopped", "{m:?}");
    serde_json::from_value(m.payload.clone()).expect("a Stopped")
}

/// The `Stopped` of an absent `actor`.
fn absent(actor: &ActorAddress) -> Stopped {
    Stopped { actor: actor.clone(), cause: StopCause::Deleted, by: None }
}

/// watch_existing - a present target is watched, and a later delete writes one `Stopped`
pub async fn watch_existing_of_a_present_actor_sets_the_watch_and_a_later_delete_writes_one_stopped<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let c = turn(t, "p").await;
        let p = c.actor.clone();
        t.commit_ok(CommitRequest { event: event("e1"), watches: vec![existing(&x)], ..done(&c) }).await;
        assert_eq!(watches_of(&*store, &p).await, [x.to_string()], "a plain watch");
        assert!(notifications(t, &p).await.is_empty(), "nothing is written while it is there");

        assert_matches!(store.delete(&x, &event("e2")).await.unwrap(), DeleteResult::Deleted { .. });
        let got = notifications(t, &p).await;
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["e2.0"], "one, from the delete");
        assert_eq!(stopped_of(&got[0]), absent(&x));
        assert_eq!(got[0].caused_by, None, "a delete from outside has no causedBy");
        assert_eq!(watches_of(&*store, &p).await, [x.to_string()], "the watch survives the delete (ADR 0023)");
    })
    .await
}

/// watch_existing - an absent target gives one `Stopped` with every field, named after the event, and no watch; a
/// stored watch of the pair is removed
pub async fn watch_existing_of_an_absent_actor_writes_one_stopped_and_leaves_no_watch<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let c = turn(t, "p").await;
        let p = c.actor.clone();
        let gone = ActorAddress::of(&TYPE, "gone").unwrap();
        assert_eq!(store.watch(&WatchTarget::Actor(gone.clone()), &p).await.unwrap(), SetWatchResult::Created);
        t.commit_ok(CommitRequest { event: event("e1"), watches: vec![existing(&gone)], ..done(&c) }).await;

        let got = notifications(t, &p).await;
        assert_eq!(got.len(), 1, "{got:?}");
        let m = &got[0];
        assert_eq!(m.id.as_str(), "e1.0", "named after the event");
        assert_eq!(stopped_of(m), absent(&gone));
        assert_eq!(m.payload["by"], json!(null), "always written");
        assert_eq!(m.from.as_ref(), Some(&gone), "sent by the absent actor");
        assert_eq!(m.caused_by.as_ref(), Some(&c.message.id), "caused by the turn's message");
        assert!(m.notification, "marked");
        assert!(m.retention.is_none() && m.correlation_id.is_none() && m.headers.is_empty());
        assert!(watches_of(&*store, &p).await.is_empty(), "the older watch of the pair is removed too");

        // A create of that address later is not watched.
        t.new_actor_named(&TYPE, "gone", json!({})).await;
        assert_matches!(store.delete(&gone, &event("e2")).await.unwrap(), DeleteResult::Deleted { .. });
        assert_eq!(notifications(t, &p).await.len(), 1, "no Stopped from the later delete");

        // The handled Stopped ends like any notification.
        let c = again(t, &p).await;
        assert_eq!(c.message.id.as_str(), "e1.0", "the Stopped wakes the watcher first");
        t.commit_ok(done(&c)).await;
    })
    .await
}

/// watch_existing - a commit refuses an existing watch change on a type or every type, or without set, as invalid
pub async fn watch_existing_of_a_type_or_without_set_is_invalid<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let c = turn(t, "p").await;
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let bad = [
            WatchChange { target: target("billing"), set: true, existing: true },
            WatchChange { target: target("*"), set: true, existing: true },
            WatchChange { target: WatchTarget::Actor(x.clone()), set: false, existing: true },
        ];
        for change in bad {
            let request = CommitRequest { watches: vec![set(&x), change.clone()], ..done(&c) };
            assert_matches!(t.store().commit(request).await, Err(StoreError::Invalid(_)), "{change:?}");
        }
        assert!(watches_of(&*store, &c.actor).await.is_empty(), "nothing is set");
        t.commit_ok(done(&c)).await; // the lease is still held
    })
    .await
}

/// watch_existing - a delete of the target in the same turn finds the watch set and writes its own `Stopped`
pub async fn watch_existing_with_a_delete_of_the_target_in_the_same_turn_writes_the_stopped_of_the_delete<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let c = turn(t, "p").await;
        let p = c.actor.clone();
        let delete = Control { effect: ControlEffect::Delete, actor: x.clone() };
        let request =
            CommitRequest { event: event("e1"), watches: vec![existing(&x)], controls: vec![delete], ..done(&c) };
        t.commit_ok(request).await;
        let got = notifications(t, &p).await;
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(stopped_of(&got[0]), Stopped { actor: x.clone(), cause: StopCause::Deleted, by: Some(p.clone()) });
        assert_eq!(t.store().get(&x).await.unwrap(), None);
    })
    .await
}

/// watch_existing - an actor created in the same turn is not there yet at the check: a `Stopped` and no watch
pub async fn watch_existing_of_an_actor_created_in_the_same_turn_writes_a_stopped<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let c = turn(t, "p").await;
        let p = c.actor.clone();
        let x = ActorAddress::of(&TYPE, "x").unwrap();
        t.register_type(&TYPE).await;
        let create = ActorCreate { address: x.clone(), state: json!({}), child: false, pinned_to: None };
        let request =
            CommitRequest { event: event("e1"), watches: vec![existing(&x)], creates: vec![create], ..done(&c) };
        t.commit_ok(request).await;
        assert!(store.get(&x).await.unwrap().is_some(), "created");
        let got = notifications(t, &p).await;
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(stopped_of(&got[0]), absent(&x));
        assert!(watches_of(&*store, &p).await.is_empty());
    })
    .await
}

/// watch_existing - the watcher itself is there: a watch of itself
pub async fn watch_existing_of_the_watcher_itself_sets_the_watch<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let c = turn(t, "p").await;
        let p = c.actor.clone();
        t.commit_ok(CommitRequest { watches: vec![existing(&p)], ..done(&c) }).await;
        assert_eq!(watches_of(&*store, &p).await, [p.to_string()]);
        assert!(notifications(t, &p).await.is_empty());
    })
    .await
}

/// watch_existing - the watch changes of one turn apply in the order of the calls, each existing one checked at its
/// place: then an unwatch, after a watch, and twice give one `Stopped` and no watch; a watcher that stops gets nothing
pub async fn watch_existing_applies_in_the_order_of_the_calls_and_two_calls_give_one_stopped<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let gone = ActorAddress::of(&TYPE, "gone").unwrap();
        let orders = [
            ("a", vec![existing(&gone), unset(&gone)]),
            ("b", vec![set(&gone), existing(&gone)]),
            ("c", vec![existing(&gone), existing(&gone)]),
        ];
        for (key, changes) in orders {
            let c = turn(t, key).await;
            t.commit_ok(CommitRequest { event: event(&format!("e-{key}")), watches: changes, ..done(&c) }).await;
            let got = notifications(t, &c.actor).await;
            assert_eq!(got.iter().map(|m| m.id.to_string()).collect::<Vec<_>>(), [format!("e-{key}.0")], "{key}");
            assert_eq!(stopped_of(&got[0]), absent(&gone), "{key}");
            assert!(watches_of(&*store, &c.actor).await.is_empty(), "{key}: no watch of the pair");
        }

        // A watch after it stands: the last call for a target stands.
        let c = turn(t, "d").await;
        t.commit_ok(CommitRequest { watches: vec![existing(&gone), set(&gone)], ..done(&c) }).await;
        assert_eq!(notifications(t, &c.actor).await.len(), 1);
        assert_eq!(watches_of(&*store, &c.actor).await, [gone.to_string()]);

        // A watcher that stops in the same turn gets nothing.
        let c = turn(t, "e").await;
        let request = CommitRequest { watches: vec![existing(&gone)], state: StateChange::Stop, ..done(&c) };
        t.commit_ok(request).await;
        assert_eq!(store.get(&c.actor).await.unwrap(), None);
        assert!(t.pending(&c.actor).await.is_empty(), "nothing is written for a removed watcher");
    })
    .await
}

/// watch_existing - the `Stopped` of an absent target is numbered with the other notifications of the commit's event
pub async fn watch_existing_numbers_its_stopped_with_the_other_notifications_of_the_event<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let c = turn(t, "p").await;
        let p = c.actor.clone();
        let other = t.new_actor_named(&OTHER, "y", json!({})).await;
        let w = t.new_actor_named(&type_name("w"), "a", json!({})).await;
        assert_eq!(store.watch(&WatchTarget::Actor(other.clone()), &w).await.unwrap(), SetWatchResult::Created);
        assert_eq!(store.watch(&WatchTarget::Actor(other.clone()), &p).await.unwrap(), SetWatchResult::Created);
        let gone = ActorAddress::of(&TYPE, "gone").unwrap();
        let delete = Control { effect: ControlEffect::Delete, actor: other.clone() };
        let request =
            CommitRequest { event: event("e1"), watches: vec![existing(&gone)], controls: vec![delete], ..done(&c) };
        t.commit_ok(request).await;
        // In the order of the actor they are about and then of the watcher: conformance/gone for p, then other/y for
        // w/a and for watcher.p/p.
        let at_p = notifications(t, &p).await;
        let ids: Vec<(&str, ActorAddress)> = at_p.iter().map(|m| (m.id.as_str(), stopped_of(m).actor)).collect();
        assert_eq!(ids, [("e1.0", gone.clone()), ("e1.2", other.clone())]);
        let at_w = notifications(t, &w).await;
        assert_eq!(at_w.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["e1.1"]);
        assert_eq!(stopped_of(&at_p[0]), absent(&gone));
        assert_eq!(stopped_of(&at_p[1]).by, Some(p.clone()), "the delete's Stopped names the turn");
    })
    .await
}

/// watch_existing - a delete of the target racing the commit of a `watch_existing` gives exactly one `Stopped`, with
/// the watcher's address before and after the target's
pub async fn watch_existing_racing_a_delete_of_the_target_gives_exactly_one_stopped<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let before = type_name("a.target");
        let after = type_name("z.target");
        for round in 0..40 {
            let ty = if round % 2 == 0 { &before } else { &after };
            let x = t.new_actor_named(ty, &format!("x{round}"), json!({})).await;
            let c = turn(t, &format!("p{round}")).await;
            let p = c.actor.clone();
            let results = t
                .concurrently(2, |i, peer| {
                    let (c, x) = (c.clone(), x.clone());
                    async move {
                        if i == 0 {
                            let request = CommitRequest { watches: vec![existing(&x)], ..done(&c) };
                            Err(peer.commit(request).await.unwrap())
                        } else {
                            Ok(peer.delete(&x, &MessageId::random()).await.unwrap())
                        }
                    }
                })
                .await;
            for result in results {
                match result {
                    Err(commit) => assert_matches!(commit, crate::store::CommitResult::Committed { .. }, "{round}"),
                    Ok(deleted) => assert_matches!(deleted, DeleteResult::Deleted { .. }, "round {round}"),
                }
            }
            let got = notifications(t, &p).await;
            assert_eq!(got.len(), 1, "round {round}: {got:?}");
            assert_eq!(stopped_of(&got[0]), absent(&x), "round {round}");
            assert_matches!(store.delete(&p, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. });
        }
    })
    .await
}
