//! Control effects (ADR 0024): a commit pauses, resumes, purges and deletes other actors, and pauses, resumes and
//! purges its own, together with its other effects or not at all; each effect does what the call from outside a turn
//! does, and nothing to an actor that is not there; deletes come before creates, so a commit can restart an actor.

use std::sync::LazyLock;

use crate::address::{ActorAddress, TypeName};
use crate::store::{
    ActorCreate, Backend, CommitRequest, CommitResult, Control, ControlEffect, CreateResult, DeleteResult, Message,
    MessageId, OutcomeStatus, OutgoingMessage, RenewResult, SetWatchResult, StateChange, StoreError, TurnOutcome,
};
use crate::watch::{StopCause, Stopped, WatchChange, WatchTarget};
use serde_json::{Value, json};

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

/// The type of the watchers: never claimed by the cases, so their notifications stay in their mailboxes.
static W: LazyLock<TypeName> = LazyLock::new(|| type_name("w"));

/// How many times a race of the suite runs.
const ROUNDS: usize = 50;

/// The address of `key` of [`TYPE`], the type of the committing actors.
fn at(key: &str) -> ActorAddress {
    ActorAddress::of(&TYPE, key).expect("a valid address")
}

/// The address of `key` of [`OTHER`], the type of the controlled actors.
fn other(key: &str) -> ActorAddress {
    ActorAddress::of(&OTHER, key).expect("a valid address")
}

fn control(effect: ControlEffect, actor: &ActorAddress) -> Control {
    Control { effect, actor: actor.clone() }
}

fn pause(actor: &ActorAddress) -> Control {
    control(ControlEffect::Pause, actor)
}

fn resume(actor: &ActorAddress) -> Control {
    control(ControlEffect::Resume, actor)
}

fn purge(actor: &ActorAddress) -> Control {
    control(ControlEffect::Purge, actor)
}

fn delete(actor: &ActorAddress) -> Control {
    control(ControlEffect::Delete, actor)
}

fn independent(address: &ActorAddress, state: Value) -> ActorCreate {
    ActorCreate { address: address.clone(), state, child: false }
}

fn child(address: &ActorAddress, state: Value) -> ActorCreate {
    ActorCreate { address: address.clone(), state, child: true }
}

fn to(address: &ActorAddress) -> OutgoingMessage {
    OutgoingMessage::new(address.clone(), "Msg", json!({}))
}

fn event(id: &str) -> MessageId {
    MessageId::new(id).expect("a valid id")
}

async fn status_of(store: &dyn Backend, id: &MessageId) -> Option<OutcomeStatus> {
    outcome_record(store, id).await.map(|o| o.status)
}

/// Sends a message to `actor`, the only due instance of its type, claims it and returns the claim's commit with
/// `request` merged in by the caller.
async fn turn_of<F: StoreFixture>(t: &Env<F>, actor: &ActorAddress) -> crate::store::Claim {
    t.send(actor).await;
    let c = t.claim_one_types(&[actor.type_name()]).await;
    assert_eq!(&c.actor, actor, "the only due instance of its type");
    c
}

/// Runs a turn of `parent` that creates `kids` as its children.
async fn with_children<F: StoreFixture>(t: &Env<F>, parent: &ActorAddress, kids: &[&ActorAddress]) {
    let c = turn_of(t, parent).await;
    t.commit_ok(CommitRequest { creates: kids.iter().map(|k| child(k, json!({}))).collect(), ..done(&c) }).await;
}

async fn watcher<F: StoreFixture>(t: &Env<F>, key: &str) -> ActorAddress {
    t.new_actor_named(&W, key, json!({})).await
}

async fn watch(store: &dyn Backend, on: &ActorAddress, watcher: &ActorAddress) {
    let result = store.watch(&WatchTarget::Actor(on.clone()), watcher).await.unwrap();
    assert_eq!(result, SetWatchResult::Created, "{on} by {watcher}");
}

/// The notifications pending at `address`.
async fn notifications<F: StoreFixture>(t: &Env<F>, address: &ActorAddress) -> Vec<Message> {
    t.pending(address).await.into_iter().filter(|m| m.notification).collect()
}

fn stopped_of(m: &Message) -> Stopped {
    assert_eq!(m.message_type, "Stopped", "{m:?}");
    serde_json::from_value(m.payload.clone()).expect("a Stopped")
}

/// The `Stopped` messages pending at `watcher`, by the actor they are about.
async fn stopped_at<F: StoreFixture>(t: &Env<F>, watcher: &ActorAddress) -> Vec<(ActorAddress, StopCause)> {
    notifications(t, watcher).await.iter().map(stopped_of).map(|s| (s.actor, s.cause)).collect()
}

