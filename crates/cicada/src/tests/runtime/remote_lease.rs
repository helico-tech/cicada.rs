//! Faults and races of external leases (spec `turn-leases`, ADR 0037), on virtual time: a `lease_turns` future dropped
//! while its claim is in flight or while the claim limit commits, late answers, a system that stops during a lease,
//! cleanup that cannot reach the store or whose runtime ends, a shorter peer renewal whose answer is lost, a batch lost
//! after handoff, and the same peer renewal on a shared SQLite file and on PostgreSQL.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use super::common::*;
use super::remote_claim_limit::*;
use crate::address::{ActorAddress, TypeName};
use crate::runtime::{LeaseTurnsResult, NodeConfig, OpError, RenewTurnResult, TurnLease};
use crate::store::{Backend, Claim, CommitRequest, CommitResult, Message, NodeId, StoreError, StoreResult};
use crate::testing::{VirtualClock, default_start};
use crate::time::Timestamp;
use crate::{ActorSystem, SendOptions, Store};
use async_trait::async_trait;
use tokio::time::{sleep, timeout};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn plus(instant: Timestamp, duration: Duration) -> Timestamp {
    instant + chrono::Duration::from_std(duration).unwrap()
}

/// A server that hosts nothing, with the TTL `ttl`, the claim limit of 3 and an observer writing to `events`.
async fn server_with(store: impl IntoStore, name: &str, ttl: Duration, events: &Events) -> ActorSystem {
    let config = NodeConfig { lease_ttl: ttl, max_lost_claims: Some(3), ..node_config(name) };
    let system = node(store, name).config(config).observe(Recorder::new(events)).build();
    system.register_contract(&crate::actor::contract_of::<JobActor>().unwrap()).await.unwrap();
    system
}

async fn server(store: impl IntoStore, events: &Events) -> ActorSystem {
    server_with(store, "server", LEASE, events).await
}

/// Creates `job/<id>` and sends it `Run`.
async fn runnable(system: &ActorSystem, id: &str) -> Message {
    system.create(&job(id), &JobState::default()).await.unwrap();
    accepted(system.send(&job(id), JobMsg::Run).await.unwrap())
}

/// A message `job/limited` at the limit and a message `job/runnable` below it.
async fn mixed(memory: &Store, system: &ActorSystem) -> (Message, Message) {
    let limited = job_with_lost_claims(memory, system, "limited", 3, SendOptions::default()).await;
    (limited, runnable(system, "runnable").await)
}

fn one(result: LeaseTurnsResult) -> TurnLease {
    match result {
        LeaseTurnsResult::Leased(mut leases) if leases.len() == 1 => leases.remove(0),
        other => panic!("expected one lease, got {other:?}"),
    }
}

/// Asserts that `m` waits without a lease, with `attempt` and `lost` lost claims.
async fn released(memory: &Store, m: &Message, attempt: u32, lost: u32) {
    let waiting = pending(memory, m).await;
    assert_eq!((waiting.attempt, waiting.lost_claims), (attempt, lost), "{}", m.to);
    assert!(memory.running_turn(&m.to).await.unwrap().is_none(), "{} holds no lease", m.to);
}

/// Claims answer `delay` late while `slow` is set (the store claimed at once); releases fail while `unreleasable` is set.
struct SlowClaims {
    slow: AtomicBool,
    delay: Duration,
    unreleasable: AtomicBool,
}

impl SlowClaims {
    fn new(delay: Duration) -> Self {
        Self { slow: AtomicBool::new(true), delay, unreleasable: AtomicBool::new(false) }
    }
}

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
        if self.slow.load(Ordering::SeqCst) {
            sleep(self.delay).await;
        }
        Ok(claims)
    }

    async fn release(&self, inner: &dyn Backend, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        if self.unreleasable.load(Ordering::SeqCst) {
            return Err(StoreError::unavailable("store unreachable"));
        }
        inner.release(address, epoch).await
    }
}

const PASS: u8 = 0;
const HANG_BEFORE: u8 = 1;
const HANG_AFTER: u8 = 2;

/// Commits take `delay` before (or after) they apply, by the mode; each one is logged once it applied.
struct SlowCommits {
    mode: AtomicU8,
    delay: Duration,
    applied: Events,
}

