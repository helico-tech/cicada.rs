//! Faults and races of completing leased turns (spec `turn-leases`, ADR 0038), on virtual time: an error before the
//! commit, a commit whose answer is lost, concurrent reports, stale epochs and a delete and re-create, a failed read of
//! the retry snapshot, an outcome hidden by retention, a lost rejection, an acknowledged rejection followed by an
//! uncertain failure commit, and a completion dropped during its commit, also on a shared SQLite file. Every path
//! commits at most one outcome and effect set, runs no handler (the servers host none) and never releases a lease.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use super::common::*;
use super::remote_claim_limit::{JobState, job, notifications, selection, worker};
use crate::address::ActorAddress;
use crate::retention::Retention;
use crate::retry::{Backoff, RetryPolicy};
use crate::runtime::{
    CompleteTurnResult, LeaseTurnsResult, NodeConfig, OpError, StateReport, TurnEffects, TurnFailure, TurnLease,
    TurnReport, TurnSend,
};
use crate::store::{
    Backend, CommitRequest, CommitResult, Message, MessageId, MessageStatus, OutcomeStatus, StoreError, StoreResult,
    TurnOutcome,
};
use crate::testing::{VirtualClock, default_start};
use crate::{ActorSystem, FailureCause, SendOptions, Store, WatchTarget};
use async_trait::async_trait;
use serde_json::json;
use tokio::time::{sleep, timeout};

const LEASE: Duration = Duration::from_secs(30);
/// How long a hanging commit waits: past the replacement of a lease of `LEASE`, within the lease that replaces it.
const HANG: Duration = Duration::from_secs(45);

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

const PASS: u8 = 0;
/// The next commit fails without applying.
const FAIL_BEFORE: u8 = 1;
/// The next commit applies and its answer is lost.
const LOSE_AFTER: u8 = 2;
/// Commits wait before they apply.
const HANG_BEFORE: u8 = 3;
/// Commits wait after they applied.
const HANG_AFTER: u8 = 4;

/// Commit faults by mode (the two failing modes apply once), status reads that fail while `unreadable` is set, and a
/// count of every commit that applied and of every release.
#[derive(Default)]
struct Chaos {
    mode: AtomicU8,
    unreadable: AtomicBool,
    commits: AtomicUsize,
    applied: AtomicUsize,
    releases: AtomicUsize,
}

impl Chaos {
    fn set(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
    }

    fn applied(&self) -> usize {
        self.applied.load(Ordering::SeqCst)
    }

    fn assert_never_released(&self) {
        assert_eq!(self.releases.load(Ordering::SeqCst), 0, "no release");
    }
}

#[async_trait]
impl Faults for Chaos {
    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        let mode = self.mode.load(Ordering::SeqCst);
        if mode == FAIL_BEFORE {
            self.set(PASS);
            return Err(StoreError::unavailable("the store is unreachable"));
        }
        if mode == HANG_BEFORE {
            sleep(HANG).await;
        }
        let result = inner.commit(commit).await?;
        if matches!(result, CommitResult::Committed { .. }) {
            self.applied.fetch_add(1, Ordering::SeqCst);
        }
        if mode == LOSE_AFTER {
            self.set(PASS);
            return Err(StoreError::unavailable("the answer was lost"));
        }
        if mode == HANG_AFTER {
            sleep(HANG).await;
        }
        Ok(result)
    }

    async fn status(&self, inner: &dyn Backend, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(StoreError::unavailable("the store is unreachable"));
        }
        inner.status(id).await
    }

    async fn release(&self, inner: &dyn Backend, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        inner.release(address, epoch).await
    }
}

/// A server that hosts nothing, with an observer writing to `events`.
async fn server(store: impl IntoStore, name: &str, events: &Events) -> ActorSystem {
    let config = NodeConfig { lease_ttl: LEASE, ..node_config(name) };
    let system = node(store, name).config(config).observe(Recorder::new(events)).build();
    system
        .register_contract(&crate::actor::contract_of::<super::remote_claim_limit::JobActor>().unwrap())
        .await
        .unwrap();
    system
}

