//! The claim limit at external lease admission (spec `claim-limit`, ADR 0037): `lease_turns` never hands out a message at
//! the serving system's limit. One fenced failure commit each, without a handler or the retry policy; watchers and then
//! observers hear only of a confirmed commit, and every other one is released.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use super::common::*;
use crate::actor::contract_of;
use crate::address::{ActorAddress, TypeName};
use crate::prelude::*;
use crate::runtime::{LeaseTurnsResult, NodeConfig, TurnLease};
use crate::store::{Backend, CommitRequest, CommitResult, Message, MessageStatus, NodeId, StoreError, StoreResult};
use crate::testing::claim_and_abandon;
use crate::{ActorSystem, Failed, FailureCause, Stopped, Store, WatchTarget};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Job {
    id: String,
}

impl ActorRef for Job {
    const TYPE: &'static str = "job";
    type Message = JobMsg;
}

pub(super) fn job(id: &str) -> Job {
    Job { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub(super) enum JobMsg {
    Run,
    Failed(Failed),
    Stopped(Stopped),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub(super) struct JobState {}

/// The job type; the servers here only know its contract.
pub(super) struct JobActor;

impl Actor for JobActor {
    type Ref = Job;
    type State = JobState;

    async fn handle(&self, _: &mut Turn<Self>, _: JobMsg) -> HandlerResult {
        Ok(())
    }
}

pub(super) const LEASE: Duration = Duration::from_secs(30);
pub(super) const LIMIT_FAILURE: &str = "Failed(error=claim limit: 3 lost claims)";

pub(super) fn types() -> BTreeSet<TypeName> {
    BTreeSet::from([type_name::<Job>()])
}

pub(super) fn selection() -> BTreeMap<TypeName, String> {
    BTreeMap::from([(type_name::<Job>(), contract_of::<JobActor>().unwrap().hash().to_owned())])
}

pub(super) fn worker() -> NodeId {
    NodeId::new("worker").unwrap()
}

/// A server that hosts nothing, with the claim limit `limit`, the TTL of 30 s and an observer writing to `events`.
async fn server(store: impl IntoStore, limit: Option<u32>, events: &Events) -> ActorSystem {
    let config = NodeConfig { max_lost_claims: limit, ..node_config("server") };
    let system = node(store, "server").config(config).observe(Recorder::new(events)).build();
    system.register_contract(&contract_of::<JobActor>().unwrap()).await.unwrap();
    system
}

/// Creates `job/<id>`, sends it `Run` with `options` and lets `lost` claims of it run out: the next claim finds `lost`
/// lost claims. Only that job may have a due message while it runs.
pub(super) async fn job_with_lost_claims(
    memory: &Store,
    system: &ActorSystem,
    id: &str,
    lost: u32,
    options: SendOptions,
) -> Message {
    system.create(&job(id), &JobState::default()).await.unwrap();
    let m = accepted(system.send_with(&job(id), JobMsg::Run, options).await.unwrap());
    for _ in 0..lost {
        assert_eq!(claim_and_abandon(memory, &types(), LEASE, 1).await.unwrap().len(), 1);
        sleep(LEASE + Duration::from_secs(1)).await;
    }
    m
}

pub(super) async fn lease(system: &ActorSystem) -> Vec<TurnLease> {
    match system.lease_turns(&worker(), &selection(), 10).await.unwrap() {
        LeaseTurnsResult::Leased(leases) => leases,
        other => panic!("expected Leased, got {other:?}"),
    }
}

/// The `Failed` notifications waiting in the mailbox of `watcher`.
pub(super) async fn notifications(memory: &Store, watcher: &ActorAddress) -> Vec<Message> {
    let page = memory.pending(watcher, None, 10).await.unwrap();
    page.items.into_iter().filter(|m| m.notification && m.message_type == "Failed").collect()
}

pub(super) async fn pending(memory: &Store, m: &Message) -> Message {
    match memory.status(&m.id).await.unwrap() {
        Some(MessageStatus::Pending(message)) => message,
        other => panic!("expected pending, got {other:?}"),
    }
}

/// Scenario: A mixed batch. Scenario: A watcher is notified once.
#[tokio::test(start_paused = true)]
async fn a_mixed_batch_returns_only_the_runnable_message_and_fails_the_one_at_the_limit() {
    let memory = virtual_store();
    let events = Events::default();
    let system = server(memory.clone(), Some(3), &events).await;
    let limited = job_with_lost_claims(&memory, &system, "limited", 3, SendOptions::default()).await;
    for watcher in ["watcher-a", "watcher-b"] {
        system.create(&job(watcher), &JobState::default()).await.unwrap();
        system.watch(&WatchTarget::from(addr(&job("limited"))), &addr(&job(watcher))).await.unwrap();
    }
    system.create(&job("runnable"), &JobState::default()).await.unwrap();
    let runnable = accepted(system.send(&job("runnable"), JobMsg::Run).await.unwrap());

    let leases = lease(&system).await;

    assert_eq!(leases.iter().map(|l| l.message.id.clone()).collect::<Vec<_>>(), vec![runnable.id]);
    let outcome = outcome_of(&memory, &limited.id).await.expect("failed at the limit");
    assert_eq!(outcome.error.as_deref(), Some("claim limit: 3 lost claims"));
    assert_eq!((outcome.envelope.attempt, outcome.envelope.lost_claims, outcome.envelope.failures), (4, 3, 0));
    assert_eq!(events.all(), vec!["start:Run:4".to_owned(), format!("end:{LIMIT_FAILURE}:sent=0")]);
    for watcher in ["watcher-a", "watcher-b"] {
        let told = notifications(&memory, &addr(&job(watcher))).await;
        assert_eq!(told.len(), 1, "{watcher}: {told:?}");
        let failed: Failed = serde_json::from_value(told[0].payload.clone()).unwrap();
        assert_eq!((failed.message_id, failed.cause), (limited.id.clone(), FailureCause::ClaimLimit));
        assert_eq!(failed.error, "claim limit: 3 lost claims");
    }
    assert_eq!(system.running_turns(), 0);
}

/// Scenario: All claims reach the limit. The retry policy is not asked.
#[tokio::test(start_paused = true)]
async fn when_every_claim_is_at_the_limit_nothing_is_leased_and_the_retry_policy_is_not_asked() {
    let memory = virtual_store();
    let events = Events::default();
    let system = server(memory.clone(), Some(3), &events).await;
    let retry = RetryPolicy::new(10, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    let m = job_with_lost_claims(&memory, &system, "j", 3, SendOptions::new().retry(retry)).await;

    assert!(lease(&system).await.is_empty());

    let outcome = outcome_of(&memory, &m.id).await.expect("failed, not rescheduled");
    assert_eq!((outcome.envelope.failures, outcome.error.as_deref()), (0, Some("claim limit: 3 lost claims")));
    assert!(memory.pending(&addr(&job("j")), None, 10).await.unwrap().items.is_empty());
    assert_eq!(events.all(), vec!["start:Run:4".to_owned(), format!("end:{LIMIT_FAILURE}:sent=0")]);
}

/// Scenario: Claim limit disabled.
#[tokio::test(start_paused = true)]
async fn without_a_limit_a_message_with_many_lost_claims_is_leased() {
    let memory = virtual_store();
    let events = Events::default();
    let system = server(memory.clone(), None, &events).await;
    let m = job_with_lost_claims(&memory, &system, "j", 6, SendOptions::default()).await;

    let leases = lease(&system).await;

    assert_eq!(leases.len(), 1);
    assert_eq!((leases[0].message.id.clone(), leases[0].message.lost_claims), (m.id, 6));
    assert!(events.all().is_empty(), "a returned lease starts no observer: {:?}", events.all());
}

/// Scenario: An observer is told after the commit, for the external path too.
#[tokio::test(start_paused = true)]
async fn the_observers_hear_the_start_and_the_end_only_after_the_commit() {
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

    let memory = virtual_store();
    let events = Events::default();
    let system = server(Faulty::new(memory.clone(), CommitLog(events.clone())), Some(3), &events).await;
    job_with_lost_claims(&memory, &system, "j", 3, SendOptions::default()).await;

    assert!(lease(&system).await.is_empty());

    assert_eq!(
        events.all(),
        vec![format!("committed:{LIMIT_FAILURE}"), "start:Run:4".to_owned(), format!("end:{LIMIT_FAILURE}:sent=0")]
    );
}

/// How the one commit of the claim limit goes wrong, then succeeds.
const FAIL_BEFORE: u8 = 0;
const REFUSE: u8 = 1;
const LEASE_LOST: u8 = 2;
const APPLY_THEN_FAIL: u8 = 3;
const PASS: u8 = 4;

/// The first commit goes wrong as its mode says; later ones pass.
struct OneBadCommit(AtomicU8);

#[async_trait]
impl Faults for OneBadCommit {
    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        match self.0.swap(PASS, Ordering::SeqCst) {
            FAIL_BEFORE => Err(StoreError::unavailable("store unreachable")),
            REFUSE => Err(StoreError::Invalid("refused".into())),
            LEASE_LOST => Ok(CommitResult::LeaseLost),
            APPLY_THEN_FAIL => {
                inner.commit(commit).await?;
                Err(StoreError::unavailable("the answer was lost"))
            }
            _ => inner.commit(commit).await,
        }
    }
}

/// Scenario: Failed or uncertain claim-limit commit: a commit that fails, is refused or answers `LeaseLost` hands out
/// nothing, tells nobody and releases the epoch; the message stays at the limit, and the next claim fails it.
#[tokio::test(start_paused = true)]
async fn a_claim_limit_commit_that_does_not_apply_is_released_and_the_next_claim_fails_the_message() {
    for mode in [FAIL_BEFORE, REFUSE, LEASE_LOST] {
        let memory = virtual_store();
        let events = Events::default();
        let system = server(Faulty::new(memory.clone(), OneBadCommit(AtomicU8::new(mode))), Some(3), &events).await;
        let m = job_with_lost_claims(&memory, &system, "j", 3, SendOptions::default()).await;

        assert!(lease(&system).await.is_empty(), "mode {mode}");

        assert!(events.all().is_empty(), "mode {mode}: observers silent: {:?}", events.all());
        settle().await;
        let waiting = pending(&memory, &m).await;
        assert_eq!((waiting.attempt, waiting.lost_claims), (4, 3), "mode {mode}: released, still at the limit");
        assert!(memory.running_turn(&addr(&job("j"))).await.unwrap().is_none(), "mode {mode}: released at once");

        assert!(lease(&system).await.is_empty(), "mode {mode}");
        let outcome = outcome_of(&memory, &m.id).await.expect("the next claim fails it");
        assert_eq!(outcome.error.as_deref(), Some("claim limit: 3 lost claims"), "mode {mode}");
        assert_eq!(events.all(), vec!["start:Run:5".to_owned(), format!("end:{LIMIT_FAILURE}:sent=0")], "mode {mode}");
    }
}

/// Scenario: Failed or uncertain claim-limit commit, when it applied but its answer was lost. Scenario: A watcher is
/// notified once: the release that follows changes nothing, and the observers stay silent.
#[tokio::test(start_paused = true)]
async fn a_claim_limit_commit_that_applied_without_an_answer_is_kept_and_its_watcher_told_once() {
    let memory = virtual_store();
    let events = Events::default();
    let system =
        server(Faulty::new(memory.clone(), OneBadCommit(AtomicU8::new(APPLY_THEN_FAIL))), Some(3), &events).await;
    let m = job_with_lost_claims(&memory, &system, "j", 3, SendOptions::default()).await;
    system.create(&job("watcher"), &JobState::default()).await.unwrap();
    system.watch(&WatchTarget::from(addr(&job("j"))), &addr(&job("watcher"))).await.unwrap();

    assert!(lease(&system).await.is_empty());
    settle().await;

    let outcome = outcome_of(&memory, &m.id).await.expect("the commit applied");
    assert_eq!(outcome.error.as_deref(), Some("claim limit: 3 lost claims"));
    assert!(events.all().is_empty(), "unconfirmed: the observers stay silent: {:?}", events.all());
    assert_eq!(notifications(&memory, &addr(&job("watcher"))).await.len(), 1, "told once");
    let rest = lease(&system).await;
    assert!(rest.iter().all(|l| l.actor != addr(&job("j"))), "nothing of j left to lease: {rest:?}");
    assert_eq!(notifications(&memory, &addr(&job("watcher"))).await.len(), 1, "still once");
}