impl SlowCommits {
    fn new(mode: u8, delay: Duration) -> Self {
        Self { mode: AtomicU8::new(mode), delay, applied: Events::default() }
    }
}

#[async_trait]
impl Faults for SlowCommits {
    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        let mode = self.mode.load(Ordering::SeqCst);
        if mode == HANG_BEFORE {
            sleep(self.delay).await;
        }
        let address = commit.address.to_string();
        let result = inner.commit(commit).await?;
        self.applied.push(address);
        if mode == HANG_AFTER {
            sleep(self.delay).await;
        }
        Ok(result)
    }
}

/// Scenario: Cancellation during a claim.
#[tokio::test(start_paused = true)]
async fn a_lease_dropped_while_its_claim_is_in_flight_releases_the_claims_when_they_arrive() {
    let memory = virtual_store();
    let events = Events::default();
    let faulty = Faulty::new(memory.clone(), SlowClaims::new(secs(5)));
    let system = server(faulty.clone(), &events).await;
    let m = runnable(&system, "j").await;

    assert!(timeout(secs(1), system.lease_turns(&worker(), &selection(), 10)).await.is_err(), "dropped");
    sleep(secs(5)).await; // the claim arrives, and nobody takes it
    settle().await;

    released(&memory, &m, 1, 0).await;
    faulty.faults.slow.store(false, Ordering::SeqCst);
    let next = one(system.lease_turns(&worker(), &selection(), 10).await.unwrap());
    assert_eq!((next.message.attempt, next.message.lost_claims), (2, 0), "the release was no lost claim");
    assert!(events.all().is_empty());
}

/// Scenario: Cancellation during filtering, before and after the claim limit's commit applied.
#[tokio::test(start_paused = true)]
async fn a_lease_dropped_during_the_claim_limit_commit_releases_every_claim_it_did_not_hand_out() {
    // The commit has not applied: both claims are released, and the next claim fails the message at the limit.
    let memory = virtual_store();
    let events = Events::default();
    let faulty = Faulty::new(memory.clone(), SlowCommits::new(HANG_BEFORE, secs(10)));
    let system = server(faulty.clone(), &events).await;
    let (limited, runnable) = mixed(&memory, &system).await;

    assert!(timeout(secs(1), system.lease_turns(&worker(), &selection(), 10)).await.is_err(), "dropped");
    settle().await;

    released(&memory, &limited, 4, 3).await;
    released(&memory, &runnable, 1, 0).await;
    assert!(faulty.faults.applied.all().is_empty(), "the dropped commit never applied");
    assert!(events.all().is_empty());
    faulty.faults.mode.store(PASS, Ordering::SeqCst);
    let next = one(system.lease_turns(&worker(), &selection(), 10).await.unwrap());
    assert_eq!((next.actor.clone(), next.message.lost_claims), (runnable.to.clone(), 0));
    let outcome = outcome_of(&memory, &limited.id).await.expect("the next claim fails it");
    assert_eq!(outcome.error.as_deref(), Some("claim limit: 3 lost claims"));

    // The commit applied, its answer had not come: the cleanup undoes nothing, and nobody is told twice.
    let memory = virtual_store();
    let events = Events::default();
    let faulty = Faulty::new(memory.clone(), SlowCommits::new(HANG_AFTER, secs(10)));
    let system = server(faulty.clone(), &events).await;
    let (limited, runnable) = mixed(&memory, &system).await;
    system.create(&job("watcher"), &JobState::default()).await.unwrap();
    system.watch(&crate::WatchTarget::from(limited.to.clone()), &addr(&job("watcher"))).await.unwrap();

    assert!(timeout(secs(1), system.lease_turns(&worker(), &selection(), 10)).await.is_err(), "dropped");
    settle().await;

    assert_eq!(
        outcome_of(&memory, &limited.id).await.expect("applied").error.as_deref(),
        Some("claim limit: 3 lost claims")
    );
    released(&memory, &runnable, 1, 0).await;
    assert!(events.all().is_empty(), "unconfirmed: the observers stay silent");
    assert_eq!(notifications(&memory, &addr(&job("watcher"))).await.len(), 1);
}

