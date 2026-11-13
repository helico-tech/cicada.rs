//! Edge cases of S1–S10: time bounds and rounding, clock step-back, lease boundaries,
//! repeated (ambiguous) commits and id races between a commit and concurrent writes.

use std::time::Duration;

use crate::store::{
    ActorCreate, CancelResult, CommitRequest, CommitResult, Delivery, EnqueueResult, MessageId, MessageStatus,
    OutcomeStatus, OutgoingMessage, RenewResult, StateChange, StoreError,
};
use crate::time::{self, MAX_INSTANT, Timestamp};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

/// Delivery times beyond the supported range, up to chrono's latest instant (year 262143).
fn too_late() -> [Delivery; 3] {
    [
        Delivery::At(DateTime::<Utc>::MAX_UTC),
        Delivery::After(Duration::MAX),
        Delivery::After(Duration::from_secs(36_500_000 * 86_400)),
    ]
}

/// time - a delivery time beyond the supported range is refused and nothing is enqueued
pub async fn time_a_delivery_time_beyond_the_supported_range_is_refused_and_nothing_is_enqueued<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        for delivery in too_late() {
            let message = OutgoingMessage { delivery, ..OutgoingMessage::new(a.clone(), "Msg", json!(1)) };
            assert_matches!(store.enqueue(message).await, Err(StoreError::Invalid(_)), "{delivery:?}");
        }
        let m = t.send(&a).await;
        let c = t.claim_one_long().await;
        for delivery in too_late() {
            let message = OutgoingMessage { delivery, ..OutgoingMessage::new(a.clone(), "Msg", json!(2)) };
            assert_matches!(
                store.commit(CommitRequest { sends: vec![message], ..done(&c) }).await,
                Err(StoreError::Invalid(_)),
                "{delivery:?}"
            );
        }
        assert_eq!(ids(&t.pending(&a).await), vec![m.id.clone()], "nothing was enqueued");
        assert_matches!(
            store.status(&m.id).await.unwrap(),
            Some(MessageStatus::Running { .. }),
            "the lease is still held"
        );
        t.commit_ok(done(&c)).await;
        assert_eq!(t.claim().await, vec![]);
    })
    .await
}

/// time - the latest supported delivery time is stored exactly and never due
pub async fn time_the_latest_supported_delivery_time_is_stored_exactly_and_never_due<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let max = time::parse_instant("9999-12-31T23:59:59.999999Z").unwrap();
        assert_eq!(max, MAX_INSTANT);
        let m = t.send_at(&a, Delivery::At(max)).await;
        assert_eq!(m.deliver_at, max);
        assert_eq!(t.pending(&a).await, vec![m.clone()]);
        assert_eq!(store.next_due_at(&type_set(&[&TYPE])).await.unwrap(), Some(max));
        let status = store.status(&m.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Pending(p)) => p).deliver_at, max);
        assert_eq!(t.claim().await, vec![]);
    })
    .await
}

/// time - a sub-microsecond delivery time is rounded up, never down
pub async fn time_a_sub_microsecond_delivery_time_is_rounded_up_never_down<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let a = t.new_actor(&TYPE, json!({})).await;
        let now = t.store().now().await.unwrap();
        let at = t.send_at(&a, Delivery::At(plus(now, Duration::from_nanos(500)))).await;
        let after = t.send_at(&a, Delivery::After(Duration::from_nanos(1))).await;
        assert_eq!(at.deliver_at, plus(now, Duration::from_micros(1)));
        assert_eq!(after.deliver_at, plus(now, Duration::from_micros(1)));
        assert_eq!(t.claim().await, vec![], "never due before the requested time");
        t.advance(Duration::from_micros(1)).await;
        assert_eq!(t.claim_one().await.message.id, at.id);
    })
    .await
}

