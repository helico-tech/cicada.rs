//! A turn never outlives its lease (local lease deadline), commits are retried instead of re-running the handler, and
//! claimed turns are never left behind. Virtual time, two nodes on one store.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use super::common::*;
use crate::address::{ActorAddress, TypeName};
use crate::store::{
    Backend, Claim, CommitRequest, CommitResult, MessageStatus, NodeId, OutcomeStatus, RenewResult, StoreError,
    StoreResult,
};
use crate::{ActorSystem, Unhandled};
use async_trait::async_trait;
use tokio::time::{Instant, sleep};

fn a() -> Counter {
    Counter::new("a")
}

// The counter type with a maximum turn of 5 minutes.
counter_type!(SlowCounter, Unhandled::Fail, Duration::from_secs(300));

fn slow_node(store: impl IntoStore, name: &str) -> ActorSystem {
    node(store, name).host(SlowCounter).build()
}

async fn log_of(memory: &dyn Backend) -> Vec<String> {
    state_of::<CounterState>(memory, &a().address()).await.log
}

/// A process pause (GC, SIGSTOP) right after the handler returned: neither heartbeats nor the commit run until it is
/// over.
#[derive(Default)]
struct Pausing {
    paused: AtomicBool,
    stale_commit: Mutex<Option<CommitResult>>,
}

#[async_trait]
impl Faults for Pausing {
    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        self.paused.store(true, Ordering::SeqCst);
        sleep(Duration::from_secs(40)).await;
        let result = inner.commit(commit).await?;
        *self.stale_commit.lock().unwrap() = Some(result.clone());
        Ok(result)
    }

    async fn renew(
        &self,
        inner: &dyn Backend,
        address: &ActorAddress,
        epoch: i64,
        ttl: Duration,
    ) -> StoreResult<RenewResult> {
        if self.paused.load(Ordering::SeqCst) {
            sleep(Duration::from_secs(40)).await;
        }
        inner.renew(address, epoch, ttl).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_stale_commit_after_a_pause_is_fenced_off_exactly_one_outcome_effects_once() {
    let memory = virtual_store();
    let pausing = Faulty::new(memory.clone(), Pausing::default());
    let stale = slow_node(pausing.clone(), "stale");
    let healthy = slow_node(memory.clone(), "healthy");
    stale.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(stale.send(&a(), CounterMsg::Sleep { millis: 5_000 }).await.unwrap());

    // Claims at t = 0, the handler ends at 5 s, the commit is paused until 45 s.
    let first = tokio::spawn({
        let stale = stale.clone();
        async move { stale.run_due_turns(1).await.unwrap() }
    });
    sleep(Duration::from_secs(31)).await;
    assert_eq!(healthy.run_due_turns(1).await.unwrap(), 1); // claims at t = 31 s (attempt 2), commits at 36 s
    first.await.unwrap();

    assert_eq!(*pausing.faults.stale_commit.lock().unwrap(), Some(CommitResult::LeaseLost));
    let outcome = outcome_of(&memory, &m.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!(outcome.envelope.attempt, 2);
    assert_eq!(log_of(&memory).await, vec!["sleep:5000", "woke"]);
    assert_eq!(memory.get(&a().address()).await.unwrap().unwrap().version, 1);
}

/// Commits fail while `failures` is positive (counting down); with `after`, the failure comes after the commit applied.
struct FlakyCommit {
    failures: AtomicI32,
    after: bool,
}

#[async_trait]
impl Faults for FlakyCommit {
    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        let fail = self.failures.fetch_sub(1, Ordering::SeqCst) > 0;
        if fail && !self.after {
            return Err(StoreError::unavailable("connection reset"));
        }
        let result = inner.commit(commit).await?;
        if fail {
            return Err(StoreError::unavailable("connection reset after COMMIT"));
        }
        Ok(result)
    }
}

#[tokio::test(start_paused = true)]
async fn a_commit_that_fails_is_retried_and_the_handler_is_not_run_again() {
    let memory = virtual_store();
    let system =
        slow_node(Faulty::new(memory.clone(), FlakyCommit { failures: AtomicI32::new(2), after: false }), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(system.send(&a(), CounterMsg::Add { n: 1 }).await.unwrap());
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&memory, &m.id).await.expect("committed by a retry");
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!(outcome.envelope.attempt, 1);
    assert_eq!(log_of(&memory).await, vec!["add:1"]);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 0, "nothing left to re-run");
}

