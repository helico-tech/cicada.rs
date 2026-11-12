//! The claim limit (spec `claim-limit`, ADR 0025): a message with as many lost claims as the node's limit ends `failed`
//! without running, through a commit that tells its watchers, and the observers are told after that commit.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use super::common::*;
use crate::address::{ActorAddress, TypeName};
use crate::observer::TurnObserver;
use crate::prelude::*;
use crate::runtime::{NodeConfig, RetryResult, StartProblem};
use crate::store::{
    Backend, CommitRequest, CommitResult, Message, MessageStatus, NodeId, OutcomeRecord, OutcomeStatus,
    OutgoingMessage, RenewResult, StoreError, StoreResult,
};
use crate::testing::{TestActors, claim_and_abandon};
use crate::{ActorSystem, Failed, Stopped, WatchTarget};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Job {
    id: String,
}

impl ActorRef for Job {
    const TYPE: &'static str = "job";
    type Message = JobMsg;
}

fn job(id: &str) -> Job {
    Job { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum JobMsg {
    /// Counts a run, then sleeps `secs`, then fails while the message has fewer than `fail_runs` failures.
    Run {
        #[serde(default)]
        secs: u64,
        #[serde(default, rename = "failRuns")]
        fail_runs: u32,
    },
    Failed(Failed),
    Stopped(Stopped),
}

fn run() -> JobMsg {
    JobMsg::Run { secs: 0, fail_runs: 0 }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct JobState {
    /// The `Failed` notifications it was told.
    #[serde(default)]
    failed: Vec<Failed>,
}

/// The job type; counts the runs of its handler.
struct JobActor(Arc<AtomicU32>);

impl Actor for JobActor {
    type Ref = Job;
    type State = JobState;

    async fn handle(&self, turn: &mut Turn<Self>, message: JobMsg) -> HandlerResult {
        match message {
            JobMsg::Run { secs, fail_runs } => {
                self.0.fetch_add(1, Ordering::SeqCst);
                if secs > 0 {
                    sleep(Duration::from_secs(secs)).await;
                }
                if turn.message().failures < fail_runs {
                    return Err(HandlerError::msg("flaky"));
                }
            }
            JobMsg::Failed(failed) => turn.state_mut().failed.push(failed),
            JobMsg::Stopped(_) => {}
        }
        Ok(())
    }
}

const LEASE: Duration = Duration::from_secs(30);

/// A harness whose node has the claim limit `limit`, the counter of handler runs and `observers`.
async fn harness(limit: Option<u32>, observers: Vec<Arc<dyn TurnObserver>>) -> (TestActors, Arc<AtomicU32>) {
    let runs = Arc::new(AtomicU32::new(0));
    let actor = JobActor(runs.clone());
    let test = TestActors::builder()
        .with(move |n| {
            let mut n = n.host(actor).config(NodeConfig { max_lost_claims: limit, ..NodeConfig::default() });
            for observer in observers {
                n = n.observe(observer);
            }
            n
        })
        .build()
        .await
        .unwrap();
    test.create(&job("j"), &JobState::default()).await;
    (test, runs)
}

fn types() -> BTreeSet<TypeName> {
    BTreeSet::from([type_name::<Job>()])
}

/// Plays `n` nodes that each claim the head of `job/j` and die, one after the other, each lease running out: the next
/// claim of the message replaces the last of them, so it finds `n` lost claims.
async fn die(test: &TestActors, n: u32) {
    for _ in 0..n {
        let abandoned = claim_and_abandon(test.store(), &types(), LEASE, 1).await.unwrap();
        assert_eq!(abandoned.len(), 1);
        sleep(LEASE + Duration::from_secs(1)).await;
    }
}

async fn outcome(test: &TestActors, message: &Message) -> OutcomeRecord {
    test.outcome(message).await.expect("the message has its outcome")
}

/// Scenario: A message whose turns keep dying.
#[tokio::test(start_paused = true)]
async fn a_message_whose_turns_keep_dying_ends_failed_at_the_limit_without_running_again() {
    struct LoseLease;

    #[async_trait]
    impl Faults for LoseLease {
        async fn renew(&self, _: &dyn Backend, _: &ActorAddress, _: i64, _: Duration) -> StoreResult<RenewResult> {
            Ok(RenewResult::LeaseLost)
        }
    }

    let memory = virtual_store();
    let runs = Arc::new(AtomicU32::new(0));
    let config = NodeConfig { max_lost_claims: Some(3), ..node_config("n1") };
    let system = node(Faulty::new(memory.clone(), LoseLease), "n1").host(JobActor(runs.clone())).config(config).build();
    system.create(&job("j"), &JobState::default()).await.unwrap();
    let m = accepted(system.send(&job("j"), JobMsg::Run { secs: 20, fail_runs: 0 }).await.unwrap());
    for run in 1..=3 {
        assert_eq!(system.run_due_turns(1).await.unwrap(), 1); // the lease is lost at the first heartbeat
        assert_eq!(runs.load(Ordering::SeqCst), run);
        sleep(LEASE + Duration::from_secs(1)).await; // and runs out
    }
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&memory, &m.id).await.expect("an outcome");
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert_eq!(outcome.error.as_deref(), Some("claim limit: 3 lost claims"));
    assert_eq!((outcome.envelope.attempt, outcome.envelope.lost_claims), (4, 3));
    assert_eq!(runs.load(Ordering::SeqCst), 3, "the handler does not run a fourth time");
}

/// Scenario: Fewer lost claims.
#[tokio::test(start_paused = true)]
async fn a_message_with_fewer_lost_claims_than_the_limit_runs() {
    let (test, runs) = harness(Some(3), vec![]).await;
    let m = test.send(&job("j"), run()).await;
    die(&test, 2).await;
    test.run_until_idle().await;
    let outcome = outcome(&test, &m).await;
    assert_eq!((outcome.status, outcome.envelope.lost_claims), (OutcomeStatus::Done, 2));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

/// Scenario: Claims that were given back do not count.
#[tokio::test(start_paused = true)]
async fn claims_that_were_given_back_do_not_count() {
    let (test, runs) = harness(Some(3), vec![]).await;
    let m = test.send(&job("j"), run()).await;
    for _ in 0..5 {
        let claim = test.store().claim_due(&types(), &NodeId::random(), LEASE, 1).await.unwrap().remove(0);
        test.store().release(&claim.actor, claim.epoch).await.unwrap();
    }
    test.run_until_idle().await;
    let outcome = outcome(&test, &m).await;
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!((outcome.envelope.attempt, outcome.envelope.lost_claims), (6, 0));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

/// Scenario: Runs that were committed do not count.
#[tokio::test(start_paused = true)]
async fn runs_that_were_committed_as_retries_do_not_count() {
    let (test, runs) = harness(Some(3), vec![]).await;
    let retry = RetryPolicy::new(10, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    let m = test.send_with(&job("j"), JobMsg::Run { secs: 0, fail_runs: 5 }, SendOptions::new().retry(retry)).await;
    test.advance_time_by(Duration::from_secs(60)).await;
    let outcome = outcome(&test, &m).await;
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!((outcome.envelope.failures, outcome.envelope.lost_claims), (5, 0));
    assert_eq!(runs.load(Ordering::SeqCst), 6, "the handler runs after five committed failures");
}

/// Scenario: The retry policy is not asked.
#[tokio::test(start_paused = true)]
async fn the_retry_policy_of_a_message_at_the_limit_is_not_asked() {
    let (test, runs) = harness(Some(3), vec![]).await;
    let retry = RetryPolicy::new(10, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    let m = test.send_with(&job("j"), run(), SendOptions::new().retry(retry)).await;
    die(&test, 3).await;
    test.advance_time_by(Duration::from_secs(60)).await;
    let outcome = outcome(&test, &m).await;
    assert_eq!(outcome.status, OutcomeStatus::Failed, "not rescheduled");
    assert_eq!(outcome.envelope.failures, 0);
    assert!(test.pending(&job("j")).await.is_empty());
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

/// Scenario: The mailbox moves on.
#[tokio::test(start_paused = true)]
async fn the_mailbox_moves_on_after_a_message_fails_at_the_limit() {
    let (test, runs) = harness(Some(3), vec![]).await;
    let head = test.send(&job("j"), run()).await;
    let next = test.send(&job("j"), run()).await;
    die(&test, 3).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &head).await.status, OutcomeStatus::Failed);
    let next = outcome(&test, &next).await;
    assert_eq!((next.status, next.envelope.lost_claims), (OutcomeStatus::Done, 0));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

/// Scenario: A watcher is told.
#[tokio::test(start_paused = true)]
async fn each_watcher_is_told_once_of_a_message_that_fails_at_the_limit() {
    let (test, _) = harness(Some(3), vec![]).await;
    let target = WatchTarget::from(addr(&job("j")));
    for watcher in ["watcher-a", "watcher-b"] {
        test.create(&job(watcher), &JobState::default()).await;
        test.system().watch(&target, &addr(&job(watcher))).await.unwrap();
    }
    let m = test.send(&job("j"), run()).await;
    die(&test, 3).await;
    test.run_until_idle().await;
    for watcher in ["watcher-a", "watcher-b"] {
        let told = test.state::<_, JobState>(&job(watcher)).await.failed;
        assert_eq!(told.len(), 1, "{watcher}: {told:?}");
        assert_eq!(told[0].message_id, m.id);
        assert_eq!(told[0].error, "claim limit: 3 lost claims");
        assert_eq!(told[0].attempts, 4, "the attempt of the claim that failed it");
    }
}

/// Scenario: A message whose turns keep dying, when the message cannot be decoded: the limit fails it before the
/// decoder would.
#[tokio::test(start_paused = true)]
async fn a_message_that_cannot_be_decoded_ends_failed_by_the_limit_and_not_by_the_decoder() {
    let (test, runs) = harness(Some(3), vec![]).await;
    let m = enqueue(test.store(), OutgoingMessage::new(addr(&job("j")), "Nope", serde_json::json!({}))).await;
    die(&test, 3).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &m).await.error.as_deref(), Some("claim limit: 3 lost claims"));
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

/// Writes `committed:<outcome>` after each commit that succeeded.
struct CommitLog(Events);

#[async_trait]
impl Faults for CommitLog {
    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        let outcome = outcome_text(&commit.outcome);
        let result = inner.commit(commit).await?;
        if matches!(result, CommitResult::Committed { .. }) {
            self.0.push(format!("committed:{outcome}"));
        }
        Ok(result)
    }
}

/// Scenario: An observer is told after the commit.
#[tokio::test(start_paused = true)]
async fn an_observer_is_told_the_start_and_the_end_after_the_commit_of_the_limit() {
    let memory = virtual_store();
    let events = Events::default();
    let config = NodeConfig { max_lost_claims: Some(3), ..node_config("n1") };
    let system = node(Faulty::new(memory.clone(), CommitLog(events.clone())), "n1")
        .host(JobActor(Arc::new(AtomicU32::new(0))))
        .observe(Recorder::new(&events))
        .config(config)
        .build();
    system.create(&job("j"), &JobState::default()).await.unwrap();
    accepted(system.send(&job("j"), run()).await.unwrap());
    for _ in 0..3 {
        claim_and_abandon(&memory, &types(), LEASE, 1).await.unwrap();
        sleep(LEASE + Duration::from_secs(1)).await;
    }
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let failed = "Failed(error=claim limit: 3 lost claims)";
    assert_eq!(
        events.all(),
        vec![format!("committed:{failed}"), "start:Run:4".to_owned(), format!("end:{failed}:sent=0")]
    );
}

/// Commits fail as unavailable while the flag is set.
struct Unreachable(AtomicBool);

#[async_trait]
impl Faults for Unreachable {
    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        if self.0.load(Ordering::SeqCst) {
            return Err(StoreError::unavailable("store unreachable"));
        }
        inner.commit(commit).await
    }
}

/// Scenario: The commit of the limit does not succeed.
#[tokio::test(start_paused = true)]
async fn a_commit_of_the_limit_that_does_not_succeed_tells_nobody_and_the_next_claim_finds_one_more_lost_claim() {
    let memory = virtual_store();
    let events = Events::default();
    let faulty = Faulty::new(memory.clone(), Unreachable(AtomicBool::new(true)));
    let config = NodeConfig { max_lost_claims: Some(3), ..node_config("n1") };
    let system = node(faulty.clone(), "n1")
        .host(JobActor(Arc::new(AtomicU32::new(0))))
        .observe(Recorder::new(&events))
        .config(config)
        .build();
    system.create(&job("j"), &JobState::default()).await.unwrap();
    let m = accepted(system.send(&job("j"), run()).await.unwrap());
    for _ in 0..3 {
        claim_and_abandon(&memory, &types(), LEASE, 1).await.unwrap();
        sleep(LEASE + Duration::from_secs(1)).await;
    }
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1); // its commit is retried until the lease deadline
    assert!(events.all().is_empty(), "the observers are told nothing of that claim: {:?}", events.all());
    assert!(matches!(
        memory.status(&m.id).await.unwrap(),
        Some(MessageStatus::Running { .. } | MessageStatus::Pending(_))
    ));
    faulty.faults.0.store(false, Ordering::SeqCst);
    sleep(LEASE).await; // the lease runs out
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&memory, &m.id).await.expect("an outcome");
    assert_eq!(outcome.error.as_deref(), Some("claim limit: 4 lost claims"));
    assert_eq!(events.all(), vec!["start:Run:5", "end:Failed(error=claim limit: 4 lost claims):sent=0"]);
}

/// The answer of a claim arrives `delay` after the claim was made while `slow` is set: a store that answers late.
struct SlowClaims {
    slow: AtomicBool,
    delay: Duration,
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
    ) -> StoreResult<Vec<crate::store::Claim>> {
        let claims = inner.claim_due(types, node, ttl, limit).await?;
        if self.slow.load(Ordering::SeqCst) {
            sleep(self.delay).await;
        }
        Ok(claims)
    }
}