/// time - store time stays monotonic when the underlying clock steps back
pub async fn time_store_time_stays_monotonic_when_the_underlying_clock_steps_back<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !assume(t.fixture.supports_clock_step_back(), "the fixture cannot step its clock back") {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let first = t.send(&a).await;
        let t1 = store.now().await.unwrap();
        t.fixture.step_back(Duration::from_secs(3600)).await;
        assert!(store.now().await.unwrap() >= t1, "now() went backwards");
        let second = t.send(&a).await;
        assert!(second.deliver_at >= first.deliver_at, "a new message jumped ahead of a due one");
        let delayed = t.send_at(&a, Delivery::After(t.tick)).await;
        assert!(delayed.deliver_at >= plus(t1, t.tick), "a delay is measured from monotonic store time");
        let c1 = t.claim_one().await;
        assert!(c1.claimed_at >= t1);
        assert!(c1.lease_until >= plus(t1, t.ttl()));
        t.commit_ok(done(&c1)).await;
        assert!(outcome_record(&*store, &first.id).await.expect("an outcome").finished_at >= t1);
        let c2 = t.claim_one().await;
        assert_eq!(c2.message.id, second.id);
        t.commit_ok(done(&c2)).await;
        assert_eq!(ids(&t.pending(&a).await), vec![delayed.id.clone()]);
        if t.fixture.supports_reopen() {
            t.reopen().await;
            assert!(t.store().now().await.unwrap() >= t1, "store time stays monotonic across a restart");
        }
    })
    .await
}

/// S4 - a lease is valid strictly before leaseUntil and expired at it
pub async fn s4_a_lease_is_valid_strictly_before_lease_until_and_expired_at_it<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        let now = store.now().await.unwrap();
        t.advance(time::between(now, c.lease_until) - Duration::from_micros(1)).await;
        assert_eq!(t.claim_node(&NODE_B).await, vec![], "valid one microsecond before leaseUntil");
        assert_matches!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. }));
        assert_eq!(store.cancel(&m.id).await.unwrap(), CancelResult::Running);
        assert_eq!(store.next_due_at(&type_set(&[&TYPE])).await.unwrap(), Some(c.lease_until));
        t.advance(Duration::from_micros(1)).await;
        assert_eq!(store.now().await.unwrap(), c.lease_until);
        assert_matches!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(_)), "expired at leaseUntil");
        let again = t.claim_one_node(&NODE_B).await;
        assert!(again.epoch > c.epoch);
        assert_eq!(again.message.attempt, 2);
    })
    .await
}

/// S5 - renew and commit still work after expiry as long as nobody re-claimed
pub async fn s5_renew_and_commit_still_work_after_expiry_as_long_as_nobody_re_claimed<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        t.expire_lease().await;
        assert_matches!(
            store.status(&m.id).await.unwrap(),
            Some(MessageStatus::Pending(_)),
            "an expired lease is not running"
        );
        let renewed =
            expect_matches!(store.renew(&a, c.epoch, t.ticks(8)).await.unwrap(), RenewResult::Renewed(until) => until);
        assert_matches!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. }), "renewed again");
        // A shorter renewal keeps the later expiry; the lease then runs out past that retained expiry.
        let retained =
            expect_matches!(store.renew(&a, c.epoch, t.ttl()).await.unwrap(), RenewResult::Renewed(until) => until);
        assert_eq!(retained, renewed, "a shorter renewal keeps the held expiry");
        t.advance(time::between(store.now().await.unwrap(), retained) + t.tick).await;
        assert_matches!(
            store.status(&m.id).await.unwrap(),
            Some(MessageStatus::Pending(_)),
            "expired again before the commit"
        );
        t.commit_ok(done(&c)).await;
        assert_eq!(outcome_record(&*store, &m.id).await.map(|o| o.status), Some(OutcomeStatus::Done));
    })
    .await
}