/// Scenario: A late claim answer, exactly at the conservative deadline (30 s less 1 s).
#[tokio::test(start_paused = true)]
async fn a_claim_answer_at_its_deadline_starts_no_claim_limit_commit_and_releases_every_claim() {
    let memory = virtual_store();
    let events = Events::default();
    let commits = SlowCommits::new(PASS, Duration::ZERO);
    let applied = commits.applied.clone();
    let faulty = Faulty::new(memory.clone(), Both(SlowClaims::new(secs(29)), commits));
    let system = server(faulty.clone(), &events).await;
    let (limited, runnable) = mixed(&memory, &system).await;

    assert_eq!(system.lease_turns(&worker(), &selection(), 10).await.unwrap(), LeaseTurnsResult::Leased(Vec::new()));
    settle().await;

    assert!(applied.all().is_empty(), "no claim-limit commit");
    assert!(events.all().is_empty(), "no observer");
    released(&memory, &limited, 4, 3).await;
    released(&memory, &runnable, 1, 0).await;
}

/// Scenario: Post-processing consumes the TTL.
#[tokio::test(start_paused = true)]
async fn a_batch_whose_claim_limit_commit_passes_the_deadline_hands_out_nothing_and_keeps_the_failure() {
    let memory = virtual_store();
    let events = Events::default();
    let faulty = Faulty::new(memory.clone(), SlowCommits::new(HANG_BEFORE, secs(29)));
    let system = server(faulty.clone(), &events).await;
    let (limited, runnable) = mixed(&memory, &system).await;

    assert_eq!(system.lease_turns(&worker(), &selection(), 10).await.unwrap(), LeaseTurnsResult::Leased(Vec::new()));
    settle().await;

    assert!(outcome_of(&memory, &limited.id).await.is_some(), "the confirmed failure stays");
    assert_eq!(events.all(), vec!["start:Run:4".to_owned(), format!("end:{LIMIT_FAILURE}:sent=0")]);
    released(&memory, &runnable, 1, 0).await;
}

/// Scenario: Stopped server, observed after the claim and after the claim limit's commits.
#[tokio::test(start_paused = true)]
async fn a_system_that_stops_during_a_lease_answers_unavailable_and_releases_what_it_did_not_hand_out() {
    // While the claim is in flight.
    let memory = virtual_store();
    let events = Events::default();
    let system = server(Faulty::new(memory.clone(), SlowClaims::new(secs(5))), &events).await;
    let m = runnable(&system, "j").await;
    system.start().await.unwrap();
    let leasing = tokio::spawn({
        let system = system.clone();
        async move { system.lease_turns(&worker(), &selection(), 10).await }
    });
    sleep(secs(1)).await;
    system.stop().await;
    let result = leasing.await.unwrap();
    assert!(matches!(result, Err(OpError::Store(StoreError::Unavailable(_)))), "{result:?}");
    settle().await;
    released(&memory, &m, 1, 0).await;

    // While the claim limit commits: the confirmed failure stays.
    let memory = virtual_store();
    let events = Events::default();
    let system = server(Faulty::new(memory.clone(), SlowCommits::new(HANG_BEFORE, secs(5))), &events).await;
    let (limited, runnable) = mixed(&memory, &system).await;
    system.start().await.unwrap();
    let leasing = tokio::spawn({
        let system = system.clone();
        async move { system.lease_turns(&worker(), &selection(), 10).await }
    });
    sleep(secs(1)).await;
    system.stop().await;
    let result = leasing.await.unwrap();
    assert!(matches!(result, Err(OpError::Store(StoreError::Unavailable(_)))), "{result:?}");
    settle().await;
    assert!(outcome_of(&memory, &limited.id).await.is_some());
    assert_eq!(events.all(), vec!["start:Run:4".to_owned(), format!("end:{LIMIT_FAILURE}:sent=0")]);
    released(&memory, &runnable, 1, 0).await;
}