/// control - a pause, a resume and a purge by a commit do what the calls from outside do, also to the committing actor
pub async fn control_a_pause_a_resume_and_a_purge_by_a_commit_do_what_the_calls_from_outside_do_also_to_the_committing_actor<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({"n": 1})).await;
        let running = t.send(&x).await;
        let waiting = [t.send(&x).await, t.send(&x).await];
        let on_x = single(t.claim_with(&[&OTHER], &NODE_B, LONG_LEASE, 10).await);
        assert_eq!(on_x.message.id, running.id, "a turn runs on x");

        let c = turn_of(t, &p).await;
        t.commit_ok(CommitRequest { controls: vec![pause(&x), purge(&x)], ..done(&c) }).await;
        for m in &waiting {
            assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged), "{}", m.id);
        }
        assert_eq!(ids(&t.pending(&x).await), std::slice::from_ref(&running.id), "the message being handled stays");
        let record = store.get(&x).await.unwrap().expect("x stays");
        assert!(record.paused, "paused");
        assert_eq!(record.state, json!({"n": 1}), "its state stays");
        t.commit_ok(done(&on_x)).await; // the running turn finishes
        assert_eq!(status_of(&*store, &running.id).await, Some(OutcomeStatus::Done));
        let later = t.send(&x).await;
        assert!(t.claim_types(&[&OTHER]).await.is_empty(), "no new turn while paused; messages queue up");

        let c = turn_of(t, &p).await;
        t.commit_ok(CommitRequest { controls: vec![resume(&x)], ..done(&c) }).await;
        assert!(!store.get(&x).await.unwrap().expect("x").paused, "resumed");
        assert_eq!(t.claim_one_types(&[&OTHER]).await.message.id, later.id, "claimed again");

        // The committing actor itself: the turn's own message ends `done`, the others are purged.
        let c = turn_of(t, &p).await;
        let own = [t.send(&p).await, t.send(&p).await];
        t.commit_ok(CommitRequest { controls: vec![purge(&p), pause(&p)], ..done(&c) }).await;
        assert_eq!(status_of(&*store, &c.message.id).await, Some(OutcomeStatus::Done));
        for m in &own {
            assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged), "{}", m.id);
        }
        assert!(store.get(&p).await.unwrap().expect("p").paused);
        let next = t.send(&p).await;
        assert!(t.claim().await.is_empty(), "the next message waits");
        assert!(store.set_paused(&p, false).await.unwrap());
        assert_eq!(t.claim_one().await.message.id, next.id);
    })
    .await
}

/// control - a delete by a commit removes the actor, its mailbox and its descendants, and voids a turn that runs on one
/// of them
pub async fn control_a_delete_by_a_commit_removes_the_actor_its_mailbox_and_its_descendants_and_voids_a_turn<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({})).await;
        let kept = t.new_actor_named(&OTHER, "kept", json!({"keep": 1})).await;
        let (y, z) = (other("y"), other("z"));
        with_children(t, &x, &[&y]).await;
        with_children(t, &y, &[&z]).await;
        let on_z = t.send(&z).await;
        let turn = single(t.claim_with(&[&OTHER], &NODE_B, LONG_LEASE, 10).await);
        assert_eq!(turn.message.id, on_z.id, "a turn runs on z");
        let pending = [t.send(&x).await, t.send(&y).await, t.send(&z).await];
        let left = t.send(&kept).await;

        let c = turn_of(t, &p).await;
        t.commit_ok(CommitRequest { controls: vec![delete(&x)], ..done(&c) }).await;
        for gone in [&x, &y, &z] {
            assert_eq!(store.get(gone).await.unwrap(), None, "{gone}");
            assert!(t.pending(gone).await.is_empty(), "{gone}");
        }
        for m in pending.iter().chain([&on_z]) {
            assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged), "{}", m.id);
        }
        let late = CommitRequest { state: StateChange::NewState(json!({"late": 1})), ..done(&turn) };
        assert_eq!(t.commit(late).await, CommitResult::LeaseLost, "the turn on z commits nothing");
        assert_eq!(store.get(&z).await.unwrap(), None);
        assert_eq!(store.get(&kept).await.unwrap().expect("stays").state, json!({"keep": 1}));
        assert_eq!(ids(&t.pending(&kept).await), [left.id], "no other actor changed");
        assert_eq!(status_of(&*store, &c.message.id).await, Some(OutcomeStatus::Done));
    })
    .await
}

/// control - a commit that is rejected, that lost its lease or that is refused applies no control effect
pub async fn control_a_commit_that_fails_or_is_rejected_applies_no_control_effect<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({})).await;
        let y = t.new_actor_named(&OTHER, "y", json!({})).await;
        let m = t.send(&x).await;
        let c = turn_of(t, &p).await;
        let controls = vec![pause(&x), purge(&x), delete(&y)];
        let rejected = CommitRequest { controls: controls.clone(), sends: vec![to(&other("absent"))], ..done(&c) };
        assert_matches!(t.commit(rejected).await, CommitResult::Rejected(_));
        let lost = CommitRequest { epoch: c.epoch + 1_000, controls: controls.clone(), ..done(&c) };
        assert_eq!(t.commit(lost).await, CommitResult::LeaseLost);
        let wrong = CommitRequest { message_id: event("not-the-claimed-one"), controls, ..done(&c) };
        assert_matches!(store.commit(wrong).await, Err(StoreError::Invalid(_)));
        assert!(!store.get(&x).await.unwrap().expect("x").paused, "not paused");
        assert_eq!(ids(&t.pending(&x).await), [m.id], "not purged");
        assert!(store.get(&y).await.unwrap().is_some(), "not deleted");
        t.commit_ok(done(&c)).await; // the lease is still held
    })
    .await
}

