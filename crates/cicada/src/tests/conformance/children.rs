//! Child actors (ADR 0019): a commit's child create gives the new instance the committing one as its parent; children
//! are listed by parent; a delete or a stop removes every descendant with its mailbox; a child create never takes over
//! an existing instance.

use crate::address::ActorAddress;
use crate::store::{
    ActorCreate, Backend, CommitRequest, CommitResult, DeleteResult, MessageId, OutcomeStatus, OutgoingMessage,
    StateChange, StoreError, TurnOutcome,
};
use serde_json::{Value, json};

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

/// The address of `key` of [`TYPE`].
fn at(key: &str) -> ActorAddress {
    ActorAddress::of(&TYPE, key).expect("a valid address")
}

/// A child create of `address` with `state`.
fn child(address: &ActorAddress, state: Value) -> ActorCreate {
    ActorCreate { address: address.clone(), state, child: true, pinned_to: None }
}

/// An independent create of `address` with `state`.
fn independent(address: &ActorAddress, state: Value) -> ActorCreate {
    ActorCreate { address: address.clone(), state, child: false, pinned_to: None }
}

/// Runs a turn of `parent` that creates the instances of `keys` as its children, and returns their addresses.
/// `parent` must be the only instance of [`TYPE`] with a due message.
async fn with_children<F: StoreFixture>(t: &Env<F>, parent: &ActorAddress, keys: &[&str]) -> Vec<ActorAddress> {
    t.send(parent).await;
    let c = t.claim_one().await;
    assert_eq!(&c.actor, parent);
    let children: Vec<ActorAddress> = keys.iter().map(|k| at(k)).collect();
    t.commit_ok(CommitRequest { creates: children.iter().map(|a| child(a, json!({}))).collect(), ..done(&c) }).await;
    children
}

async fn parent_of(store: &dyn Backend, a: &ActorAddress) -> Option<ActorAddress> {
    store.get(a).await.unwrap().expect("exists").parent
}

async fn status_of(store: &dyn Backend, id: &MessageId) -> Option<OutcomeStatus> {
    outcome_record(store, id).await.map(|o| o.status)
}

/// Every instance of the store.
async fn all_actors(store: &dyn Backend) -> Vec<ActorAddress> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = store.list("", cursor.as_ref(), 100).await.unwrap();
        out.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return out,
        }
    }
}

/// Every instance whose parent is gone.
async fn orphans(store: &dyn Backend) -> Vec<ActorAddress> {
    let mut out = Vec::new();
    for a in all_actors(store).await {
        if let Some(record) = store.get(&a).await.unwrap()
            && let Some(parent) = record.parent
            && store.get(&parent).await.unwrap().is_none()
        {
            out.push(a);
        }
    }
    out
}

/// children - a commit creates a child of the committing actor, and other creates have no parent
pub async fn children_a_commit_creates_a_child_of_the_committing_actor_and_other_creates_have_no_parent<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor(&TYPE, json!({})).await;
        let (kid, solo) = (at("kid"), at("solo"));
        t.send(&p).await;
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest {
            creates: vec![child(&kid, json!({"n": 1})), independent(&solo, json!({}))],
            ..done(&c)
        })
        .await;
        assert_eq!(parent_of(&*store, &kid).await, Some(p.clone()), "a child create");
        assert_eq!(parent_of(&*store, &solo).await, None, "an independent create in a commit");
        assert_eq!(parent_of(&*store, &p).await, None, "a create from outside a turn");

        // A child create of an own child leaves it untouched: state and mailbox stay.
        t.send(&p).await;
        let c = t.claim_one().await;
        let waiting = t.send(&kid).await;
        t.commit_ok(CommitRequest { creates: vec![child(&kid, json!({"n": 2}))], ..done(&c) }).await;
        let record = store.get(&kid).await.unwrap().expect("exists");
        assert_eq!((record.state, record.parent), (json!({"n": 1}), Some(p.clone())));
        assert_eq!(ids(&t.pending(&kid).await), [waiting.id]);
    })
    .await
}