/// Scenario: A claim that arrives too late.
#[tokio::test(start_paused = true)]
async fn a_claim_that_arrives_after_its_lease_deadline_is_given_back_and_counts_no_lost_claim() {
    let memory = virtual_store();
    let events = Events::default();
    let runs = Arc::new(AtomicU32::new(0));
    let slow =
        Faulty::new(memory.clone(), SlowClaims { slow: AtomicBool::new(true), delay: LEASE + Duration::from_secs(1) });
    let system = node(slow.clone(), "n1").host(JobActor(runs.clone())).observe(Recorder::new(&events)).build();
    system.create(&job("j"), &JobState::default()).await.unwrap();
    let m = accepted(system.send(&job("j"), run()).await.unwrap());
    claim_and_abandon(&memory, &types(), LEASE, 1).await.unwrap();
    sleep(LEASE + Duration::from_secs(1)).await;

    assert_eq!(system.run_due_turns(1).await.unwrap(), 1); // its answer arrives after the lease deadline
    assert_eq!(runs.load(Ordering::SeqCst), 0, "its handler does not run");
    assert!(events.all().is_empty(), "the turn is not observed: {:?}", events.all());
    let pending = match memory.status(&m.id).await.unwrap() {
        Some(MessageStatus::Pending(message)) => message,
        other => panic!("given back, so pending without a lease: {other:?}"),
    };
    assert_eq!((pending.attempt, pending.lost_claims), (2, 1), "the abandoned claim is lost, the late one not");

    slow.faults.slow.store(false, Ordering::SeqCst);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&memory, &m.id).await.expect("an outcome");
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!((outcome.envelope.attempt, outcome.envelope.lost_claims), (3, 1), "the same lost claims as before");
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