/// control - an effect on an actor that is not there does nothing and does not fail the commit, and a send to a deleted
/// actor still rejects it
pub async fn control_an_effect_on_an_actor_that_is_not_there_does_nothing_and_a_send_to_a_deleted_actor_still_rejects<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let q = t.new_actor_named(&OTHER, "q", json!({})).await;
        let stopped = t.new_actor_named(&OTHER, "stopped", json!({})).await;
        assert_matches!(store.delete(&stopped, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. });
        let absent = [other("a1"), other("a2"), other("a3")];
        let c = turn_of(t, &p).await;
        let committed = t
            .commit_ok(CommitRequest {
                state: StateChange::NewState(json!({"n": 2})),
                controls: vec![pause(&stopped), purge(&stopped), delete(&stopped), resume(&absent[0])],
                creates: vec![independent(&other("new"), json!({}))],
                sends: vec![to(&q)],
                ..done(&c)
            })
            .await;
        assert_eq!(committed.version, 1, "the other effects are applied");
        assert_eq!(committed.sent.len(), 1);
        assert!(store.get(&other("new")).await.unwrap().is_some());
        for a in absent.iter().chain([&stopped]) {
            assert_eq!(store.get(a).await.unwrap(), None, "{a} is not created");
        }

        let c = turn_of(t, &p).await;
        let result = t.commit(CommitRequest { controls: vec![delete(&q)], sends: vec![to(&q)], ..done(&c) }).await;
        let reason = expect_matches!(result, CommitResult::Rejected(reason) => reason);
        assert!(reason.contains(q.as_str()), "{reason}");
        assert!(store.get(&q).await.unwrap().is_some(), "nothing is applied");
        t.commit_ok(ending(&c, TurnOutcome::Failed(reason))).await; // the lease is still held
        assert_eq!(status_of(&*store, &c.message.id).await, Some(OutcomeStatus::Failed));
    })
    .await
}

/// control - a delete of an ancestor of the committing actor rejects the commit and removes nothing
pub async fn control_a_delete_of_an_ancestor_of_the_committing_actor_rejects_the_commit_and_removes_nothing<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let g = t.new_actor_named(&TYPE, "g", json!({})).await;
        let solo = t.new_actor_named(&OTHER, "solo", json!({})).await;
        let (p, k) = (at("p"), at("k"));
        with_children(t, &g, &[&p]).await;
        with_children(t, &p, &[&k]).await;
        let c = turn_of(t, &k).await;
        for ancestor in [&p, &g] {
            let request = CommitRequest { controls: vec![delete(&solo), delete(ancestor)], ..done(&c) };
            let reason =
                expect_matches!(t.commit(request).await, CommitResult::Rejected(reason) => reason, "{ancestor}");
            assert!(reason.contains(ancestor.as_str()), "the reason names the ancestor: {reason}");
        }
        for a in [&g, &p, &k, &solo] {
            assert!(store.get(a).await.unwrap().is_some(), "{a} stays");
        }
        t.commit_ok(done(&c)).await; // the lease is still held
    })
    .await
}

/// control - deletes come before creates (a restart, also as a child of the committing actor), creates before purges,
/// pauses and resumes, and those before sends
pub async fn control_deletes_come_before_creates_creates_before_purges_pauses_and_resumes_and_those_before_sends<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({"n": 1})).await;
        let old = t.send(&x).await;
        let c = turn_of(t, &p).await;
        let committed = t
            .commit_ok(CommitRequest {
                sends: vec![to(&x)],
                creates: vec![independent(&x, json!({"n": 2}))],
                controls: vec![delete(&x)],
                ..done(&c)
            })
            .await;
        let record = store.get(&x).await.unwrap().expect("a new x");
        assert_eq!((record.state, record.version, record.parent), (json!({"n": 2}), 0, None), "a restart");
        assert_eq!(status_of(&*store, &old.id).await, Some(OutcomeStatus::Purged));
        assert_eq!(ids(&t.pending(&x).await), ids(&committed.sent), "the new message is in the new mailbox");

        // A parent restarts its child: the new actor is its child, and the descendants of the old one are gone.
        let kids = type_name("kids");
        t.register_type(&kids).await;
        let (k, k1) = (ActorAddress::of(&kids, "k").unwrap(), ActorAddress::of(&kids, "k1").unwrap());
        with_children(t, &p, &[&k]).await;
        with_children(t, &k, &[&k1]).await;
        let c = turn_of(t, &p).await;
        let again =
            CommitRequest { controls: vec![delete(&k)], creates: vec![child(&k, json!({"again": 1}))], ..done(&c) };
        t.commit_ok(again).await;
        let record = store.get(&k).await.unwrap().expect("a new k");
        assert_eq!((record.state, record.parent), (json!({"again": 1}), Some(p.clone())));
        assert_eq!(store.get(&k1).await.unwrap(), None, "the old descendants are gone");

        // Creates before pauses, and pauses before sends: a new actor that starts paused, with a message waiting.
        let z = other("z");
        let c = turn_of(t, &p).await;
        let paused = CommitRequest {
            sends: vec![to(&z)],
            controls: vec![pause(&z)],
            creates: vec![independent(&z, json!({}))],
            ..done(&c)
        };
        t.commit_ok(paused).await;
        assert!(store.get(&z).await.unwrap().expect("z").paused);
        assert_eq!(t.pending(&z).await.len(), 1);

        // Purges before sends: the mailbox is emptied and gets its first message.
        let before = t.pending(&x).await;
        let c = turn_of(t, &p).await;
        let refill = t.commit_ok(CommitRequest { sends: vec![to(&x)], controls: vec![purge(&x)], ..done(&c) }).await;
        for m in &before {
            assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged));
        }
        assert_eq!(ids(&t.pending(&x).await), ids(&refill.sent));
    })
    .await
}

