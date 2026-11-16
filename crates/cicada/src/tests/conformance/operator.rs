//! The operator's tools: the type registry refusing creates, the node table, running turns and turn cancel, mailbox
//! pause and purge, and delivering a message now.

use std::collections::BTreeSet;

use crate::store::{
    ActorCreate, CommitRequest, CommitResult, CreateResult, DeliverNowResult, Delivery, MessageId, MessageStatus,
    NodeDetails, OutcomeStatus, OutgoingMessage, RemoveTypeResult, RenewResult, TurnOutcome,
};
use serde_json::json;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

/// The types of a single-claim fixture.
fn typeset() -> BTreeSet<crate::address::TypeName> {
    type_set(&[&TYPE, &OTHER])
}

async fn status_of(store: &dyn crate::store::Backend, id: &MessageId) -> Option<OutcomeStatus> {
    outcome_record(store, id).await.map(|o| o.status)
}

async fn state_of(store: &dyn crate::store::Backend, a: &crate::address::ActorAddress) -> serde_json::Value {
    store.get(a).await.unwrap().expect("exists").state
}

/// registry - a create of an unregistered type is refused and creates nothing
pub async fn registry_a_create_of_an_unregistered_type_is_refused_and_creates_nothing<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = address("nosuch/x");
        assert_eq!(store.create(&a, json!({}), None).await.unwrap(), CreateResult::NoSuchType);
        assert_eq!(store.get(&a).await.unwrap(), None);
        assert!(t.claim_types(&[&type_name("nosuch")]).await.is_empty());
    })
    .await
}

/// registry - a commit creating an unregistered type is rejected, applies nothing and keeps the lease
pub async fn registry_a_commit_creating_an_unregistered_type_is_rejected_applies_nothing_and_keeps_the_lease<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 1})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        let ghost = address("nosuch/g");
        let result = t
            .commit(CommitRequest {
                state: crate::store::StateChange::NewState(json!({"n": 2})),
                creates: vec![ActorCreate { address: ghost.clone(), state: json!({}), child: false, pinned_to: None }],
                ..done(&c)
            })
            .await;
        match result {
            CommitResult::Rejected(reason) => assert!(reason.contains("not registered"), "{reason}"),
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert_eq!(store.get(&ghost).await.unwrap(), None, "nothing created");
        assert_eq!(state_of(&*store, &a).await, json!({"n": 1}), "nothing applied");
        t.commit_ok(ending(&c, TurnOutcome::Failed("rejected".into()))).await; // the lease is still held
        assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Failed));
    })
    .await
}

/// registry - removing a type racing a commit that creates an instance of it leaves no instance behind the removal
pub async fn registry_removing_a_type_racing_a_commit_that_creates_an_instance_of_it_leaves_no_orphan<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..20 {
            let name = type_name(&format!("racy{round}"));
            let child = address(&format!("{name}/c"));
            t.register_type(&name).await;
            let a = t.new_actor(&TYPE, json!({})).await;
            t.send(&a).await;
            let c = t.claim_one().await;
            let results = t
                .concurrently(2, |i, peer| {
                    let (c, name, child) = (c.clone(), name.clone(), child.clone());
                    async move {
                        if i == 0 {
                            peer.commit(CommitRequest {
                                creates: vec![ActorCreate {
                                    address: child,
                                    state: json!({}),
                                    child: false,
                                    pinned_to: None,
                                }],
                                ..done(&c)
                            })
                            .await
                            .unwrap();
                            None
                        } else {
                            Some(peer.remove_type(&name).await.unwrap())
                        }
                    }
                })
                .await;
            let removal = results.into_iter().flatten().next().expect("the remove answered");
            let exists = store.get(&child).await.unwrap().is_some();
            // A rejected commit left the lease in place; end the turn, so the next round claims only its own actor.
            t.commit(ending(&c, TurnOutcome::Failed("rejected".into()))).await;
            match removal {
                RemoveTypeResult::Removed => assert!(!exists, "round {round}: orphan behind the removal"),
                RemoveTypeResult::InUse => assert!(exists, "round {round}"),
                RemoveTypeResult::Absent => panic!("round {round}: the type was registered"),
            }
        }
    })
    .await
}