/// S5 - a renewal never shortens a held lease: a shorter one keeps the expiry, a longer one extends it
pub async fn s5_a_renewal_never_shortens_a_held_lease_and_a_longer_one_extends_it<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let c = t.claim_one_ttl(t.ticks(12)).await;
        t.advance_ticks(2).await;
        let short =
            expect_matches!(store.renew(&a, c.epoch, t.ticks(1)).await.unwrap(), RenewResult::Renewed(until) => until);
        assert_eq!(short, c.lease_until, "a shorter renewal keeps the held expiry");
        assert_eq!(store.running_turn(&a).await.unwrap().expect("running").lease_until, c.lease_until);
        t.advance_ticks(3).await; // past what the short renewal asked for, before the held expiry
        assert!(t.claim_node(&NODE_B).await.is_empty(), "the retained lease stays exclusive");

        let before = store.now().await.unwrap();
        let long =
            expect_matches!(store.renew(&a, c.epoch, t.ticks(16)).await.unwrap(), RenewResult::Renewed(until) => until);
        assert!(long >= plus(before, t.ticks(16)) && long > c.lease_until, "a longer renewal extends the lease");
        if t.fixture.is_virtual_time() {
            assert_eq!(long, plus(c.claimed_at, t.ticks(21)));
        }
        assert_eq!(store.running_turn(&a).await.unwrap().expect("running").lease_until, long);
        let again =
            expect_matches!(store.renew(&a, c.epoch, t.ticks(1)).await.unwrap(), RenewResult::Renewed(until) => until);
        assert_eq!(again, long, "and a shorter one after it keeps that");
    })
    .await
}

/// S5 - a shorter renewal of a lease with a cancel request still reports it, with the retained expiry
pub async fn s5_a_shorter_renewal_still_reports_the_cancel_request_with_the_retained_expiry<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let c = t.claim_one_ttl(t.ticks(12)).await;
        assert!(store.request_cancel(&a).await.unwrap());
        t.advance_ticks(1).await;
        let short = expect_matches!(
            store.renew(&a, c.epoch, t.ticks(1)).await.unwrap(),
            RenewResult::CancelRequested(until) => until
        );
        assert_eq!(short, c.lease_until, "the retained expiry");
        let long = expect_matches!(
            store.renew(&a, c.epoch, t.ticks(20)).await.unwrap(),
            RenewResult::CancelRequested(until) => until
        );
        assert!(long > c.lease_until, "a longer renewal extends it and still reports the request");
        assert_eq!(store.running_turn(&a).await.unwrap().expect("running").lease_until, long);
    })
    .await
}

/// S5 - concurrent shorter and longer renewals of one lease never shorten it
pub async fn s5_concurrent_shorter_and_longer_renewals_never_shorten_the_lease<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let c = t.claim_one_ttl(t.ticks(12)).await;
        let mut floor = c.lease_until;
        let epoch = c.epoch;
        for round in 0..4u32 {
            let tick = t.tick;
            let results = t
                .concurrently(8, |i, peer| {
                    let a = a.clone();
                    // Every other renewal is shorter than the lease it finds; the longer ones grow per round.
                    let ttl = if i % 2 == 0 { tick } else { tick * (13 + round * 4 + i as u32) };
                    async move { peer.renew(&a, epoch, ttl).await.unwrap() }
                })
                .await;
            let untils: Vec<Timestamp> = results
                .iter()
                .map(|r| expect_matches!(r, RenewResult::Renewed(until) => *until, "round {round}"))
                .collect();
            assert!(untils.iter().all(|until| *until >= floor), "no renewal went below the held expiry: {untils:?}");
            let stored = store.running_turn(&a).await.unwrap().expect("running").lease_until;
            assert_eq!(Some(&stored), untils.iter().max(), "the stored expiry is the latest any renewal returned");
            floor = stored;
        }
        assert!(t.claim_node(&NODE_B).await.is_empty(), "the lease stays exclusive");
    })
    .await
}

/// S5 - a stale renewal leaves a newer lease as it is; an expired lease nobody replaced is renewed from store now
pub async fn s5_a_stale_renewal_leaves_a_newer_lease_and_an_expired_held_one_renews_from_now<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let old = t.claim_one().await;
        t.expire_lease().await;
        let new = t.claim_one_node(&NODE_B).await;
        assert_eq!(store.renew(&a, old.epoch, LONG_LEASE).await.unwrap(), RenewResult::LeaseLost);
        assert_eq!(store.running_turn(&a).await.unwrap().expect("running").lease_until, new.lease_until);

        t.expire_lease().await;
        let before = store.now().await.unwrap();
        let renewed =
            expect_matches!(store.renew(&a, new.epoch, t.ttl()).await.unwrap(), RenewResult::Renewed(until) => until);
        assert!(renewed >= plus(before, t.ttl()) && renewed > new.lease_until, "renewed from store now");
    })
    .await
}