/// control - a delete with a purge, a pause or a resume of the same actor and no create applies the delete
pub async fn control_a_delete_with_a_purge_or_a_pause_of_the_same_actor_and_no_create_applies_the_delete<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({})).await;
        let y = t.new_actor_named(&OTHER, "y", json!({})).await;
        assert!(store.set_paused(&y, true).await.unwrap());
        let (mx, my) = (t.send(&x).await, t.send(&y).await);
        let c = turn_of(t, &p).await;
        let controls = vec![purge(&x), delete(&x), pause(&x), resume(&y), delete(&y), delete(&x)];
        t.commit_ok(CommitRequest { controls, ..done(&c) }).await;
        for (gone, m) in [(&x, &mx), (&y, &my)] {
            assert_eq!(store.get(gone).await.unwrap(), None, "{gone}");
            assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged), "{gone}");
        }
    })
    .await
}

/// control - a restart of an actor that is not there creates it and writes no `Stopped`
pub async fn control_a_restart_of_an_actor_that_is_not_there_creates_it_and_writes_no_stopped<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let w = watcher(t, "a").await;
        let x = other("x");
        t.register_type(&OTHER).await;
        watch(&*store, &x, &w).await;
        let restart = |c: &crate::store::Claim, id: &str, n: i64| CommitRequest {
            event: event(id),
            controls: vec![delete(&x)],
            creates: vec![independent(&x, json!({"n": n}))],
            ..done(c)
        };
        let c = turn_of(t, &p).await;
        t.commit_ok(restart(&c, "e1", 1)).await;
        assert_eq!(store.get(&x).await.unwrap().expect("created").state, json!({"n": 1}));
        assert!(notifications(t, &w).await.is_empty(), "nothing was removed");

        // Once it is there, the same restart removes it, and its watcher is told.
        let c = turn_of(t, &p).await;
        t.commit_ok(restart(&c, "e2", 2)).await;
        assert_eq!(store.get(&x).await.unwrap().expect("created again").state, json!({"n": 2}));
        assert_eq!(stopped_at(t, &w).await, [(x.clone(), StopCause::Deleted)]);
    })
    .await
}

/// control - the watches of a commit are set before its deletes: a turn that watches an actor and deletes it is told
pub async fn control_the_watches_of_the_commit_are_set_before_its_deletes<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({})).await;
        let c = turn_of(t, &p).await;
        let set = WatchChange { target: WatchTarget::Actor(x.clone()), set: true };
        t.commit_ok(CommitRequest { event: event("e1"), watches: vec![set], controls: vec![delete(&x)], ..done(&c) })
            .await;
        let got = notifications(t, &p).await;
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(stopped_of(&got[0]), Stopped { actor: x.clone(), cause: StopCause::Deleted, by: Some(p.clone()) });
    })
    .await
}

/// control - the watchers of an actor a commit deletes read `deleted`, with `by` and `causedBy`, and the watchers of
/// its descendants read `parent`
pub async fn control_a_deleted_actors_watchers_read_deleted_with_by_and_caused_by_and_its_descendants_parent<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({})).await;
        let y = other("y");
        with_children(t, &x, &[&y]).await;
        let w = watcher(t, "a").await;
        watch(&*store, &x, &w).await;
        watch(&*store, &y, &w).await;
        let c = turn_of(t, &p).await;
        t.commit_ok(CommitRequest { event: event("e1"), controls: vec![delete(&x)], ..done(&c) }).await;
        let got = notifications(t, &w).await;
        let ids: Vec<&str> = got.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["e1.0", "e1.1"], "in the order of the removed actors");
        assert_eq!(stopped_of(&got[0]), Stopped { actor: x.clone(), cause: StopCause::Deleted, by: Some(p.clone()) });
        assert_eq!(stopped_of(&got[1]), Stopped { actor: y.clone(), cause: StopCause::Parent, by: Some(p.clone()) });
        for m in &got {
            assert_eq!(m.caused_by.as_ref(), Some(&c.message.id), "caused by the turn's message");
        }
    })
    .await
}