/// A faulty server and a peer on the same memory store, and the jobs `ids`.
async fn setup(ids: &[&str]) -> (Store, Arc<Faulty<Chaos>>, ActorSystem, ActorSystem, Events) {
    let memory = virtual_store();
    let events = Events::default();
    let faulty = Faulty::new(memory.clone(), Chaos::default());
    let system = server(faulty.clone(), "server", &events).await;
    let peer = server(memory.clone(), "peer", &events).await;
    for id in ids {
        create(&system, id).await;
    }
    (memory, faulty, system, peer, events)
}

async fn create(system: &ActorSystem, id: &str) {
    system.create(&job(id), &JobState::default()).await.unwrap();
}

async fn send_with(system: &ActorSystem, id: &str, options: SendOptions) -> Message {
    accepted(system.send_with(&job(id), super::remote_claim_limit::JobMsg::Run, options).await.unwrap())
}

/// Sends `job/<id>` a message and leases its turn; only that job may have a due message.
async fn leased(system: &ActorSystem, id: &str) -> TurnLease {
    send_with(system, id, SendOptions::default()).await;
    lease(system).await
}

async fn lease(system: &ActorSystem) -> TurnLease {
    match system.lease_turns(&worker(), &selection(), 10).await.unwrap() {
        LeaseTurnsResult::Leased(mut leases) if leases.len() == 1 => leases.remove(0),
        other => panic!("expected one lease, got {other:?}"),
    }
}

async fn complete(system: &ActorSystem, lease: &TurnLease, report: TurnReport) -> Result<CompleteTurnResult, OpError> {
    system.complete_turn(&lease.actor, lease.epoch, &lease.message.id, report).await
}

fn run_to(id: &str) -> TurnSend {
    TurnSend::new(addr(&job(id)), "Run", json!({}))
}

/// A returned report that sets a state and sends `job/<to>` one message.
fn sending(to: &str) -> TurnReport {
    let effects = TurnEffects { sends: vec![run_to(to)], ..TurnEffects::default() };
    TurnReport::Returned { state: StateReport::Set(json!({"n": 1})), effects }
}

fn failure(retryable: bool) -> TurnReport {
    TurnReport::Failed(TurnFailure { cause: FailureCause::Handler, retryable, reason: "flaky".into() })
}

fn completed(result: Result<CompleteTurnResult, OpError>) -> TurnOutcome {
    match result {
        Ok(CompleteTurnResult::Completed(done)) => done.outcome,
        other => panic!("expected Completed, got {other:?}"),
    }
}

fn assert_uncertain(result: Result<CompleteTurnResult, OpError>) {
    assert!(matches!(result, Err(OpError::Store(StoreError::Unavailable(_)))), "{result:?}");
}

/// The messages waiting for `job/<id>`.
async fn mailbox(memory: &Store, id: &str) -> Vec<Message> {
    memory.pending(&addr(&job(id)), None, 100).await.unwrap().items
}

async fn version(memory: &Store, id: &str) -> i64 {
    memory.get(&addr(&job(id))).await.unwrap().expect("the job").version
}

/// Asserts that the lease of `lease` is still held and its message has no outcome.
async fn still_held(memory: &Store, lease: &TurnLease) {
    assert_eq!(memory.running_turn(&lease.actor).await.unwrap().expect("held").message_id, lease.message.id);
    assert!(outcome_of(memory, &lease.message.id).await.is_none());
}