/// Scenario: Cleanup cannot reach the store: the lease stays until it expires, and the claim that replaces it counts it.
#[tokio::test(start_paused = true)]
async fn a_cleanup_that_cannot_reach_the_store_leaves_the_lease_to_expire_and_the_next_claim_counts_it_lost() {
    let memory = virtual_store();
    let events = Events::default();
    let faulty = Faulty::new(memory.clone(), SlowClaims::new(secs(5)));
    faulty.faults.unreleasable.store(true, Ordering::SeqCst);
    let system = server(faulty.clone(), &events).await;
    let m = runnable(&system, "j").await;

    assert!(timeout(secs(1), system.lease_turns(&worker(), &selection(), 10)).await.is_err(), "dropped");
    sleep(secs(5)).await;
    settle().await;

    faulty.faults.slow.store(false, Ordering::SeqCst);
    assert_eq!(memory.running_turn(&m.to).await.unwrap().expect("still held").node, worker());
    assert_eq!(lease(&system).await, Vec::new(), "nobody claims it before it expires");
    sleep(LEASE).await;
    let next = one(system.lease_turns(&worker(), &selection(), 10).await.unwrap());
    assert_eq!((next.message.attempt, next.message.lost_claims), (2, 1), "the claim it replaced is lost");
}

/// Scenario: Cleanup cannot reach the store, because the runtime ends with the claim in flight.
#[test]
fn a_runtime_that_ends_before_cleanup_leaves_the_lease_to_expire() {
    let clock = Arc::new(VirtualClock::new(default_start()));
    let memory = Store::memory_on(clock.clone());
    let paused = || tokio::runtime::Builder::new_current_thread().enable_all().start_paused(true).build().unwrap();
    let events = Events::default();
    let first = paused();
    let m = first.block_on(async {
        let system = server(Faulty::new(memory.clone(), SlowClaims::new(secs(5))), &events).await;
        let m = runnable(&system, "j").await;
        assert!(timeout(secs(1), system.lease_turns(&worker(), &selection(), 10)).await.is_err(), "dropped");
        m
    });
    drop(first); // the claim task, holding its claims, ends with the runtime

    paused().block_on(async {
        let system = server(memory.clone(), &events).await;
        assert!(memory.running_turn(&m.to).await.unwrap().is_some(), "nobody released it");
        assert_eq!(lease(&system).await, Vec::new());
        clock.advance_by(LEASE);
        let next = one(system.lease_turns(&worker(), &selection(), 10).await.unwrap());
        assert_eq!((next.message.attempt, next.message.lost_claims), (2, 1));
    });
}

/// The answers of renewals are lost after they applied.
struct LostRenewals;

#[async_trait]
impl Faults for LostRenewals {
    async fn renew(
        &self,
        inner: &dyn Backend,
        address: &ActorAddress,
        epoch: i64,
        ttl: Duration,
    ) -> StoreResult<crate::store::RenewResult> {
        inner.renew(address, epoch, ttl).await?;
        Err(StoreError::unavailable("the answer was lost"))
    }
}

/// Scenario: A shorter renewal response is withheld: B applies a 5 s renewal at t=10 and its answer is lost; nobody
/// claims the actor before the 60 s lease A granted runs out.
#[tokio::test(start_paused = true)]
async fn a_shorter_peer_renewal_whose_answer_is_lost_lets_no_claim_in_before_the_first_expiry() {
    let memory = virtual_store();
    let events = Events::default();
    let a = server_with(memory.clone(), "a", secs(60), &events).await;
    let b = server_with(Faulty::new(memory.clone(), LostRenewals), "b", secs(5), &events).await;
    let c = server_with(memory.clone(), "c", secs(60), &events).await;
    runnable(&a, "j").await;
    let t0 = a.now().await.unwrap();
    let held = one(a.lease_turns(&worker(), &selection(), 10).await.unwrap());

    sleep(secs(10)).await;
    assert!(b.renew_turn(&held.actor, held.epoch).await.is_err(), "the answer is lost");
    assert_eq!(memory.running_turn(&held.actor).await.unwrap().unwrap().lease_until, plus(t0, secs(60)));

    sleep(secs(5)).await;
    assert!(lease(&c).await.is_empty(), "t=15");
    sleep(secs(44)).await;
    assert!(lease(&c).await.is_empty(), "t=59");
    sleep(secs(1)).await;
    assert_eq!(one(c.lease_turns(&worker(), &selection(), 10).await.unwrap()).message.lost_claims, 1, "t=60");
}