/// nodes - a heartbeat upserts the row with store time, keeps started_at and lists by id
pub async fn nodes_a_heartbeat_upserts_the_row_with_store_time_and_nodes_are_listed_by_id<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = node("node-a");
        let b = node("node-b");
        let hosted = type_set(&[&TYPE, &OTHER]);
        store.heartbeat_node(&b, &hosted, &Default::default()).await.unwrap();
        store.heartbeat_node(&a, &BTreeSet::new(), &Default::default()).await.unwrap();
        let first = store.list_nodes().await.unwrap();
        assert_eq!(first.iter().map(|n| n.id.clone()).collect::<Vec<_>>(), [a.clone(), b.clone()]);
        let mut names = first[1].hosted.clone();
        names.sort();
        assert_eq!(
            names,
            vec![(*OTHER).clone(), (*TYPE).clone()]
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert!(first[0].hosted.is_empty(), "a send-only node hosts nothing");
        t.advance_ticks(1).await;
        store.heartbeat_node(&a, &type_set(&[&TYPE]), &Default::default()).await.unwrap();
        let second = store.list_nodes().await.unwrap();
        assert_eq!(second[0].started_at, first[0].started_at, "started_at stays");
        assert!(second[0].last_seen > first[0].last_seen);
        assert_eq!(second[0].hosted, vec![(*TYPE).clone()], "the hosted types are replaced");
        store.remove_node(&a).await.unwrap();
        store.remove_node(&a).await.unwrap(); // a no-op
        assert_eq!(store.list_nodes().await.unwrap().iter().map(|n| n.id.clone()).collect::<Vec<_>>(), [b]);
    })
    .await
}

/// nodes - any heartbeat removes rows last seen more than a day ago
pub async fn nodes_any_heartbeat_removes_rows_last_seen_more_than_a_day_ago<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let old = node("old");
        let live = node("live");
        store.heartbeat_node(&old, &BTreeSet::new(), &Default::default()).await.unwrap();
        t.advance(std::time::Duration::from_secs(24 * 3600)).await;
        store.heartbeat_node(&live, &BTreeSet::new(), &Default::default()).await.unwrap();
        assert_eq!(store.list_nodes().await.unwrap().len(), 2, "exactly a day old stays");
        t.advance(std::time::Duration::from_micros(1)).await;
        store.heartbeat_node(&live, &BTreeSet::new(), &Default::default()).await.unwrap();
        assert_eq!(store.list_nodes().await.unwrap().iter().map(|n| n.id.clone()).collect::<Vec<_>>(), [live]);
    })
    .await
}

/// nodes - a heartbeat replaces the details, Unicode included, and absent ones clear them
pub async fn nodes_a_heartbeat_replaces_the_details_unicode_included_and_absent_ones_clear_them<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = node("node-a");
        let details = |hostname: Option<&str>, version: Option<&str>, label: Option<&str>| NodeDetails {
            hostname: hostname.map(str::to_owned),
            cicada_version: version.map(str::to_owned),
            label: label.map(str::to_owned),
        };
        let row = async || store.list_nodes().await.unwrap().remove(0);
        let first = details(Some("hôst-äß-東京"), Some("0.19.0"), Some("  billing 🦀 v2  "));
        store.heartbeat_node(&a, &BTreeSet::new(), &first).await.unwrap();
        let written = row().await;
        assert_eq!(
            (written.hostname.as_deref(), written.cicada_version.as_deref(), written.label.as_deref()),
            (Some("hôst-äß-東京"), Some("0.19.0"), Some("  billing 🦀 v2  ")),
            "kept as given"
        );
        t.advance_ticks(1).await;
        store.heartbeat_node(&a, &BTreeSet::new(), &details(Some("other"), Some("0.19.1"), None)).await.unwrap();
        let replaced = row().await;
        assert_eq!(replaced.started_at, written.started_at, "started_at stays");
        assert_eq!(
            (replaced.hostname.as_deref(), replaced.cicada_version.as_deref(), replaced.label),
            (Some("other"), Some("0.19.1"), None),
            "an absent label clears the stored one"
        );
        store.heartbeat_node(&a, &BTreeSet::new(), &NodeDetails::default()).await.unwrap();
        let cleared = row().await;
        assert_eq!((cleared.hostname, cleared.cicada_version, cleared.label), (None, None, None));
    })
    .await
}