/// Scenario: Failure before the primary commit applies.
#[tokio::test(start_paused = true)]
async fn an_error_before_the_commit_applies_is_returned_and_the_identical_report_then_commits_once() {
    let (memory, faulty, system, peer, events) = setup(&["a", "b"]).await;
    let held = leased(&system, "a").await;
    let report = sending("b");
    faulty.faults.set(FAIL_BEFORE);

    assert_uncertain(complete(&system, &held, report.clone()).await);
    still_held(&memory, &held).await;
    assert!(mailbox(&memory, "b").await.is_empty());

    assert_eq!(completed(complete(&peer, &held, report).await), TurnOutcome::Done);
    assert_eq!((mailbox(&memory, "b").await.len(), version(&memory, "a").await), (1, 1));
    faulty.faults.assert_never_released();
    assert!(events.all().is_empty(), "no observer of the server hears of a remote turn");
}

/// Scenario: Failure after the primary commit applies. Scenario: Server observers and wakes.
#[tokio::test(start_paused = true)]
async fn a_commit_whose_answer_is_lost_is_lease_lost_on_retry_and_nothing_applies_twice() {
    let (memory, faulty, system, peer, events) = setup(&["a", "b", "w"]).await;
    system.watch(&WatchTarget::from(addr(&job("a"))), &addr(&job("w"))).await.unwrap();
    system.pause_mailbox(&addr(&job("w"))).await.unwrap();
    system.pause_mailbox(&addr(&job("b"))).await.unwrap();

    // A returned report: one state, one send.
    let held = leased(&system, "a").await;
    faulty.faults.set(LOSE_AFTER);
    assert_uncertain(complete(&system, &held, sending("b")).await);
    assert_eq!(complete(&peer, &held, sending("b")).await.unwrap(), CompleteTurnResult::LeaseLost);
    assert_eq!(complete(&system, &held, sending("b")).await.unwrap(), CompleteTurnResult::LeaseLost);
    assert_eq!((mailbox(&memory, "b").await.len(), version(&memory, "a").await), (1, 1));

    // A failure: one outcome, one notification.
    let held = leased(&system, "a").await;
    faulty.faults.set(LOSE_AFTER);
    assert_uncertain(complete(&system, &held, failure(false)).await);
    assert_eq!(complete(&peer, &held, failure(false)).await.unwrap(), CompleteTurnResult::LeaseLost);
    assert_eq!(outcome_of(&memory, &held.message.id).await.unwrap().status, OutcomeStatus::Failed);
    assert_eq!(notifications(&memory, &addr(&job("w"))).await.len(), 1);
    assert_eq!(faulty.faults.applied(), 2);
    faulty.faults.assert_never_released();
    assert!(events.all().is_empty());
}

/// Scenario: Concurrent reports.
#[tokio::test(start_paused = true)]
async fn identical_or_different_reports_racing_for_one_epoch_apply_at_most_once() {
    let (memory, faulty, system, peer, _) = setup(&["a", "b"]).await;
    system.pause_mailbox(&addr(&job("b"))).await.unwrap();
    for different in [false, true] {
        let held = leased(&system, "a").await;
        let first = sending("b");
        let second = if different { failure(false) } else { first.clone() };
        faulty.faults.set(HANG_BEFORE); // the faulty server's commit waits; the peer's goes first

        let (one, other) = tokio::join!(complete(&system, &held, first), complete(&peer, &held, second));

        let results = [one.unwrap(), other.unwrap()];
        let done = results.iter().filter(|r| matches!(r, CompleteTurnResult::Completed(_))).count();
        let lost = results.iter().filter(|r| **r == CompleteTurnResult::LeaseLost).count();
        assert_eq!((done, lost), (1, 1), "{results:?}");
        faulty.faults.set(PASS);
    }
    // Identical: the peer's send; different: the peer's failure, and no second send.
    assert_eq!(mailbox(&memory, "b").await.len(), 1);
    assert_eq!(faulty.faults.applied(), 0, "the waiting commits found the lease gone");
    faulty.faults.assert_never_released();
}