/// commit - repeating a commit that already applied changes nothing
pub async fn commit_repeating_a_commit_that_already_applied_changes_nothing<B: FixtureFactory>(backend: &B) {
    // A commit whose acknowledgement was lost (e.g. a network error after COMMIT) is retried by the runtime with the
    // same epoch and message id. It must neither apply twice nor fail in a way that re-runs the handler.
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 0})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let child = address("other/repeated-child");
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        let request = CommitRequest {
            state: StateChange::NewState(json!({"n": 1})),
            creates: vec![ActorCreate { address: child.clone(), state: json!({"c": 1}), child: false }],
            sends: vec![
                OutgoingMessage::new(b.clone(), "T", json!(1)),
                OutgoingMessage::new(child.clone(), "T", json!(2)),
            ],
            ..done(&c)
        };
        let first = t.commit_ok(request.clone()).await;
        assert_eq!(t.commit(request).await, CommitResult::LeaseLost);
        assert_eq!(store.get(&a).await.unwrap().expect("the instance").version, 1);
        assert_eq!(ids(&t.pending(&b).await), vec![first.sent[0].id.clone()]);
        assert_eq!(ids(&t.pending(&child).await), vec![first.sent[1].id.clone()]);
        assert_eq!(outcome_record(&*store, &m.id).await.map(|o| o.status), Some(OutcomeStatus::Done));

        // Repeated while the next turn holds the lease, and after a stop.
        let m2 = t.send(&a).await;
        let c2 = t.claim_one().await;
        assert_eq!(
            t.commit(CommitRequest { state: StateChange::NewState(json!({"n": 9})), ..done(&c) }).await,
            CommitResult::LeaseLost
        );
        t.commit_ok(CommitRequest { state: StateChange::Stop, ..done(&c2) }).await;
        assert_eq!(t.commit(CommitRequest { state: StateChange::Stop, ..done(&c2) }).await, CommitResult::LeaseLost);
        assert_eq!(store.get(&a).await.unwrap(), None, "a repeated stop does not resurrect or touch the instance");
        assert_eq!(outcome_record(&*store, &m2.id).await.map(|o| o.status), Some(OutcomeStatus::Done));
    })
    .await
}

/// S6 - equal deliverAt keeps enqueue order on any clock
pub async fn s6_equal_deliver_at_keeps_enqueue_order_on_any_clock<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let at = plus(t.store().now().await.unwrap(), t.ticks(5));
        let mut msgs = Vec::new();
        for _ in 0..6 {
            msgs.push(t.send_at(&a, Delivery::At(at)).await);
        }
        assert!(msgs.iter().all(|m| m.deliver_at == at), "a future deliverAt is kept exactly");
        t.advance_ticks(6).await;
        let mut order = Vec::new();
        for _ in 0..6 {
            let c = t.claim_one().await;
            t.commit_ok(done(&c)).await;
            order.push(c.message.id);
        }
        assert_eq!(order, ids(&msgs));
    })
    .await
}

/// G5 - a message is never claimed before its deliverAt on any clock
pub async fn g5_a_message_is_never_claimed_before_its_deliver_at_on_any_clock<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let mut msgs = Vec::new();
        for i in 1..=4 {
            msgs.push(t.send_at(&a, Delivery::After(t.ticks(i))).await);
        }
        let mut handled: Vec<MessageId> = Vec::new();
        while handled.len() < msgs.len() {
            for c in t.claim().await {
                assert!(
                    c.claimed_at >= c.message.deliver_at,
                    "claimed at {}, due at {}",
                    c.claimed_at,
                    c.message.deliver_at
                );
                handled.push(c.message.id.clone());
                t.commit_ok(done(&c)).await;
            }
            t.advance(t.tick / 4).await;
        }
        assert_eq!(handled, ids(&msgs));
    })
    .await
}

/// The result of one of two racing writes.
#[derive(Debug)]
enum Raced {
    Commit(CommitResult),
    Enqueue(EnqueueResult),
    Cancel(CancelResult),
}