/// control - several removals in one commit: an actor and its descendant that are both named read `deleted`; a commit
/// that deletes a child and stops reads `deleted`, `parent` and `stopped`; a watcher that the commit removes and
/// creates again has nothing
pub async fn control_several_removals_in_one_commit_give_each_actor_one_cause_and_a_watcher_created_again_has_nothing<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let w = watcher(t, "a").await;

        // An actor and its child, both named.
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({})).await;
        let y = other("y");
        with_children(t, &x, &[&y]).await;
        watch(&*store, &x, &w).await;
        watch(&*store, &y, &w).await;
        let c = turn_of(t, &p).await;
        t.commit_ok(CommitRequest { controls: vec![delete(&y), delete(&x)], ..done(&c) }).await;
        let got = stopped_at(t, &w).await;
        assert_eq!(got, [(x.clone(), StopCause::Deleted), (y.clone(), StopCause::Deleted)], "one each");

        // A turn deletes one of its two children and stops.
        let q = t.new_actor_named(&TYPE, "q", json!({})).await;
        let (k1, k2) = (other("k1"), other("k2"));
        with_children(t, &q, &[&k1, &k2]).await;
        let v = watcher(t, "b").await;
        for on in [&q, &k1, &k2] {
            watch(&*store, on, &v).await;
        }
        let c = turn_of(t, &q).await;
        t.commit_ok(CommitRequest { controls: vec![delete(&k1)], state: StateChange::Stop, ..done(&c) }).await;
        let mut got = stopped_at(t, &v).await;
        got.sort_by(|a, b| a.0.cmp(&b.0));
        let mut expected =
            vec![(k1.clone(), StopCause::Deleted), (k2.clone(), StopCause::Parent), (q.clone(), StopCause::Stopped)];
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(got, expected);
        for gone in [&q, &k1, &k2] {
            assert_eq!(store.get(gone).await.unwrap(), None, "{gone}");
        }

        // A watcher that the commit removes and creates again is another life: no message and no watch.
        let target = t.new_actor_named(&OTHER, "t", json!({})).await;
        let u = watcher(t, "u").await;
        watch(&*store, &target, &u).await;
        let c = turn_of(t, &p).await;
        let request = CommitRequest {
            controls: vec![delete(&target), delete(&u)],
            creates: vec![independent(&u, json!({"again": true}))],
            ..done(&c)
        };
        t.commit_ok(request).await;
        assert_eq!(store.get(&u).await.unwrap().expect("u again").state, json!({"again": true}));
        assert!(t.pending(&u).await.is_empty(), "no message");
        assert!(store.watches(Some(&u), None, None, 10).await.unwrap().items.is_empty(), "no watch");
    })
    .await
}

/// control - a commit that deletes its own actor, or that both pauses and resumes one actor, is invalid, also when the
/// epoch is not current, and applies nothing
pub async fn control_a_commit_that_deletes_its_own_actor_or_pauses_and_resumes_one_actor_is_invalid<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({})).await;
        let c = turn_of(t, &p).await;
        let malformed = [vec![purge(&x), delete(&p)], vec![resume(&x), pause(&x)], vec![pause(&p), resume(&p)]];
        for controls in malformed {
            for epoch in [c.epoch, c.epoch + 1_000] {
                let request = CommitRequest { epoch, controls: controls.clone(), ..done(&c) };
                assert_matches!(store.commit(request).await, Err(StoreError::Invalid(_)), "{controls:?}, {epoch}");
            }
        }
        assert!(!store.get(&x).await.unwrap().expect("x").paused, "nothing is applied");
        assert!(!store.get(&p).await.unwrap().expect("p").paused);
        t.commit_ok(done(&c)).await; // the lease is still held
    })
    .await
}

