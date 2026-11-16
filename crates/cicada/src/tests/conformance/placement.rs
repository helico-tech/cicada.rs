//! Placement (ADR 0052): an actor may be pinned to one claim owner at creation. Only that owner claims it; every other
//! owner skips it before the limit of a claim, changes nothing of it, and does not count it in its next due time. The pin
//! never changes for a life of the actor, is not inherited by children, and needs no node row.

use crate::address::ActorAddress;
use crate::store::{
    ActorCreate, Backend, CommitRequest, CommitResult, CreateResult, Delivery, MessageStatus, NodeId, StoreError,
};
use serde_json::{Value, json};

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

/// Creates the instance of [`TYPE`] with the key `key`, pinned to `pin`, and returns its address.
async fn pinned<F: StoreFixture>(t: &Env<F>, key: &str, pin: Option<&NodeId>) -> ActorAddress {
    t.register_type(&TYPE).await;
    let address = ActorAddress::of(&TYPE, key).expect("a valid address");
    assert_eq!(t.store().create(&address, json!({}), pin).await.unwrap(), CreateResult::Created);
    address
}

async fn pin_of(store: &dyn Backend, a: &ActorAddress) -> Option<NodeId> {
    store.get(a).await.unwrap().expect("exists").pinned_to
}

fn create(address: &ActorAddress, state: Value, child: bool, pin: Option<&NodeId>) -> ActorCreate {
    ActorCreate { address: address.clone(), state, child, pinned_to: pin.cloned() }
}

/// placement - a create records the pin without a node row, a repeat keeps the first pin, and a pin beyond the bounds
/// of a claim owner is invalid and creates nothing
pub async fn placement_a_create_records_its_pin_and_a_repeat_keeps_the_first_one<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let future = node("not-started-yet");
        let a = pinned(t, "a", Some(&future)).await;
        assert!(store.list_nodes().await.unwrap().is_empty(), "no node row is needed");
        assert_eq!(pin_of(&*store, &a).await, Some(future.clone()));
        for again in [None, Some(&*NODE_B)] {
            let existing = expect_matches!(store.create(&a, json!({"x": 1}), again).await.unwrap(),
                CreateResult::Exists(r) => r);
            assert_eq!((existing.pinned_to, existing.state), (Some(future.clone()), json!({})), "left untouched");
        }
        let b = pinned(t, "b", None).await;
        assert_eq!(pin_of(&*store, &b).await, None, "unpinned by default");
        let records = store.list_records("", None, 10).await.unwrap().items;
        assert_eq!(records.iter().map(|r| r.pinned_to.clone()).collect::<Vec<_>>(), [Some(future), None]);

        let c = ActorAddress::of(&TYPE, "c").unwrap();
        for bad in [node(&"x".repeat(201)), node("a\0b")] {
            assert_matches!(store.create(&c, json!({}), Some(&bad)).await, Err(StoreError::Invalid(_)));
        }
        assert!(store.get(&c).await.unwrap().is_none(), "nothing is created");
        let longest = node(&"é".repeat(200));
        assert_eq!(store.create(&c, json!({}), Some(&longest)).await.unwrap(), CreateResult::Created);
        assert_eq!(pin_of(&*store, &c).await, Some(longest), "200 characters, not bytes");
    })
    .await
}

/// placement - actors pinned to another owner are skipped before the limit: an eligible actor due later is claimed
/// with a limit of one, the foreign ones keep their attempts and leases, and unpinned actors keep their order
pub async fn placement_foreign_pins_never_take_the_place_of_an_eligible_actor<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let mut foreign = Vec::new();
        for key in ["f1", "f2"] {
            let f = pinned(t, key, Some(&NODE_B)).await;
            t.send(&f).await;
            foreign.push(f);
        }
        assert!(t.claim().await.is_empty(), "only foreign pins are due: an empty batch for node A");
        assert_eq!(store.next_due_at(&type_set(&[&TYPE]), &NODE_A).await.unwrap(), None, "none due for node A");
        assert!(store.next_due_at(&type_set(&[&TYPE]), &NODE_B).await.unwrap().is_some(), "due for node B");

        let mine = pinned(t, "mine", Some(&NODE_A)).await;
        let loose = pinned(t, "loose", None).await;
        let later = Delivery::After(t.ticks(1));
        t.send_at(&mine, later).await;
        t.send_at(&loose, Delivery::After(t.ticks(2))).await;
        t.advance_ticks(3).await;
        let first = t.claim_limit(1).await;
        assert_eq!(
            first.iter().map(|c| &c.actor).collect::<Vec<_>>(),
            [&mine],
            "the eligible actor, not a foreign one"
        );
        let second = t.claim_limit(1).await;
        assert_eq!(second.iter().map(|c| &c.actor).collect::<Vec<_>>(), [&loose], "unpinned keeps its order");
        assert!(t.claim().await.is_empty());
        for f in &foreign {
            assert!(store.running_turn(f).await.unwrap().is_none(), "{f} holds no lease");
            let head = &t.pending(f).await[0];
            assert_eq!((head.attempt, head.lost_claims), (0, 0), "{f} was never claimed");
        }
        let theirs = t.claim_node(&NODE_B).await;
        let mut claimed: Vec<&ActorAddress> = theirs.iter().map(|c| &c.actor).collect();
        claimed.sort();
        assert_eq!(claimed, foreign.iter().collect::<Vec<_>>(), "node B claims its own");
        assert!(theirs.iter().all(|c| c.message.attempt == 1));
    })
    .await
}