/// Scenario: Concurrent reports, with stale epochs and a delete and re-create.
#[tokio::test(start_paused = true)]
async fn a_stale_epoch_or_one_of_a_deleted_life_applies_nothing_and_leaves_the_later_lease_alone() {
    let (memory, faulty, system, _, _) = setup(&["a", "b"]).await;
    system.pause_mailbox(&addr(&job("b"))).await.unwrap();
    let old = leased(&system, "a").await;
    sleep(LEASE + secs(1)).await;
    let new = lease(&system).await;
    assert!(new.epoch > old.epoch);

    assert_eq!(complete(&system, &old, sending("b")).await.unwrap(), CompleteTurnResult::LeaseLost);
    assert_eq!(complete(&system, &old, failure(false)).await.unwrap(), CompleteTurnResult::LeaseLost);
    still_held(&memory, &new).await;

    system.delete_actor(&new.actor).await.unwrap();
    create(&system, "a").await;
    let next = leased(&system, "a").await;
    assert_eq!(complete(&system, &new, sending("b")).await.unwrap(), CompleteTurnResult::LeaseLost);
    still_held(&memory, &next).await;
    assert!(mailbox(&memory, "b").await.is_empty());
    assert_eq!(faulty.faults.applied(), 0);
    faulty.faults.assert_never_released();
}

/// Scenario: Status read fails or races replacement.
#[tokio::test(start_paused = true)]
async fn a_retry_snapshot_that_cannot_be_read_returns_the_error_and_commits_nothing() {
    let (memory, faulty, system, _, _) = setup(&["a"]).await;
    let policy = RetryPolicy::new(3, Backoff::fixed(secs(1)).unwrap()).unwrap();
    send_with(&system, "a", SendOptions::new().retry(policy)).await;
    let held = lease(&system).await;
    faulty.faults.unreadable.store(true, Ordering::SeqCst);

    assert_uncertain(complete(&system, &held, failure(true)).await);
    assert_eq!(faulty.faults.commits.load(Ordering::SeqCst), 0, "no commit");
    still_held(&memory, &held).await;

    // A failure that is not retryable reads nothing.
    assert_eq!(completed(complete(&system, &held, failure(false)).await), TurnOutcome::Failed("flaky".into()));
    faulty.faults.assert_never_released();
}

/// Scenario: Status read fails or races replacement: a snapshot read before the lease is replaced cannot apply.
#[tokio::test(start_paused = true)]
async fn a_retry_decided_from_a_snapshot_before_the_lease_was_replaced_is_fenced() {
    let (memory, faulty, system, peer, _) = setup(&["a"]).await;
    let policy = RetryPolicy::new(3, Backoff::fixed(secs(1)).unwrap()).unwrap();
    let m = send_with(&system, "a", SendOptions::new().retry(policy)).await;
    let old = lease(&system).await;
    faulty.faults.set(HANG_BEFORE);
    let completing = tokio::spawn({
        let (system, old) = (system.clone(), old.clone());
        async move { complete(&system, &old, failure(true)).await }
    });
    sleep(LEASE + secs(1)).await; // the snapshot was read; the lease runs out and is replaced
    let new = lease(&peer).await;

    assert_eq!(completing.await.unwrap().unwrap(), CompleteTurnResult::LeaseLost);
    let MessageStatus::Running { message, .. } = memory.status(&m.id).await.unwrap().unwrap() else { panic!() };
    assert_eq!((message.failures, message.lost_claims), (0, 1), "no reschedule under the old epoch");
    still_held(&memory, &new).await;
}

/// Scenario: Retention hides the result.
#[tokio::test(start_paused = true)]
async fn an_outcome_hidden_by_retention_still_lets_the_epoch_refuse_a_duplicate() {
    let (memory, faulty, system, peer, _) = setup(&["a", "b"]).await;
    system.pause_mailbox(&addr(&job("b"))).await.unwrap();
    send_with(&system, "a", SendOptions::new().retention(Retention::REMOVE_ON_DONE)).await;
    let held = lease(&system).await;
    faulty.faults.set(LOSE_AFTER);

    assert_uncertain(complete(&system, &held, sending("b")).await);

    assert!(memory.status(&held.message.id).await.unwrap().is_none(), "the outcome is gone: a read cannot tell");
    assert_eq!(complete(&peer, &held, sending("b")).await.unwrap(), CompleteTurnResult::LeaseLost);
    assert_eq!(mailbox(&memory, "b").await.len(), 1, "sent once");
    faulty.faults.assert_never_released();
}