/// control - two actors that restart each other's neighbours, which have children, while a third sends to the
/// neighbours, end without an error, many times
pub async fn control_two_actors_that_restart_each_others_neighbours_while_a_third_sends_to_them_end_without_an_error<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..ROUNDS {
            // The restarts lock the trees of the neighbours level by level after their ordered locks, while the send
            // locks both neighbours: the commits cross on them.
            let (a, b, s) = (at(&format!("r{round}-a")), at(&format!("r{round}-b")), at(&format!("r{round}-s")));
            let (na, nb) = (other(&format!("r{round}-na")), other(&format!("r{round}-nb")));
            for actor in [&a, &b, &s] {
                t.new_actor_named(&TYPE, actor.key(), json!({})).await;
            }
            for (n, kid) in [(&na, other(&format!("r{round}-na-kid"))), (&nb, other(&format!("r{round}-nb-kid")))] {
                t.new_actor_named(&OTHER, n.key(), json!({"old": true})).await;
                with_children(t, n, &[&kid]).await;
            }
            for actor in [&a, &b, &s] {
                t.send(actor).await;
            }
            let mut claims = in_order(t.claim_with(&[&TYPE], &NODE_A, LONG_LEASE, 10).await);
            assert_eq!(claims.len(), 3, "round {round}");
            let (ca, cb, cs) = (claims.remove(0), claims.remove(0), claims.remove(0));
            let restart = |c: &crate::store::Claim, n: &ActorAddress| CommitRequest {
                controls: vec![delete(n)],
                creates: vec![independent(n, json!({"new": true}))],
                ..done(c)
            };
            let requests =
                [restart(&ca, &nb), restart(&cb, &na), CommitRequest { sends: vec![to(&na), to(&nb)], ..done(&cs) }];
            let results = t
                .concurrently(requests.len(), |i, peer| {
                    let request = requests[i].clone();
                    async move { peer.commit(request).await }
                })
                .await;
            for result in results {
                assert_matches!(result, Ok(CommitResult::Committed { .. }), "round {round}");
            }
            for n in [&na, &nb] {
                assert_eq!(
                    store.get(n).await.unwrap().expect("restarted").state,
                    json!({"new": true}),
                    "round {round}"
                );
                assert!(store.children(n, None, 10).await.unwrap().items.is_empty(), "round {round}: a new life");
                // The next round claims its neighbours as the only due actors of their type.
                assert_matches!(store.delete(n, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. });
            }
        }
    })
    .await
}

/// control - a restart of an actor that is not there, racing a create of it from outside, ends with the state of the
/// restart, many times
pub async fn control_a_restart_of_an_actor_that_is_not_there_racing_a_create_ends_with_the_state_of_the_restart<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        t.register_type(&OTHER).await;
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        for round in 0..ROUNDS {
            let x = other(&format!("x{round}"));
            t.send(&p).await;
            let c = single(t.claim_with(&[&TYPE], &NODE_A, LONG_LEASE, 10).await);
            let restart = CommitRequest {
                controls: vec![delete(&x)],
                creates: vec![independent(&x, json!({"restart": true}))],
                ..done(&c)
            };
            let results = t
                .concurrently(2, |i, peer| {
                    let (restart, x) = (restart.clone(), x.clone());
                    async move {
                        if i == 0 {
                            Err(peer.commit(restart).await)
                        } else {
                            Ok(peer.create(&x, json!({"outside": true})).await)
                        }
                    }
                })
                .await;
            for result in results {
                match result {
                    Err(commit) => assert_matches!(commit, Ok(CommitResult::Committed { .. }), "round {round}"),
                    Ok(create) => {
                        assert_matches!(create, Ok(CreateResult::Created | CreateResult::Exists(_)), "round {round}")
                    }
                }
            }
            let state = store.get(&x).await.unwrap().expect("x").state;
            assert_eq!(state, json!({"restart": true}), "round {round}: the turn's create stands");
        }
    })
    .await
}