/// Scenario: A claim that arrives shortly before its deadline. The answer comes 25 s after the claim, with 4 s of the
/// local deadline left (lease 30 s, less a margin of 1 s) and less than the heartbeat of 10 s: the keeper renews at
/// once, and the handler of 15 s runs to the end and commits.
#[tokio::test(start_paused = true)]
async fn a_claim_that_arrives_with_less_than_a_heartbeat_left_is_renewed_at_once_and_its_turn_commits() {
    let memory = virtual_store();
    let runs = Arc::new(AtomicU32::new(0));
    let slow = Faulty::new(memory.clone(), SlowClaims { slow: AtomicBool::new(true), delay: Duration::from_secs(25) });
    let system = node(slow.clone(), "n1").host(JobActor(runs.clone())).build();
    system.create(&job("j"), &JobState::default()).await.unwrap();
    let m = accepted(system.send(&job("j"), JobMsg::Run { secs: 15, fail_runs: 0 }).await.unwrap());
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&memory, &m.id).await.expect("the turn committed");
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!((outcome.envelope.attempt, outcome.envelope.lost_claims), (1, 0), "no claim is lost");
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

/// Scenario: The message is sent again.
#[tokio::test(start_paused = true)]
async fn a_message_that_failed_at_the_limit_and_is_sent_again_starts_without_lost_claims() {
    let (test, runs) = harness(Some(3), vec![]).await;
    let m = test.send(&job("j"), run()).await;
    die(&test, 3).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &m).await.status, OutcomeStatus::Failed);
    let RetryResult::Accepted(again) = test.system().retry_message(&m.id).await.unwrap() else {
        panic!("the failed message is retried")
    };
    assert_eq!(again.lost_claims, 0);
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &again).await.status, OutcomeStatus::Done);
    assert_eq!(runs.load(Ordering::SeqCst), 1, "its handler runs");
}