/// Scenario: A lost rejection response.
#[tokio::test(start_paused = true)]
async fn after_a_lost_rejection_the_original_report_is_evaluated_again_against_the_store() {
    let (memory, faulty, system, peer, _) = setup(&["a"]).await;
    let held = leased(&system, "a").await;

    // Rejected: the send's target is absent. The answer never reaches the caller.
    let lost = complete(&system, &held, sending("late")).await.unwrap();
    assert!(matches!(lost, CompleteTurnResult::Rejected(_)), "{lost:?}");
    assert_eq!(faulty.faults.applied(), 0, "no failure was committed");
    still_held(&memory, &held).await;

    create(&system, "late").await;
    system.pause_mailbox(&addr(&job("late"))).await.unwrap();
    assert_eq!(completed(complete(&peer, &held, sending("late")).await), TurnOutcome::Done);
    assert_eq!(mailbox(&memory, "late").await.len(), 1);
    faulty.faults.assert_never_released();
}

/// What a compliant executor does with its current report: repeats it after an uncertain answer, replaces it for good
/// with the failure of a rejection, and stops at any other answer. Returns the reports it submitted.
async fn submit(servers: &[&ActorSystem], lease: &TurnLease, report: TurnReport) -> (TurnReport, Vec<TurnReport>) {
    let mut current = report;
    let mut submitted = Vec::new();
    for attempt in 0..10 {
        submitted.push(current.clone());
        match complete(servers[attempt % servers.len()], lease, current.clone()).await {
            Ok(CompleteTurnResult::Rejected(failure)) => current = TurnReport::Failed(failure),
            Err(OpError::Store(_)) => {}
            Ok(CompleteTurnResult::Completed(_) | CompleteTurnResult::LeaseLost) | Err(_) => break,
        }
    }
    (current, submitted)
}

/// Scenario: An acknowledged rejection followed by an uncertain failed commit.
#[tokio::test(start_paused = true)]
async fn after_an_acknowledged_rejection_only_the_failure_is_retried_even_once_the_original_would_apply() {
    let (memory, faulty, system, peer, _) = setup(&["a"]).await;
    let held = leased(&system, "a").await;
    let original = sending("late");
    let rejection = complete(&system, &held, original.clone()).await.unwrap();
    let CompleteTurnResult::Rejected(failure) = rejection else { panic!("{rejection:?}") };
    faulty.faults.set(LOSE_AFTER); // the failure commit applies; its answer is lost
    assert_uncertain(complete(&system, &held, TurnReport::Failed(failure.clone())).await);
    create(&system, "late").await; // now the original effects would be valid

    let (last, submitted) = submit(&[&peer, &system], &held, TurnReport::Failed(failure.clone())).await;

    assert_eq!(last, TurnReport::Failed(failure.clone()));
    assert!(submitted.iter().all(|r| r != &original), "the original effects never come back");
    let record = outcome_of(&memory, &held.message.id).await.expect("failed");
    assert_eq!((record.status, record.error), (OutcomeStatus::Failed, Some(failure.reason)));
    assert!(mailbox(&memory, "late").await.is_empty());
    assert_eq!(version(&memory, "a").await, 0);
    faulty.faults.assert_never_released();

    // The whole path through the executor: rejected, replaced for good, and the failure committed.
    let held = leased(&system, "a").await;
    let (last, submitted) = submit(&[&system], &held, sending("absent")).await;
    assert!(matches!(last, TurnReport::Failed(_)));
    assert_eq!(submitted.len(), 2);
    assert_eq!(outcome_of(&memory, &held.message.id).await.unwrap().status, OutcomeStatus::Failed);
}