/// control - a restart of an actor that is there starts a new life in every field: the state, the version, the parent
/// (a root restarted as a child, a child as a root, a child under another parent), the creation time, the lease, the
/// cancel request, the pause, the mailbox and when it is due, the children and the watches it held
pub async fn control_a_restart_of_an_actor_that_is_there_starts_a_new_life_in_every_field<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let q = t.new_actor_named(&TYPE, "q", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({"n": 1})).await;
        let w = watcher(t, "a").await;
        let far = watcher(t, "far").await;
        // The old life: version 1, a child, a watch it holds, a watch on it, a running turn with a cancel request, a
        // pending message, paused.
        let y = other("y");
        let c = turn_of(t, &x).await;
        let changed = CommitRequest {
            state: StateChange::NewState(json!({"n": 5})),
            creates: vec![child(&y, json!({}))],
            ..done(&c)
        };
        assert_eq!(t.commit_ok(changed).await.version, 1);
        watch(&*store, &x, &w).await;
        watch(&*store, &far, &x).await;
        let running = t.send(&x).await;
        let waiting = t.send(&x).await;
        let on_x = single(t.claim_with(&[&OTHER], &NODE_B, LONG_LEASE, 10).await);
        assert_eq!(on_x.message.id, running.id);
        assert!(store.request_cancel(&x).await.unwrap(), "a cancel request");
        assert!(store.running_turn(&x).await.unwrap().expect("runs").cancel_requested);
        assert!(store.set_paused(&x, true).await.unwrap());
        let before = store.get(&x).await.unwrap().unwrap();
        assert_eq!((before.version, before.paused, before.parent.clone()), (1, true, None));
        t.advance_ticks(1).await; // the new life is created later than the old one

        // p restarts x as its child, with a send: a root becomes a child.
        let c = turn_of(t, &p).await;
        let restart = CommitRequest {
            event: event("e1"),
            controls: vec![delete(&x)],
            creates: vec![child(&x, json!({"n": 2}))],
            sends: vec![to(&x)],
            ..done(&c)
        };
        let committed = t.commit_ok(restart).await;
        let after = store.get(&x).await.unwrap().expect("a new x");
        assert_eq!(after.state, json!({"n": 2}), "state");
        assert_eq!(after.version, 0, "version");
        assert!(!after.paused, "paused");
        assert_eq!(after.parent, Some(p.clone()), "parent: a root becomes a child of the committing actor");
        let committed_at = outcome_record(&*store, &c.message.id).await.expect("done").finished_at;
        assert!(after.created_at > before.created_at, "created_at: {:?}, {:?}", before.created_at, after.created_at);
        assert_eq!(after.created_at, committed_at, "created_at is the time of the commit, as for a create");
        assert_eq!(status_of(&*store, &running.id).await, Some(OutcomeStatus::Purged), "the message being handled");
        assert_eq!(status_of(&*store, &waiting.id).await, Some(OutcomeStatus::Purged), "the pending message");
        assert_eq!(ids(&t.pending(&x).await), ids(&committed.sent), "the mailbox");
        assert_eq!(store.running_turn(&x).await.unwrap(), None, "no turn runs on the new life");
        assert_eq!(store.renew(&x, on_x.epoch, LONG_LEASE).await.unwrap(), RenewResult::LeaseLost, "renew");
        let late = CommitRequest { state: StateChange::NewState(json!({"late": 1})), ..done(&on_x) };
        assert_eq!(t.commit(late).await, CommitResult::LeaseLost, "the old turn commits nothing");
        assert_eq!(store.get(&x).await.unwrap().expect("x").state, json!({"n": 2}));
        assert_eq!(store.get(&y).await.unwrap(), None, "the descendant");
        assert!(store.children(&x, None, 10).await.unwrap().items.is_empty(), "children of the new life");
        assert_eq!(store.children(&p, None, 10).await.unwrap().items, std::slice::from_ref(&x), "children of p");
        assert!(store.watches(Some(&x), None, None, 10).await.unwrap().items.is_empty(), "the watches it held");
        let on = store.watches(None, Some(&WatchTarget::Actor(x.clone())), None, 10).await.unwrap().items;
        assert_eq!(on.len(), 1, "the watch on it stays");
        let got = notifications(t, &w).await;
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].id.as_str(), "e1.0");
        assert_eq!(stopped_of(&got[0]), Stopped { actor: x.clone(), cause: StopCause::Deleted, by: Some(p.clone()) });
        assert_eq!(got[0].caused_by.as_ref(), Some(&c.message.id));
        assert!(notifications(t, &x).await.is_empty(), "the new life has no notification");
        let listed = store.list("", None, 100).await.unwrap().items;
        assert_eq!(listed.iter().filter(|a| **a == x).count(), 1, "listed once: {listed:?}");
        assert!(!listed.contains(&y), "{listed:?}");
        // The new life is due, its claim has a greater epoch, and no cancel request is left.
        let turn = single(t.claim_with(&[&OTHER], &NODE_B, LONG_LEASE, 10).await);
        assert_eq!(turn.actor, x);
        assert!(turn.epoch > on_x.epoch, "S4: {} after {}", turn.epoch, on_x.epoch);
        assert_eq!((turn.version, turn.state.clone()), (0, json!({"n": 2})));
        assert!(!store.running_turn(&x).await.unwrap().expect("runs").cancel_requested, "no cancel request");
        assert_matches!(store.renew(&x, turn.epoch, LONG_LEASE).await.unwrap(), RenewResult::Renewed(_));
        assert_eq!(t.commit(done(&on_x)).await, CommitResult::LeaseLost, "the old turn, again");
        t.commit_ok(done(&turn)).await;

        // q restarts x as a root: a child becomes a root.
        let c = turn_of(t, &q).await;
        let again =
            CommitRequest { controls: vec![delete(&x)], creates: vec![independent(&x, json!({"n": 3}))], ..done(&c) };
        t.commit_ok(again).await;
        let record = store.get(&x).await.unwrap().expect("x");
        assert_eq!((record.state, record.parent), (json!({"n": 3}), None), "a child becomes a root");
        assert!(store.children(&p, None, 10).await.unwrap().items.is_empty());
        // p restarts it as its child, then q restarts it as its own child: a child under another parent.
        let c = turn_of(t, &p).await;
        let mine = CommitRequest { controls: vec![delete(&x)], creates: vec![child(&x, json!({"n": 4}))], ..done(&c) };
        t.commit_ok(mine).await;
        assert_eq!(store.get(&x).await.unwrap().expect("x").parent, Some(p.clone()));
        let c = turn_of(t, &q).await;
        let theirs =
            CommitRequest { controls: vec![delete(&x)], creates: vec![child(&x, json!({"n": 5}))], ..done(&c) };
        t.commit_ok(theirs).await;
        assert_eq!(store.get(&x).await.unwrap().expect("x").parent, Some(q.clone()), "under another parent");
        assert!(store.children(&p, None, 10).await.unwrap().items.is_empty());
        assert_eq!(store.children(&q, None, 10).await.unwrap().items, std::slice::from_ref(&x));

        // A restart without a send of an actor with a pending message leaves nothing due; a later send is claimed.
        let old = t.send(&x).await;
        let c = turn_of(t, &p).await;
        let quiet =
            CommitRequest { controls: vec![delete(&x)], creates: vec![independent(&x, json!({"n": 6}))], ..done(&c) };
        t.commit_ok(quiet).await;
        assert_eq!(status_of(&*store, &old.id).await, Some(OutcomeStatus::Purged));
        assert!(t.claim_types(&[&OTHER]).await.is_empty(), "nothing is due");
        let new = t.send(&x).await;
        assert_eq!(t.claim_one_types(&[&OTHER]).await.message.id, new.id);
    })
    .await
}