/// S7 - a commit and a concurrent enqueue racing for one id accept exactly one
pub async fn s7_a_commit_and_a_concurrent_enqueue_racing_for_one_id_accept_exactly_one<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        for round in 0..10 {
            let a = t.new_actor(&TYPE, json!({})).await;
            let b = t.new_actor(&OTHER, json!({})).await;
            t.send(&a).await;
            let c = t.claim_one().await;
            let id = MessageId::random();
            let results = t
                .concurrently(2, |i, peer| {
                    let (c, b, id) = (c.clone(), b.clone(), id.clone());
                    async move {
                        if i == 0 {
                            let send = OutgoingMessage { id, ..OutgoingMessage::new(b, "FromCommit", json!(1)) };
                            Raced::Commit(peer.commit(CommitRequest { sends: vec![send], ..done(&c) }).await.unwrap())
                        } else {
                            let send = OutgoingMessage { id, ..OutgoingMessage::new(b, "External", json!(2)) };
                            Raced::Enqueue(peer.enqueue(send).await.unwrap())
                        }
                    }
                })
                .await;
            let committed = matches!(results[0], Raced::Commit(CommitResult::Committed { .. }));
            let enqueued = matches!(results[1], Raced::Enqueue(EnqueueResult::Enqueued(_)));
            assert!(committed != enqueued, "round {round}: exactly one may take the id, got {results:?}");
            if !committed {
                assert_matches!(&results[0], Raced::Commit(CommitResult::Rejected(_)));
                t.commit_ok(done(&c)).await; // the rejected commit kept the lease
            } else {
                assert!(
                    matches!(&results[1], Raced::Enqueue(EnqueueResult::Duplicate(d)) if *d == id),
                    "expected Duplicate({id}), got {:?}",
                    results[1]
                );
            }
            assert_eq!(t.pending(&b).await.iter().filter(|m| m.id == id).count(), 1);
        }
    })
    .await
}

/// S7 - two commits racing for one send id accept exactly one
pub async fn s7_two_commits_racing_for_one_send_id_accept_exactly_one<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        for round in 0..10 {
            let a1 = t.new_actor(&TYPE, json!({})).await;
            let a2 = t.new_actor(&TYPE, json!({})).await;
            let b = t.new_actor(&OTHER, json!({})).await;
            t.send(&a1).await;
            t.send(&a2).await;
            let claims = t.claim_limit(2).await;
            assert_eq!(claims.len(), 2);
            let id = MessageId::random();
            let results = t
                .concurrently(2, |i, peer| {
                    let (c, b, id) = (claims[i].clone(), b.clone(), id.clone());
                    async move {
                        let send = OutgoingMessage { id, ..OutgoingMessage::new(b, format!("T{i}"), json!(i)) };
                        peer.commit(CommitRequest { sends: vec![send], ..done(&c) }).await.unwrap()
                    }
                })
                .await;
            let committed = results.iter().filter(|r| matches!(r, CommitResult::Committed { .. })).count();
            let rejected = results.iter().filter(|r| matches!(r, CommitResult::Rejected(_))).count();
            assert_eq!(committed, 1, "round {round}: {results:?}");
            assert_eq!(rejected, 1, "round {round}: {results:?}");
            assert_eq!(t.pending(&b).await.iter().filter(|m| m.id == id).count(), 1);
        }
    })
    .await
}

/// S7 - cancel racing a commit that sends the same id never loses the message
pub async fn s7_cancel_racing_a_commit_that_sends_the_same_id_never_loses_the_message<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..10 {
            let a = t.new_actor(&TYPE, json!({})).await;
            let b = t.new_actor(&OTHER, json!({})).await;
            t.send(&a).await;
            let c = t.claim_one().await;
            let id = MessageId::random();
            let results = t
                .concurrently(2, |i, peer| {
                    let (c, b, id) = (c.clone(), b.clone(), id.clone());
                    async move {
                        if i == 0 {
                            let send = OutgoingMessage { id, ..OutgoingMessage::new(b, "T", json!(1)) };
                            Raced::Commit(peer.commit(CommitRequest { sends: vec![send], ..done(&c) }).await.unwrap())
                        } else {
                            Raced::Cancel(peer.cancel(&id).await.unwrap())
                        }
                    }
                })
                .await;
            assert_matches!(&results[0], Raced::Commit(CommitResult::Committed { .. }), "round {round}");
            match &results[1] {
                Raced::Cancel(CancelResult::Cancelled(_)) => {
                    assert_eq!(outcome_record(&*store, &id).await.map(|o| o.status), Some(OutcomeStatus::Cancelled))
                }
                Raced::Cancel(CancelResult::NotFound) => {
                    assert_matches!(store.status(&id).await.unwrap(), Some(MessageStatus::Pending(_)))
                }
                other => panic!("round {round}: unexpected cancel result {other:?}"),
            }
        }
    })
    .await
}