/// placement - a paused pinned actor is not claimed by its owner either, and an expired lease of it is claimed again
/// by its owner only, counting the lost claim
pub async fn placement_pause_and_expired_leases_apply_to_the_owner_as_to_anyone<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = pinned(t, "a", Some(&NODE_B)).await;
        let m = t.send(&a).await;
        assert!(store.set_paused(&a, true).await.unwrap());
        assert!(t.claim_node(&NODE_B).await.is_empty(), "paused: not even its owner claims it");
        assert_eq!(store.next_due_at(&type_set(&[&TYPE]), &NODE_B).await.unwrap(), None);
        assert!(store.set_paused(&a, false).await.unwrap());
        let first = t.claim_one_node(&NODE_B).await;
        t.expire_lease().await;
        assert!(t.claim().await.is_empty(), "another owner does not take over an expired lease");
        assert!(store.next_due_at(&type_set(&[&TYPE]), &NODE_A).await.unwrap().is_none());
        let second = t.claim_one_long_node(&NODE_B).await;
        assert_eq!(second.message.id, m.id);
        assert_eq!((second.message.attempt, second.message.lost_claims), (2, 1), "the replaced claim is lost");
        assert!(second.epoch > first.epoch);
        let status = store.status(&m.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Running { node, .. }) => node), *NODE_B);
    })
    .await
}

/// placement - a commit's creates carry their pins, a child does not inherit its parent's pin, creates of one address
/// that disagree on their pin (None against Some included) or carry a pin beyond its bounds are invalid before the
/// epoch is checked, and a rejected commit applies no pin
pub async fn placement_commit_creates_carry_their_pins_and_disagreeing_pins_are_invalid<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let parent = pinned(t, "parent", Some(&NODE_A)).await;
        let child = ActorAddress::of(&TYPE, "child").unwrap();
        let pinned_child = ActorAddress::of(&TYPE, "pinned-child").unwrap();
        let other = ActorAddress::of(&TYPE, "other").unwrap();

        t.send(&parent).await;
        let c = t.claim_one().await;
        let invalid = [
            vec![create(&other, json!({}), false, None), create(&other, json!({}), false, Some(&NODE_B))],
            vec![create(&other, json!({}), false, Some(&NODE_A)), create(&other, json!({}), false, Some(&NODE_B))],
            vec![create(&other, json!({}), false, Some(&node(&"x".repeat(201))))],
            vec![create(&other, json!({}), false, Some(&node("a\0b")))],
        ];
        for creates in &invalid {
            let stale = CommitRequest { creates: creates.clone(), epoch: c.epoch - 1, ..done(&c) };
            assert_matches!(store.commit(stale).await, Err(StoreError::Invalid(_)), "{creates:?}");
            let current = CommitRequest { creates: creates.clone(), ..done(&c) };
            assert_matches!(store.commit(current).await, Err(StoreError::Invalid(_)), "{creates:?}");
        }
        let rejected = CommitRequest {
            creates: vec![create(&other, json!({}), false, Some(&NODE_B))],
            sends: vec![crate::store::OutgoingMessage::new(
                ActorAddress::of(&TYPE, "absent").unwrap(),
                "Msg",
                json!(1),
            )],
            ..done(&c)
        };
        assert_matches!(store.commit(rejected).await.unwrap(), CommitResult::Rejected(_));
        assert!(store.get(&other).await.unwrap().is_none(), "a rejected commit creates nothing");

        let same = create(&pinned_child, json!({}), true, Some(&NODE_B));
        let creates = vec![create(&child, json!({}), true, None), same.clone(), same];
        t.commit_ok(CommitRequest { creates, ..done(&c) }).await;
        assert_eq!(pin_of(&*store, &child).await, None, "a child does not inherit its parent's pin");
        assert_eq!(pin_of(&*store, &pinned_child).await, Some(NODE_B.clone()), "agreeing duplicates are one create");
        let record = store.get(&pinned_child).await.unwrap().unwrap();
        assert_eq!(record.parent, Some(parent.clone()), "pin and parent are independent");
        assert_eq!(pin_of(&*store, &parent).await, Some(NODE_A.clone()));
    })
    .await
}

/// placement - deleting an actor ends its life: created again, from outside or by a restart in a commit, it may have
/// another pin
pub async fn placement_a_new_life_may_choose_another_pin<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = pinned(t, "a", Some(&NODE_A)).await;
        store.delete(&a, &crate::store::MessageId::random()).await.unwrap();
        assert_eq!(store.create(&a, json!({}), Some(&NODE_B)).await.unwrap(), CreateResult::Created);
        assert_eq!(pin_of(&*store, &a).await, Some(NODE_B.clone()));

        let restarter = pinned(t, "restarter", None).await;
        t.send(&restarter).await;
        let c = t.claim_one().await;
        let restart = CommitRequest {
            controls: vec![crate::store::Control { effect: crate::store::ControlEffect::Delete, actor: a.clone() }],
            creates: vec![create(&a, json!({"life": 3}), false, None)],
            ..done(&c)
        };
        t.commit_ok(restart).await;
        let record = store.get(&a).await.unwrap().unwrap();
        assert_eq!((record.state, record.pinned_to), (json!({"life": 3}), None), "a restart chose no pin");
    })
    .await
}