/// children - children are listed by parent in address order, with paging, and only the direct ones
pub async fn children_are_listed_by_parent_in_address_order_with_paging_and_only_direct_ones<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor_named(&TYPE, "a", json!({})).await;
        t.new_actor_named(&TYPE, "a-solo", json!({})).await;
        let kids = with_children(t, &a, &["b2", "b1"]).await;
        let (b2, b1) = (kids[0].clone(), kids[1].clone());
        let c = with_children(t, &b1, &["c"]).await.remove(0);

        let all = store.children(&a, None, 100).await.unwrap();
        assert_eq!(all.items, [b1.clone(), b2.clone()], "in address order, direct children only");
        assert!(all.next.is_none());
        let first = store.children(&a, None, 1).await.unwrap();
        assert_eq!(first.items, std::slice::from_ref(&b1));
        let second = store.children(&a, first.next.as_ref(), 1).await.unwrap();
        assert_eq!(second.items, std::slice::from_ref(&b2));
        assert!(second.next.is_none(), "the last page");
        assert_eq!(store.children(&b1, None, 100).await.unwrap().items, std::slice::from_ref(&c));
        assert!(store.children(&c, None, 100).await.unwrap().items.is_empty(), "a leaf");
        assert!(store.children(&at("nosuch"), None, 100).await.unwrap().items.is_empty(), "an absent parent");
        assert!(matches!(store.children(&a, None, 0).await, Err(StoreError::Invalid(_))), "a limit of 0");
    })
    .await
}

/// children - a delete removes every descendant, purges their mailboxes and reports the sum
pub async fn children_a_delete_removes_every_descendant_purges_their_mailboxes_and_reports_the_sum<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor_named(&TYPE, "a", json!({})).await;
        let other = t.new_actor_named(&TYPE, "other", json!({"keep": 1})).await;
        let b = with_children(t, &a, &["b"]).await.remove(0);
        let c = with_children(t, &b, &["c"]).await.remove(0);
        let messages = [t.send(&a).await, t.send(&b).await, t.send(&c).await, t.send(&c).await];
        let kept = t.send(&other).await;

        assert_eq!(store.delete(&a, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { purged: 4 });
        for gone in [&a, &b, &c] {
            assert_eq!(store.get(gone).await.unwrap(), None, "{gone}");
            assert!(t.pending(gone).await.is_empty(), "{gone}");
        }
        for m in &messages {
            assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged), "{}", m.id);
        }
        assert_eq!(store.get(&other).await.unwrap().expect("stays").state, json!({"keep": 1}));
        assert_eq!(ids(&t.pending(&other).await), [kept.id], "no other actor changed");
        assert_eq!(store.delete(&b, &MessageId::random()).await.unwrap(), DeleteResult::Absent);
    })
    .await
}

/// children - a stop removes every descendant, the children of its commit included, and purges a message sent to one
pub async fn children_a_stop_removes_the_tree_with_the_children_of_its_commit_and_purges_a_message_sent_to_one<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let old = with_children(t, &p, &["old"]).await.remove(0);
        t.send(&p).await;
        let c = t.claim_one().await;
        let waiting = t.send(&old).await;
        let later = t.send(&p).await;
        let new = at("new");
        let committed = t
            .commit_ok(CommitRequest {
                state: StateChange::Stop,
                creates: vec![child(&new, json!({}))],
                sends: vec![
                    OutgoingMessage::new(new.clone(), "Msg", json!(1)),
                    OutgoingMessage::new(old.clone(), "Msg", json!(2)),
                ],
                ..done(&c)
            })
            .await;
        for gone in [&p, &old, &new] {
            assert_eq!(store.get(gone).await.unwrap(), None, "{gone}");
        }
        assert_eq!(status_of(&*store, &c.message.id).await, Some(OutcomeStatus::Done));
        for m in committed.sent.iter().chain([&waiting, &later]) {
            assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged), "{}", m.id);
        }
    })
    .await
}