#[tokio::test(start_paused = true)]
async fn a_commit_whose_acknowledgement_was_lost_is_not_applied_twice() {
    let memory = virtual_store();
    let system = slow_node(Faulty::new(memory.clone(), FlakyCommit { failures: AtomicI32::new(1), after: true }), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(system.send(&a(), CounterMsg::Burst { count: 2 }).await.unwrap());
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(outcome_of(&memory, &m.id).await.map(|o| o.status), Some(OutcomeStatus::Done));
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 0, "the burst's two sends were enqueued once");
    assert_eq!(log_of(&memory).await, vec!["add:1", "add:2"]);
}

struct BrokenCommit;

#[async_trait]
impl Faults for BrokenCommit {
    async fn commit(&self, _: &dyn Backend, _: CommitRequest) -> StoreResult<CommitResult> {
        Err(StoreError::unavailable("disk full"))
    }
}

#[tokio::test(start_paused = true)]
async fn a_commit_that_keeps_failing_is_abandoned_without_releasing_the_lease() {
    let memory = virtual_store();
    let stale = slow_node(Faulty::new(memory.clone(), BrokenCommit), "broken");
    let healthy = slow_node(memory.clone(), "healthy");
    stale.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(stale.send(&a(), CounterMsg::Add { n: 1 }).await.unwrap());
    let started = Instant::now();
    assert_eq!(stale.run_due_turns(1).await.unwrap(), 1);
    assert!(started.elapsed() <= Duration::from_secs(31), "commit retries are bounded, took {:?}", started.elapsed());
    // Not released: the handler returned, so only lease expiry may hand the message to another node (recovery of a
    // turn that never committed), not an immediate re-run.
    assert!(matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })));
    assert_eq!(healthy.run_due_turns(1).await.unwrap(), 0);
    sleep(Duration::from_secs(31)).await;
    assert_eq!(healthy.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&memory, &m.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!(outcome.envelope.attempt, 2);
}

#[tokio::test(start_paused = true)]
async fn run_due_turns_runs_the_turns_it_claimed_concurrently() {
    let memory = virtual_store();
    let system = slow_node(memory.clone(), "n1");
    let b = Counter::new("b");
    system.create(&a(), &CounterState::default()).await.unwrap();
    system.create(&b, &CounterState::default()).await.unwrap();
    system.send(&a(), CounterMsg::Sleep { millis: 25_000 }).await.unwrap();
    system.send(&b, CounterMsg::Sleep { millis: 25_000 }).await.unwrap();
    let started = Instant::now();
    assert_eq!(system.run_due_turns(2).await.unwrap(), 2);
    assert_eq!(started.elapsed(), Duration::from_secs(25), "the second claim must not wait for the first turn");
}

/// The claim's response is slow.
struct SlowClaims;

#[async_trait]
impl Faults for SlowClaims {
    async fn claim_due(
        &self,
        inner: &dyn Backend,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        let claims = inner.claim_due(types, node, ttl, limit).await?;
        sleep(Duration::from_secs(1)).await;
        Ok(claims)
    }
}

#[tokio::test(start_paused = true)]
async fn cancelling_run_due_turns_while_it_claims_releases_what_was_claimed() {
    let memory = virtual_store();
    let system = slow_node(Faulty::new(memory.clone(), SlowClaims), "n1");
    system.create(&a(), &CounterState::default()).await.unwrap();
    let m = accepted(system.send(&a(), CounterMsg::Add { n: 1 }).await.unwrap());
    let job = tokio::spawn({
        let system = system.clone();
        async move { system.run_due_turns(1).await }
    });
    sleep(Duration::from_millis(500)).await;
    job.abort();
    let _ = job.await;
    sleep(Duration::from_secs(1)).await; // the claim answers; nobody takes it
    match memory.status(&m.id).await.unwrap() {
        Some(MessageStatus::Pending(message)) => {
            assert_eq!(message.attempt, 1, "the claim was released, not left to expire")
        }
        other => panic!("not pending: {other:?}"),
    }
}