/// control - a create of a descendant of a deleted actor makes a new independent actor, and a stopping turn that
/// restarts its child takes the new child with it, unless it restarts it as an independent actor
pub async fn control_a_descendant_of_a_deleted_actor_created_again_and_a_restart_in_a_stopping_turn<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let w = watcher(t, "a").await;
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let x = t.new_actor_named(&OTHER, "x", json!({})).await;
        let (y, z) = (other("y"), other("z"));
        with_children(t, &x, &[&y]).await;
        with_children(t, &y, &[&z]).await;
        for on in [&x, &y, &z] {
            watch(&*store, on, &w).await;
        }
        let old = t.send(&y).await;
        // p deletes x and creates y, a descendant of x, as an independent actor, and sends to it.
        let c = turn_of(t, &p).await;
        let committed = t
            .commit_ok(CommitRequest {
                event: event("e1"),
                controls: vec![delete(&x)],
                creates: vec![independent(&y, json!({"new": 1}))],
                sends: vec![to(&y)],
                ..done(&c)
            })
            .await;
        assert_eq!(store.get(&x).await.unwrap(), None);
        assert_eq!(store.get(&z).await.unwrap(), None);
        let record = store.get(&y).await.unwrap().expect("a new y");
        assert_eq!((record.state, record.version, record.parent), (json!({"new": 1}), 0, None));
        assert_eq!(status_of(&*store, &old.id).await, Some(OutcomeStatus::Purged));
        assert_eq!(ids(&t.pending(&y).await), ids(&committed.sent));
        let mut got = stopped_at(t, &w).await;
        got.sort_by(|a, b| a.0.cmp(&b.0));
        let removed = [(x.clone(), StopCause::Deleted), (y.clone(), StopCause::Parent), (z.clone(), StopCause::Parent)];
        assert_eq!(got, removed);
        let turn = t.claim_one_types(&[&OTHER]).await;
        assert_eq!(turn.actor, y);
        t.commit_ok(done(&turn)).await;

        // A turn that restarts its child as its child, sends to it and stops: everything goes.
        let q = t.new_actor_named(&TYPE, "q", json!({})).await;
        let (k1, k2, g) = (other("k1"), other("k2"), other("g"));
        with_children(t, &q, &[&k1, &k2]).await;
        with_children(t, &k1, &[&g]).await;
        let v = watcher(t, "b").await;
        for on in [&q, &k1, &k2, &g] {
            watch(&*store, on, &v).await;
        }
        let c = turn_of(t, &q).await;
        let m = t
            .commit_ok(CommitRequest {
                controls: vec![delete(&k1)],
                creates: vec![child(&k1, json!({"again": 1}))],
                sends: vec![to(&k1)],
                state: StateChange::Stop,
                ..done(&c)
            })
            .await;
        for gone in [&q, &k1, &k2, &g] {
            assert_eq!(store.get(gone).await.unwrap(), None, "{gone}");
        }
        assert_eq!(status_of(&*store, &m.sent[0].id).await, Some(OutcomeStatus::Purged), "the send to the new child");
        let mut got = stopped_at(t, &v).await;
        got.sort_by(|a, b| a.0.cmp(&b.0));
        let mut expected = vec![
            (q.clone(), StopCause::Stopped),
            (k1.clone(), StopCause::Deleted),
            (k2.clone(), StopCause::Parent),
            (g.clone(), StopCause::Parent),
        ];
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(got, expected);

        // A turn that restarts its child as an independent actor and stops: the new actor stays.
        let r = t.new_actor_named(&TYPE, "r", json!({})).await;
        let k3 = other("k3");
        with_children(t, &r, &[&k3]).await;
        let c = turn_of(t, &r).await;
        t.commit_ok(CommitRequest {
            controls: vec![delete(&k3)],
            creates: vec![independent(&k3, json!({"free": 1}))],
            state: StateChange::Stop,
            ..done(&c)
        })
        .await;
        assert_eq!(store.get(&r).await.unwrap(), None);
        let record = store.get(&k3).await.unwrap().expect("k3 stays");
        assert_eq!((record.state, record.parent), (json!({"free": 1}), None));
    })
    .await
}

/// `claims` in address order.
fn in_order(mut claims: Vec<crate::store::Claim>) -> Vec<crate::store::Claim> {
    claims.sort_by(|a, b| a.actor.cmp(&b.actor));
    claims
}
