//! Leases, heartbeats and fencing, in virtual time with two nodes on one store (Kotlin `LeaseTest`).

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::address::ActorAddress;
use cicada::store::{
    CommitRequest, CommitResult, MessageStatus, OutcomeStatus, RenewResult, Store, StoreError, StoreResult,
};
use cicada::{ActorSystem, ActorType};
use common::*;
use tokio::time::Instant;

fn a() -> Counter {
    Counter::new("a")
}

/// The counter type with a maximum turn of 5 minutes.
fn slow() -> ActorType<CounterActor> {
    ActorType::new(CounterActor).max_turn(Duration::from_secs(300))
}

fn slow_node(store: Arc<dyn Store>, name: &str) -> ActorSystem {
    node(store, name).host(slow()).build()
}

#[derive(Default)]
struct CountRenewals(AtomicUsize);

#[async_trait]
impl Faults for CountRenewals {
    async fn renew(
        &self,
        inner: &dyn Store,
        address: &ActorAddress,
        epoch: i64,
        ttl: Duration,
    ) -> StoreResult<RenewResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        inner.renew(address, epoch, ttl).await
    }
}

#[tokio::test(start_paused = true)]
async fn heartbeat_renews_at_a_third_of_the_ttl_and_keeps_a_long_turn_alive() {
    let memory = virtual_store();
    let store = Faulty::new(memory.clone(), CountRenewals::default());
    let system = slow_node(store.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(system.send(&a(), CounterMsg::Sleep { millis: 95_000 }).await.unwrap());
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(store.faults.0.load(Ordering::SeqCst), 9); // at 10, 20, ..., 90 s
    assert_eq!(memory.outcome(&m.id).await.unwrap().unwrap().status, OutcomeStatus::Done);
    assert_eq!(state_of::<CounterState>(&*memory, &a().address()).await.log, vec!["sleep:95000", "woke"]);
}

struct LoseLease;

#[async_trait]
impl Faults for LoseLease {
    async fn renew(&self, _: &dyn Store, _: &ActorAddress, _: i64, _: Duration) -> StoreResult<RenewResult> {
        Ok(RenewResult::LeaseLost)
    }
}

#[tokio::test(start_paused = true)]
async fn losing_the_lease_cancels_the_handler_and_nothing_is_committed() {
    let memory = virtual_store();
    let system = slow_node(Faulty::new(memory.clone(), LoseLease), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(system.send(&a(), CounterMsg::Sleep { millis: 60_000 }).await.unwrap());
    let started = Instant::now();
    system.run_due_turns(1).await.unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(10)); // cancelled at the first heartbeat
    // The (real) lease is still held until it expires.
    match memory.status(&m.id).await.unwrap() {
        Some(MessageStatus::Running { message, .. }) => assert_eq!(message.attempt, 1),
        other => panic!("not running: {other:?}"),
    }
    assert_eq!(state_of::<CounterState>(&*memory, &a().address()).await, CounterState::default());
    assert_eq!(memory.get(&a().address()).await.unwrap().unwrap().version, 0);
}

/// Renewals fail (the store is unreachable); commits are recorded.
#[derive(Default)]
struct Partitioned(Mutex<Option<CommitResult>>);

#[async_trait]
impl Faults for Partitioned {
    async fn renew(&self, _: &dyn Store, _: &ActorAddress, _: i64, _: Duration) -> StoreResult<RenewResult> {
        Err(StoreError::unavailable("store unreachable"))
    }

    async fn commit(&self, inner: &dyn Store, commit: CommitRequest) -> StoreResult<CommitResult> {
        let result = inner.commit(commit).await?;
        *self.0.lock().unwrap() = Some(result.clone());
        Ok(result)
    }
}

#[tokio::test(start_paused = true)]
async fn a_node_that_cannot_renew_cancels_its_handler_before_the_lease_expires_exactly_one_outcome_effects_once() {
    let memory = virtual_store();
    let partitioned = Faulty::new(memory.clone(), Partitioned::default());
    let stale = slow_node(partitioned.clone(), "stale");
    let healthy = slow_node(memory.clone(), "healthy");
    stale.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(stale.send(&a(), CounterMsg::Sleep { millis: 45_000 }).await.unwrap());

    let started = Instant::now();
    let first = tokio::spawn({
        let stale = stale.clone();
        async move {
            stale.run_due_turns(1).await.unwrap(); // claims at t = 0, cannot renew
            started.elapsed()
        }
    });
    tokio::time::sleep(Duration::from_secs(31)).await; // the lease expired at t = 30 s
    assert!(first.is_finished(), "the stale handler was cancelled before its lease expired");
    let ended = first.await.unwrap();
    assert!(
        (Duration::from_secs(29)..=Duration::from_secs(30)).contains(&ended),
        "cancelled at the lease deadline minus the safety margin, was {ended:?}"
    );
    assert_eq!(healthy.run_due_turns(1).await.unwrap(), 1); // claims at t = 31 s (attempt 2), finishes at t = 76 s

    assert!(partitioned.faults.0.lock().unwrap().is_none(), "the stale node never tried to commit");
    let outcome = memory.outcome(&m.id).await.unwrap().unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!(outcome.envelope.attempt, 2);
    assert_eq!(state_of::<CounterState>(&*memory, &a().address()).await.log, vec!["sleep:45000", "woke"]);
    assert_eq!(memory.get(&a().address()).await.unwrap().unwrap().version, 1);
}

#[tokio::test(start_paused = true)]
async fn a_released_lease_leaves_the_message_pending_for_the_next_claim() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(system.send(&a(), CounterMsg::Sleep { millis: 60_000 }).await.unwrap());
    let job = tokio::spawn({
        let system = system.clone();
        async move { system.run_due_turns(1).await }
    });
    tokio::time::sleep(Duration::from_secs(5)).await;
    job.abort(); // e.g. shutdown after the grace period
    let _ = job.await;
    settle().await;
    match memory.status(&m.id).await.unwrap() {
        Some(MessageStatus::Pending(message)) => assert_eq!(message.attempt, 1),
        other => panic!("not pending: {other:?}"),
    }
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1); // claimable immediately, no TTL wait
    assert_eq!(memory.outcome(&m.id).await.unwrap().unwrap().envelope.attempt, 2);
}