/// turns - running turns are valid leases, by node and prefix, in address order across pages
pub async fn turns_running_turns_are_valid_leases_by_node_and_prefix_in_address_order_across_pages<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a1 = t.new_actor_named(&TYPE, "a1", json!({})).await;
        let a2 = t.new_actor_named(&TYPE, "a2", json!({})).await;
        let a3 = t.new_actor_named(&TYPE, "a3", json!({})).await;
        let idle = t.new_actor_named(&TYPE, "b", json!({})).await;
        for addr in [&a1, &a2, &a3] {
            t.send(addr).await;
        }
        let mut by_a = t.claim_with(&[&TYPE], &NODE_A, LONG_LEASE, 2).await;
        by_a.sort_by(|x, y| x.actor.as_str().cmp(y.actor.as_str()));
        let by_b = t.claim_with(&[&TYPE], &NODE_B, LONG_LEASE, 1).await;
        t.send(&idle).await;

        let all = store.running_turns(None, "", None, 100).await.unwrap();
        assert_eq!(all.items.iter().map(|r| r.actor.clone()).collect::<Vec<_>>(), [a1.clone(), a2.clone(), a3.clone()]);
        assert!(all.next.is_none());

        let first = store.running_turns(None, "", None, 2).await.unwrap();
        assert_eq!(first.items.len(), 2);
        let second = store.running_turns(None, "", first.next.as_ref(), 2).await.unwrap();
        assert_eq!(second.items.iter().map(|r| &r.actor).collect::<Vec<_>>(), [&a3]);
        assert!(second.next.is_none());

        let of_b = store.running_turns(Some(&NODE_B), "", None, 100).await.unwrap();
        assert_eq!(of_b.items.len(), 1);
        assert_eq!(of_b.items[0].node, *NODE_B);
        assert_eq!(of_b.items[0].actor, by_b[0].actor);
        assert_eq!(of_b.items[0].message_id, by_b[0].message.id);
        assert_eq!(of_b.items[0].lease_until, by_b[0].lease_until);
        assert!(!of_b.items[0].cancel_requested);

        let prefixed = store.running_turns(None, &format!("{}/a2", TYPE.as_str()), None, 100).await.unwrap();
        assert_eq!(prefixed.items.iter().map(|r| &r.actor).collect::<Vec<_>>(), [&a2]);

        assert_eq!(store.running_turn(&a2).await.unwrap().map(|r| r.actor), Some(a2.clone()));
        assert_eq!(store.running_turn(&idle).await.unwrap(), None, "pending but not running");
        assert_eq!(store.running_turn(&address("nosuch/x")).await.unwrap(), None);

        if t.fixture.is_virtual_time() {
            t.expire_long_lease().await;
            assert!(store.running_turns(None, "", None, 100).await.unwrap().items.is_empty(), "expired leases are out");
            assert_eq!(store.running_turn(&a2).await.unwrap(), None);
        }
    })
    .await
}