/// children - a stop of a child leaves its parent and its siblings
pub async fn children_a_stop_of_a_child_leaves_its_parent_and_its_siblings<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let kids = with_children(t, &p, &["b1", "b2"]).await;
        let (b1, b2) = (kids[0].clone(), kids[1].clone());
        let c = with_children(t, &b1, &["c"]).await.remove(0);
        t.send(&b1).await;
        let turn = t.claim_one().await;
        t.commit_ok(CommitRequest { state: StateChange::Stop, ..done(&turn) }).await;
        assert_eq!(store.get(&b1).await.unwrap(), None);
        assert_eq!(store.get(&c).await.unwrap(), None, "its own descendants go");
        assert_eq!(parent_of(&*store, &b2).await, Some(p.clone()), "a sibling stays");
        assert!(store.get(&p).await.unwrap().is_some(), "the parent stays");
        assert_eq!(store.children(&p, None, 100).await.unwrap().items, [b2]);
    })
    .await
}

/// children - a child create of an actor that is independent, has another parent, or is the committing actor itself
/// rejects the commit and keeps the lease
pub async fn children_a_child_create_of_an_independent_or_foreign_actor_or_of_itself_rejects_the_commit<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({"n": 1})).await;
        let q = t.new_actor_named(&TYPE, "q", json!({})).await;
        let solo = t.new_actor_named(&TYPE, "solo", json!({"solo": 1})).await;
        let theirs = with_children(t, &q, &["theirs"]).await.remove(0);
        let fresh = at("fresh");
        for (taken, owner) in [(&solo, None), (&theirs, Some(q.clone())), (&p, None)] {
            let m = t.send(&p).await;
            let c = t.claim_one().await;
            let result = t
                .commit(CommitRequest {
                    state: StateChange::NewState(json!({"n": 2})),
                    creates: vec![child(&fresh, json!({})), child(taken, json!({"taken": true}))],
                    sends: vec![OutgoingMessage::new(q.clone(), "Msg", json!(0))],
                    ..done(&c)
                })
                .await;
            let reason = expect_matches!(result, CommitResult::Rejected(reason) => reason, "{taken}");
            assert!(reason.contains(taken.as_str()), "the reason names the actor: {reason}");
            assert_eq!(parent_of(&*store, taken).await, owner, "{taken} is unchanged");
            assert_eq!(store.get(&fresh).await.unwrap(), None, "nothing is applied");
            assert_eq!(store.get(&p).await.unwrap().expect("exists").state, json!({"n": 1}));
            assert!(t.pending(&q).await.is_empty(), "no send");
            t.commit_ok(ending(&c, TurnOutcome::Failed(reason))).await; // the lease is still held
            assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Failed));
        }
        assert_eq!(store.get(&solo).await.unwrap().expect("exists").state, json!({"solo": 1}));
    })
    .await
}

/// children - creates of one address that disagree on being a child are invalid, also when the epoch is not current
pub async fn children_creates_of_one_address_that_disagree_are_invalid_also_when_the_epoch_is_not_current<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor(&TYPE, json!({})).await;
        let x = at("x");
        t.send(&p).await;
        let c = t.claim_one().await;
        let malformed = |epoch: i64| CommitRequest {
            epoch,
            creates: vec![independent(&x, json!({})), child(&x, json!({}))],
            ..done(&c)
        };
        for epoch in [c.epoch, c.epoch + 1_000, c.epoch - 1] {
            assert_matches!(store.commit(malformed(epoch)).await, Err(StoreError::Invalid(_)), "epoch {epoch}");
        }
        assert_eq!(store.get(&x).await.unwrap(), None, "nothing is applied");
        t.commit_ok(done(&c)).await; // the lease is still held
    })
    .await
}