/// Scenario: Cancellation of the completion request.
#[tokio::test(start_paused = true)]
async fn a_completion_dropped_during_its_commit_is_retried_through_a_peer_and_applies_once() {
    // Dropped before the commit applied: the retry applies it.
    let (memory, faulty, system, peer, _) = setup(&["a", "b"]).await;
    system.pause_mailbox(&addr(&job("b"))).await.unwrap();
    let held = leased(&system, "a").await;
    faulty.faults.set(HANG_BEFORE);
    assert!(timeout(secs(1), complete(&system, &held, sending("b"))).await.is_err(), "dropped");
    settle().await;
    still_held(&memory, &held).await;
    assert_eq!(completed(complete(&peer, &held, sending("b")).await), TurnOutcome::Done);

    // Dropped after it applied: the retry is lease lost.
    let held = leased(&system, "a").await;
    faulty.faults.set(HANG_AFTER);
    assert!(timeout(secs(1), complete(&system, &held, sending("b"))).await.is_err(), "dropped");
    assert_eq!(complete(&peer, &held, sending("b")).await.unwrap(), CompleteTurnResult::LeaseLost);
    assert_eq!((mailbox(&memory, "b").await.len(), version(&memory, "a").await), (2, 2), "once each");
    assert_eq!(faulty.faults.applied(), 1);
    faulty.faults.assert_never_released();
}

/// A completion of `job/a` through `first`, dropped after a wait that grows each round (whatever its commit did by
/// then), and the identical report through `second` once the first transaction settled: exactly one effect set applies
/// each round, by the dropped call (the retry is lease lost) or by the retry.
async fn dropped_completion_settles_once(first: &ActorSystem, second: &ActorSystem, rounds: u32) {
    create(first, "a").await;
    create(first, "b").await;
    first.pause_mailbox(&addr(&job("b"))).await.unwrap();
    for round in 0..rounds {
        let held = leased(first, "a").await;
        let report = sending("b");
        let completing = tokio::spawn({
            let (first, held, report) = (first.clone(), held.clone(), report.clone());
            async move { complete(&first, &held, report).await }
        });
        sleep(Duration::from_micros(250) * round).await;
        completing.abort();
        let _ = completing.await;
        // The settlement boundary: on SQLite the next write through the first handle settles a transaction the drop
        // left open (rolled back unless its COMMIT went out); until then a writer on the file finds it locked, an
        // uncertain answer the caller repeats its report after.
        first.pause_mailbox(&addr(&job("b"))).await.unwrap();
        match complete(second, &held, report).await.unwrap() {
            CompleteTurnResult::Completed(_) | CompleteTurnResult::LeaseLost => {}
            other => panic!("round {round}: {other:?}"),
        }
        let snapshot = first.get_actor(&addr(&job("a"))).await.unwrap().unwrap();
        let sent = first.pending(&addr(&job("b")), None, 100).await.unwrap().unwrap().items.len();
        assert_eq!((snapshot.version, sent), (round as i64 + 1, round as usize + 1), "round {round}: once");
        assert!(second.running_turn(&held.actor).await.unwrap().is_none());
    }
}

/// Scenario: Cancellation of the completion request, with two SQLite handles on one file as two processes have.
#[tokio::test]
async fn on_a_shared_sqlite_file_a_dropped_completion_settles_before_or_without_its_retry() {
    let db = crate::tests::sqlite::common::TempDb::new();
    let clock = Arc::new(VirtualClock::new(default_start()));
    let first = Store::from_backend(Arc::new(db.open(clock.clone()).await));
    let second = Store::from_backend(Arc::new(db.open(clock.clone()).await));
    let events = Events::default();
    let a = server(first.clone(), "a", &events).await;
    let b = server(second.clone(), "b", &events).await;
    dropped_completion_settles_once(&a, &b, 24).await;
    first.close().await;
    second.close().await;
}