/// turns - a cancel request is recorded only under a valid lease and reported only for its epoch
pub async fn turns_a_cancel_request_is_recorded_only_under_a_valid_lease_and_reported_only_for_its_epoch<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        assert!(!store.request_cancel(&a).await.unwrap(), "no lease: nothing to cancel");
        let c = t.claim_one_long().await;
        assert!(store.request_cancel(&a).await.unwrap());
        assert!(store.running_turn(&a).await.unwrap().expect("running").cancel_requested);

        let before = store.running_turn(&a).await.unwrap().expect("running").lease_until;
        t.advance_ticks(1).await;
        match store.renew(&a, c.epoch, LONG_LEASE).await.unwrap() {
            RenewResult::CancelRequested(until) => assert!(until > before, "the lease is still renewed"),
            other => panic!("expected CancelRequested, got {other:?}"),
        }
        t.commit_ok(ending(&c, TurnOutcome::Cancelled)).await;
        assert_eq!(status_of(&*store, &c.message.id).await, Some(OutcomeStatus::Cancelled));

        // A later turn is not hit by the old request.
        t.send(&a).await;
        let next = t.claim_one_long().await;
        assert!(!store.running_turn(&a).await.unwrap().expect("running").cancel_requested);
        assert!(matches!(store.renew(&a, next.epoch, t.ttl()).await.unwrap(), RenewResult::Renewed(_)));
        t.commit_ok(done(&next)).await;
    })
    .await
}

/// turns - a stale request (before a lease is lost and retaken) has no effect on the later turn
pub async fn turns_a_request_made_for_an_earlier_epoch_has_no_effect_on_a_later_turn<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let stale = t.claim_one().await;
        assert!(store.request_cancel(&a).await.unwrap());
        t.expire_lease().await;
        let next = t.claim_one().await;
        assert!(next.epoch > stale.epoch);
        assert!(!store.running_turn(&a).await.unwrap().expect("running").cancel_requested);
        assert!(matches!(store.renew(&a, next.epoch, t.ttl()).await.unwrap(), RenewResult::Renewed(_)));
    })
    .await
}

/// turns - cancelled outcome from a turn is fenced by epoch and applies no effects
pub async fn turns_a_cancelled_commit_is_fenced_by_its_epoch_and_moves_the_mailbox_on<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let stale = t.claim_one().await;
        t.expire_lease().await;
        let live = t.claim_one().await;
        assert_eq!(t.commit(ending(&stale, TurnOutcome::Cancelled)).await, CommitResult::LeaseLost);
        t.commit_ok(ending(&live, TurnOutcome::Cancelled)).await;
        assert_eq!(status_of(&*store, &live.message.id).await, Some(OutcomeStatus::Cancelled));
        assert!(store.pending(&a, None, 10).await.unwrap().items.is_empty(), "the message left the mailbox");
    })
    .await
}

/// pause - a paused actor is not claimed, messages still queue, resume makes it claimable
pub async fn pause_a_paused_actor_is_not_claimed_and_keeps_queueing_until_resumed<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        assert!(store.set_paused(&a, true).await.unwrap());
        assert!(store.get(&a).await.unwrap().expect("exists").paused);
        let m = t.send(&a).await;
        assert!(t.claim().await.is_empty(), "paused: not claimed");
        assert_eq!(t.pending(&a).await.len(), 1, "messages still queue up");
        assert_eq!(
            store.next_due_at(&typeset(), &crate::tests::owner()).await.unwrap(),
            None,
            "no due time for a paused actor"
        );
        assert!(store.set_paused(&a, false).await.unwrap());
        assert!(!store.get(&a).await.unwrap().expect("exists").paused);
        assert_eq!(t.claim_one().await.message.id, m.id, "resume makes it claimable");
    })
    .await
}

/// pause - a running turn of a paused actor finishes normally
pub async fn pause_a_running_turn_of_a_paused_actor_finishes_and_other_actors_are_unaffected<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let other = t.new_actor(&TYPE, json!({})).await;
        let running = t.send(&a).await;
        let c = t.claim_one().await;
        assert!(store.set_paused(&a, true).await.unwrap());
        t.send(&a).await;
        t.send(&other).await;
        t.commit_ok(done(&c)).await;
        assert_eq!(status_of(&*store, &running.id).await, Some(OutcomeStatus::Done));
        assert_eq!(t.claim_one().await.actor, other, "the paused actor's next message is not claimed");
    })
    .await
}

/// pause - unknown instances report false and purge none
pub async fn pause_and_purge_of_an_unknown_instance_report_it<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let absent = address("nosuch/x");
        assert!(!store.set_paused(&absent, true).await.unwrap());
        assert_eq!(store.purge(&absent).await.unwrap(), None);
    })
    .await
}