/// children - a turn that runs on a descendant gets `LeaseLost`, and its message is purged
pub async fn children_a_turn_that_runs_on_a_descendant_gets_lease_lost_and_its_message_is_purged<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        let b = with_children(t, &p, &["b"]).await.remove(0);
        let c = with_children(t, &b, &["c"]).await.remove(0);
        let m = t.send(&c).await;
        let running = t.claim_one().await;
        assert_eq!(running.actor, c);
        assert_eq!(store.delete(&p, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { purged: 1 });
        assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged), "also the one being handled");
        let result = t
            .commit(CommitRequest {
                state: StateChange::NewState(json!({"late": true})),
                creates: vec![child(&at("late"), json!({}))],
                ..done(&running)
            })
            .await;
        assert_eq!(result, CommitResult::LeaseLost);
        assert_eq!(store.get(&c).await.unwrap(), None);
        assert_eq!(store.get(&at("late")).await.unwrap(), None, "the turn applied nothing");
        assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged));
    })
    .await
}

/// children - a delete racing commits that create children leaves no orphan
pub async fn children_a_delete_racing_commits_that_create_children_leaves_no_orphan<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..10 {
            let root = t.new_actor_named(&TYPE, &format!("r{round}"), json!({})).await;
            let kids = with_children(t, &root, &[&format!("r{round}-b1"), &format!("r{round}-b2")]).await;
            let mut claims = Vec::new();
            for a in std::iter::once(&root).chain(&kids) {
                t.send(a).await;
                claims.push(t.claim_one().await);
            }
            let results = t
                .concurrently(claims.len() + 1, |i, peer| {
                    let (claims, root) = (claims.clone(), root.clone());
                    async move {
                        match claims.get(i) {
                            Some(c) => {
                                let grandchild = at(&format!("{}-n", c.actor.key()));
                                let request = CommitRequest { creates: vec![child(&grandchild, json!({}))], ..done(c) };
                                Some(peer.commit(request).await.unwrap())
                            }
                            None => {
                                let deleted = peer.delete(&root, &MessageId::random()).await.unwrap();
                                assert!(matches!(deleted, DeleteResult::Deleted { .. }), "{deleted:?}");
                                None
                            }
                        }
                    }
                })
                .await;
            for result in results.into_iter().flatten() {
                assert!(
                    matches!(result, CommitResult::Committed { .. } | CommitResult::LeaseLost),
                    "round {round}: {result:?}"
                );
            }
            assert_eq!(orphans(&*store).await, Vec::<ActorAddress>::new(), "round {round}");
            assert!(store.list(&format!("{}/r{round}", TYPE.as_str()), None, 100).await.unwrap().items.is_empty());
        }
    })
    .await
}

/// children - a delete racing a send up the tree, from a child whose address sorts before its parent's, succeeds and
/// leaves no actor of the tree
pub async fn children_a_delete_racing_a_send_up_the_tree_from_a_child_that_sorts_first_leaves_no_actor_of_the_tree<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..20 {
            let parent = t.new_actor_named(&TYPE, &format!("p{round}"), json!({})).await;
            let kid = with_children(t, &parent, &[&format!("a{round}")]).await.remove(0);
            assert!(kid < parent, "the child sorts first");
            t.send(&kid).await;
            let c = t.claim_one().await;
            let results = t
                .concurrently(2, |i, peer| {
                    let (c, parent) = (c.clone(), parent.clone());
                    async move {
                        if i == 0 {
                            let up = OutgoingMessage::new(parent.clone(), "Up", json!(0));
                            Err(peer.commit(CommitRequest { sends: vec![up], ..done(&c) }).await.unwrap())
                        } else {
                            Ok(peer.delete(&parent, &MessageId::random()).await.unwrap())
                        }
                    }
                })
                .await;
            for result in results {
                match result {
                    Err(commit) => assert!(
                        matches!(commit, CommitResult::Committed { .. } | CommitResult::LeaseLost),
                        "round {round}: {commit:?}"
                    ),
                    Ok(deleted) => assert!(matches!(deleted, DeleteResult::Deleted { .. }), "round {round}"),
                }
            }
            assert_eq!(store.get(&parent).await.unwrap(), None, "round {round}");
            assert_eq!(store.get(&kid).await.unwrap(), None, "round {round}");
        }
    })
    .await
}