/// Scenario: The default.
#[tokio::test(start_paused = true)]
async fn the_default_limit_is_five_lost_claims() {
    assert_eq!(NodeConfig::default().max_lost_claims, Some(5));
    let (test, runs) = harness(NodeConfig::default().max_lost_claims, vec![]).await;
    let m = test.send(&job("j"), run()).await;
    die(&test, 5).await;
    test.run_until_idle().await;
    assert_eq!(outcome(&test, &m).await.error.as_deref(), Some("claim limit: 5 lost claims"));
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

/// Scenario: No limit.
#[tokio::test(start_paused = true)]
async fn without_a_limit_a_message_with_twenty_lost_claims_runs() {
    let (test, runs) = harness(None, vec![]).await;
    let m = test.send(&job("j"), run()).await;
    die(&test, 20).await;
    test.run_until_idle().await;
    let outcome = outcome(&test, &m).await;
    assert_eq!((outcome.status, outcome.envelope.lost_claims), (OutcomeStatus::Done, 20));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

/// Scenario: A limit of zero.
#[tokio::test(start_paused = true)]
async fn a_node_with_a_limit_of_zero_does_not_start() {
    let Err(error) = TestActors::builder()
        .with(|n| {
            n.host(JobActor(Arc::new(AtomicU32::new(0))))
                .config(NodeConfig { max_lost_claims: Some(0), ..NodeConfig::default() })
        })
        .build()
        .await
    else {
        panic!("a node with a limit of zero starts");
    };
    assert!(
        error.problems.iter().any(|p| matches!(p, StartProblem::Other(m) if m.contains("max_lost_claims"))),
        "{error}"
    );
    let system = ActorSystem::builder(virtual_store())
        .config(NodeConfig { max_lost_claims: Some(0), ..NodeConfig::default() })
        .build();
    assert!(system.start().await.unwrap_err().to_string().contains("max_lost_claims"));
}