/// S3 - a commit that creates and sends to a child is serialised with concurrent writes to that child
pub async fn s3_a_commit_that_creates_and_sends_to_a_child_is_serialised_with_concurrent_writes_to_that_child<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        t.register_type(&OTHER).await;
        for round in 0..8 {
            let parent = t.new_actor(&TYPE, json!({})).await;
            t.send(&parent).await;
            let c = t.claim_one().await;
            let child = address(&format!("other/race-{round}"));
            t.concurrently(2, |i, peer| {
                let (c, child) = (c.clone(), child.clone());
                async move {
                    if i == 0 {
                        peer.commit(CommitRequest {
                            creates: vec![ActorCreate { address: child.clone(), state: json!({}), child: false }],
                            sends: vec![OutgoingMessage::new(child, "FromCommit", json!(0))],
                            ..done(&c)
                        })
                        .await
                        .unwrap();
                    } else {
                        peer.create(&child, json!({})).await.unwrap();
                        for k in 0..3 {
                            peer.enqueue(OutgoingMessage::new(child.clone(), "External", json!(k))).await.unwrap();
                        }
                    }
                }
            })
            .await;
            let pending = t.pending(&child).await;
            assert_eq!(pending.len(), 4, "round {round}");
            let mut by_seq = pending.clone();
            by_seq.sort_by_key(|m| m.seq);
            assert_eq!(
                by_seq, pending,
                "round {round}: immediate sends must be ordered by deliverAt as by seq (no stale clock reading)"
            );
        }
    })
    .await
}

/// closed - after close, every operation of the store fails as unavailable
pub async fn closed_after_close_every_operation_fails_as_unavailable<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        let (types, node) = (type_set(&[&TYPE]), NODE_A.clone());
        let since = store.now().await.unwrap();
        store.close().await;

        macro_rules! unavailable {
            ($($call:expr),* $(,)?) => {$(
                match $call.await {
                    Err(e) => assert!(e.is_unavailable(), "{}: expected unavailable, got {e:?}", stringify!($call)),
                    Ok(value) => panic!("{}: expected unavailable, got {value:?}", stringify!($call)),
                }
            )*};
        }
        unavailable!(
            store.now(),
            store.register(type_def(&OTHER, "closed")),
            store.list_types(),
            store.remove_type(&OTHER),
            store.create(&address("conformance/closed"), json!({})),
            store.get(&a),
            store.list("", None, 10),
            store.children(&a, None, 10),
            store.delete(&a, &MessageId::random()),
            store.enqueue(OutgoingMessage::new(a.clone(), "Msg", json!(0))),
            store.claim_due(&types, &node, t.ttl(), 10),
            store.renew(&a, c.epoch, t.ttl()),
            store.next_due_at(&types),
            store.pending(&a, None, 10),
            store.list_pending("", None, 10),
            store.deliver_now(&m.id),
            store.cancel(&m.id),
            store.set_paused(&a, true),
            store.purge(&a),
            store.status(&m.id),
            store.outcomes(None, "", since, None, 10),
            store.caused_by(&m.id, None, 10),
            store.stats(""),
            store.sweep_expired(),
            store.heartbeat_node(&node, &types),
            store.list_nodes(),
            store.remove_node(&node),
            store.running_turns(None, "", None, 10),
            store.running_turn(&a),
            store.request_cancel(&a),
            store.commit(done(&c)),
        );
        // `release` reports nothing; on a closed store it returns without panicking or hanging.
        let _ = store.release(&a, c.epoch).await;
    })
    .await
}