/// Scenario: Loss after handoff.
#[tokio::test(start_paused = true)]
async fn a_batch_lost_after_handoff_expires_and_counts_lost_only_when_replaced() {
    let memory = virtual_store();
    let events = Events::default();
    let system = server(memory.clone(), &events).await;
    let m = runnable(&system, "j").await;
    drop(lease(&system).await); // as if the response carrying it never arrived

    assert!(memory.running_turn(&m.to).await.unwrap().is_some(), "dropping a lease does nothing");
    sleep(LEASE).await;
    let waiting = pending(&memory, &m).await;
    assert_eq!((waiting.attempt, waiting.lost_claims), (1, 0), "expired, not yet counted");
    let next = one(system.lease_turns(&worker(), &selection(), 10).await.unwrap());
    assert_eq!((next.message.attempt, next.message.lost_claims), (2, 1), "counted once replaced");
}

/// Routes claims and releases through the first faults, commits through the second.
struct Both(SlowClaims, SlowCommits);

#[async_trait]
impl Faults for Both {
    async fn claim_due(
        &self,
        inner: &dyn Backend,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        self.0.claim_due(inner, types, node, ttl, limit).await
    }

    async fn release(&self, inner: &dyn Backend, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        self.0.release(inner, address, epoch).await
    }

    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        self.1.commit(inner, commit).await
    }
}

/// Scenario: A shorter renewal response is withheld, with two SQLite handles on one file as two processes would have.
#[tokio::test]
async fn on_a_shared_sqlite_file_a_shorter_peer_renewal_keeps_the_first_expiry() {
    let db = crate::tests::sqlite::common::TempDb::new();
    let clock = Arc::new(VirtualClock::new(default_start()));
    let first = Store::from_backend(Arc::new(db.open(clock.clone()).await));
    let second = Store::from_backend(Arc::new(db.open(clock.clone()).await));
    let events = Events::default();
    let a = server_with(first.clone(), "a", secs(60), &events).await;
    let b = server_with(second.clone(), "b", secs(5), &events).await;
    let c = server_with(second.clone(), "c", secs(60), &events).await;
    runnable(&a, "j").await;
    let held = one(a.lease_turns(&worker(), &selection(), 10).await.unwrap());

    clock.advance_by(secs(10));
    assert_eq!(
        b.renew_turn(&held.actor, held.epoch).await.unwrap(),
        RenewTurnResult::Renewed { lease_until: held.lease_until, ttl: secs(5), cancel_requested: false }
    );
    clock.advance_by(secs(5));
    assert!(lease(&c).await.is_empty(), "t=15");
    clock.advance_by(secs(44));
    assert!(lease(&c).await.is_empty(), "t=59");
    clock.advance_by(secs(1));
    assert_eq!(one(c.lease_turns(&worker(), &selection(), 10).await.unwrap()).message.lost_claims, 1, "t=60");
    first.close().await;
    second.close().await;
}

/// Scenario: Different servers grant different TTLs, on PostgreSQL with two stores (skipped without `CICADA_PG_URL`):
/// a shorter renewal keeps the expiry, a longer one extends it.
#[tokio::test(flavor = "multi_thread")]
async fn on_postgres_a_peer_renewal_never_shortens_the_lease() {
    let Some(db) = crate::tests::postgres::common::TestDb::new("rl").await else {
        eprintln!("skipped: CICADA_PG_URL is unset");
        return;
    };
    let events = Events::default();
    let a = server_with(Store::from_backend(db.open().await), "a", secs(60), &events).await;
    let short = server_with(Store::from_backend(db.open().await), "b", secs(1), &events).await;
    let long = server_with(Store::from_backend(db.open().await), "c", secs(120), &events).await;
    runnable(&a, "j").await;
    let held = one(a.lease_turns(&worker(), &selection(), 10).await.unwrap());

    let RenewTurnResult::Renewed { lease_until, ttl, .. } = short.renew_turn(&held.actor, held.epoch).await.unwrap()
    else {
        panic!("renewed")
    };
    assert_eq!((lease_until, ttl), (held.lease_until, secs(1)), "the shorter renewal keeps the expiry");
    assert!(lease(&long).await.is_empty(), "nobody else claims it");
    let RenewTurnResult::Renewed { lease_until, ttl, .. } = long.renew_turn(&held.actor, held.epoch).await.unwrap()
    else {
        panic!("renewed")
    };
    assert!(lease_until > held.lease_until && ttl == secs(120), "the longer one extends it");
    db.cleanup().await;
}