/// purge - purges every pending message except the one being handled and keeps the instance
pub async fn purge_gives_purged_to_pending_messages_keeps_the_running_one_and_the_instance<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 1})).await;
        let running = t.send(&a).await;
        let queued = t.send(&a).await;
        let delayed = t.send_at(&a, Delivery::After(t.ticks(50))).await;
        let c = t.claim_one_long().await;
        assert_eq!(store.purge(&a).await.unwrap(), Some(2));
        assert_eq!(status_of(&*store, &queued.id).await, Some(OutcomeStatus::Purged));
        assert_eq!(status_of(&*store, &delayed.id).await, Some(OutcomeStatus::Purged));
        assert_eq!(state_of(&*store, &a).await, json!({"n": 1}), "the instance stays");
        t.commit_ok(done(&c)).await;
        assert_eq!(status_of(&*store, &running.id).await, Some(OutcomeStatus::Done), "the running one finishes");
        assert_eq!(store.purge(&a).await.unwrap(), Some(0));
        assert!(
            store.next_due_at(&typeset(), &crate::tests::owner()).await.unwrap().is_none(),
            "nothing is due any more"
        );
    })
    .await
}

/// purge - purging a message whose lease expired fences the stale turn
pub async fn purge_of_a_message_whose_lease_expired_fences_the_stale_turn<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let stale = t.claim_one().await;
        t.expire_lease().await;
        assert_eq!(store.purge(&a).await.unwrap(), Some(1));
        assert_eq!(status_of(&*store, &m.id).await, Some(OutcomeStatus::Purged));
        assert_eq!(t.commit(done(&stale)).await, CommitResult::LeaseLost, "the stale turn cannot commit");
        assert!(t.claim().await.is_empty());
    })
    .await
}

/// deliver now - a delayed message becomes due at once with a new place in delivery order
pub async fn deliver_now_makes_a_delayed_message_due_at_once_with_a_new_seq<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let later = t.send_at(&a, Delivery::After(t.ticks(100))).await;
        let delayed = t.send_at(&a, Delivery::After(t.ticks(200))).await;
        assert!(t.claim().await.is_empty(), "nothing is due yet");
        let delivered = match store.deliver_now(&delayed.id).await.unwrap() {
            DeliverNowResult::Delivered(m) => m,
            other => panic!("expected Delivered, got {other:?}"),
        };
        assert!(delivered.deliver_at <= store.now().await.unwrap());
        assert!(delivered.seq > delayed.seq, "a new place in delivery order");
        let c = t.claim_one().await;
        assert_eq!(c.message.id, delayed.id, "due now, ahead of the one still delayed");
        assert!(matches!(store.status(&later.id).await.unwrap(), Some(MessageStatus::Pending(_))));
    })
    .await
}

/// deliver now - a running, finished or unknown message is refused
pub async fn deliver_now_refuses_running_finished_and_unknown_messages<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let running = t.send(&a).await;
        let c = t.claim_one_long().await;
        assert_eq!(store.deliver_now(&running.id).await.unwrap(), DeliverNowResult::Running);
        t.commit_ok(done(&c)).await;
        assert!(matches!(store.deliver_now(&running.id).await.unwrap(), DeliverNowResult::AlreadyFinished(_)));
        assert_eq!(store.deliver_now(&MessageId::random()).await.unwrap(), DeliverNowResult::NotFound);
    })
    .await
}

/// deliver now - a retried message's envelope is enqueued with the store's new seq by an ordinary enqueue
pub async fn deliver_now_keeps_the_message_order_after_an_enqueue_following_it<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let first = t.send_at(&a, Delivery::After(t.ticks(100))).await;
        let DeliverNowResult::Delivered(_) = store.deliver_now(&first.id).await.unwrap() else {
            panic!("delivered");
        };
        let second = enqueue_ok(&*store, OutgoingMessage::new(a.clone(), "Msg", json!(2))).await;
        assert_delivery_order(&[store.pending(&a, None, 1).await.unwrap().items[0].clone(), second]);
    })
    .await
}
