//! A worker's per-turn deadlines and its life cycle over a scripted turn source (spec `remote-workers`, ADR 0039), on
//! virtual time: the heartbeat and grant rules of short TTLs, a peer's shorter TTL, late grants and immediate renewal,
//! renewals that fail, hang, panic or report the lease lost, a renewal and the watchdog ready at once, a completion
//! that would start at the deadline, malformed batches, lease responses that come after their caller left or after a
//! stop, dropped manual batches, and stopping a worker that was never started.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::{Instant, sleep};

use super::common::*;
use crate::ActorSystem;
use crate::actor::contract_of;
use crate::address::{ActorAddress, TypeName};
use crate::contract::Contract;
use crate::runtime::executor::{Holder, MIN_GRANT};
use crate::runtime::{
    CompleteTurnResult, LeaseTurnsResult, NodeConfig, NodeState, RegisterContractResult, RenewTurnResult, SourceError,
    TurnLease, TurnReport, TurnSource, Worker, WorkerConfig, WorkerError,
};
use crate::store::{Message, MessageId, MessageStatus, NodeId, OutcomeStatus, Store, TypeDef};

/// Edits a leased batch before the worker gets it.
pub(super) type LeaseEdit = fn(&mut Vec<TurnLease>);

/// How the scripted source answers renewals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Renew {
    Pass,
    Fail,
    Hang,
    Panic,
    Lost,
    /// Waits, then renews.
    Delay(Duration),
}

/// How the scripted source answers one completion; the queue's front is the next call, and an empty queue passes.
#[derive(Debug, Clone)]
pub(super) enum Complete {
    Pass,
    /// Fails without applying.
    Unavailable,
    /// Applies, then answers unavailable.
    LoseAfter,
    /// Never answers, and applies nothing.
    Hang,
    /// Applies, then never answers.
    HangAfter,
    /// Answers this without asking the server.
    Answer(CompleteTurnResult),
    /// Refuses as invalid.
    Invalid,
}

/// A turn source in front of a server that hosts nothing, with scripted delays, grants, renewals and completions, and
/// a record of every renewal, release and completion.
pub(super) struct Scripted {
    pub server: ActorSystem,
    /// Where renewals go: the server, or a peer with another TTL.
    pub renewer: ActorSystem,
    pub lease_delay: Mutex<Duration>,
    pub lease_ttl: Mutex<Option<Duration>>,
    pub lease_edit: Mutex<Option<LeaseEdit>>,
    pub renew: Mutex<Renew>,
    pub complete: Mutex<VecDeque<Complete>>,
    pub leases: AtomicUsize,
    pub renews: AtomicUsize,
    pub releases: AtomicUsize,
    pub completes: AtomicUsize,
    pub applied: AtomicUsize,
    pub reports: Mutex<Vec<TurnReport>>,
}

impl Scripted {
    pub fn new(server: &ActorSystem) -> Arc<Self> {
        Self::with_renewer(server, server)
    }

    pub fn with_renewer(server: &ActorSystem, renewer: &ActorSystem) -> Arc<Self> {
        Arc::new(Self {
            server: server.clone(),
            renewer: renewer.clone(),
            lease_delay: Mutex::new(Duration::ZERO),
            lease_ttl: Mutex::new(None),
            lease_edit: Mutex::new(None),
            renew: Mutex::new(Renew::Pass),
            complete: Mutex::new(VecDeque::new()),
            leases: AtomicUsize::new(0),
            renews: AtomicUsize::new(0),
            releases: AtomicUsize::new(0),
            completes: AtomicUsize::new(0),
            applied: AtomicUsize::new(0),
            reports: Mutex::new(Vec::new()),
        })
    }

    pub fn set_renew(&self, renew: Renew) {
        *self.renew.lock().unwrap() = renew;
    }

    pub fn script(&self, completes: impl IntoIterator<Item = Complete>) {
        self.complete.lock().unwrap().extend(completes);
    }

    pub fn count(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }

    pub fn reports(&self) -> Vec<TurnReport> {
        self.reports.lock().unwrap().clone()
    }
}

#[async_trait]
impl TurnSource for Scripted {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        TurnSource::register_contract(&self.server, contract).await
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        TurnSource::list_types(&self.server).await
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &std::collections::BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        self.leases.fetch_add(1, Ordering::SeqCst);
        let mut result = TurnSource::lease_turns(&self.server, owner, types, limit).await?;
        if let LeaseTurnsResult::Leased(leases) = &mut result {
            if let Some(ttl) = *self.lease_ttl.lock().unwrap() {
                leases.iter_mut().for_each(|l| l.ttl = ttl);
            }
            if let Some(edit) = *self.lease_edit.lock().unwrap() {
                edit(leases);
            }
        }
        let delay = *self.lease_delay.lock().unwrap();
        sleep(delay).await;
        Ok(result)
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        self.renews.fetch_add(1, Ordering::SeqCst);
        let renew = *self.renew.lock().unwrap();
        match renew {
            Renew::Pass => {}
            Renew::Fail => return Err(SourceError::Unavailable("renewal failed".into())),
            Renew::Hang => std::future::pending::<()>().await,
            Renew::Panic => panic!("renewal panicked"),
            Renew::Lost => return Ok(RenewTurnResult::LeaseLost),
            Renew::Delay(delay) => sleep(delay).await,
        }
        TurnSource::renew_turn(&self.renewer, actor, epoch).await
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        TurnSource::release_turn(&self.server, actor, epoch).await
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        self.completes.fetch_add(1, Ordering::SeqCst);
        self.reports.lock().unwrap().push(report.clone());
        let mode = self.complete.lock().unwrap().pop_front().unwrap_or(Complete::Pass);
        let apply = || async {
            let result = TurnSource::complete_turn(&self.server, actor, epoch, message_id, report.clone()).await;
            if matches!(result, Ok(CompleteTurnResult::Completed(_))) {
                self.applied.fetch_add(1, Ordering::SeqCst);
            }
            result
        };
        match mode {
            Complete::Pass => apply().await,
            Complete::Unavailable => Err(SourceError::Unavailable("completion failed".into())),
            Complete::LoseAfter => {
                apply().await?;
                Err(SourceError::Unavailable("the answer was lost".into()))
            }
            Complete::Hang => std::future::pending().await,
            Complete::HangAfter => {
                apply().await?;
                std::future::pending().await
            }
            Complete::Answer(answer) => Ok(answer),
            Complete::Invalid => Err(SourceError::Invalid("refused".into())),
        }
    }
}

pub(super) const TTL: Duration = Duration::from_secs(30);
/// The conservative deadline of a lease of [`TTL`] granted at once: 30 s less one second.
pub(super) const DEADLINE: Duration = Duration::from_secs(29);

pub(super) fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A server on `store` that hosts nothing, with lease TTL `ttl`.
pub(super) fn server(store: &Store, name: &str, ttl: Duration) -> ActorSystem {
    ActorSystem::builder(store.clone())
        .config(NodeConfig { node_id: NodeId::new(name).unwrap(), lease_ttl: ttl, ..NodeConfig::default() })
        .build()
}

pub(super) fn worker_config() -> WorkerConfig {
    WorkerConfig { worker_id: NodeId::new("worker").unwrap(), concurrency: 4, ..WorkerConfig::default() }
}

/// A worker hosting the counter through `source`, observed by a recorder of `events`.
pub(super) fn worker(source: &Arc<Scripted>, events: &Events) -> Worker {
    Worker::builder(source.clone()).host(CounterActor).observe(Recorder::new(events)).config(worker_config()).build()
}

/// Registers the counter's contract and creates the counters `ids`.
pub(super) async fn setup(server: &ActorSystem, ids: &[&str]) {
    server.register_contract(&contract_of::<CounterActor>().unwrap()).await.unwrap();
    for id in ids {
        server.create(&Counter::new(id), &CounterState::default()).await.unwrap();
    }
}

pub(super) async fn send(server: &ActorSystem, id: &str, message: CounterMsg) -> Message {
    accepted(server.send(&Counter::new(id), message).await.unwrap())
}

pub(super) async fn status(server: &ActorSystem, id: &MessageId) -> MessageStatus {
    server.status(id).await.unwrap().expect("a known message")
}

pub(super) async fn total(server: &ActorSystem, id: &str) -> i32 {
    let snapshot = server.get_actor(&Counter::new(id).address()).await.unwrap().expect("the counter");
    serde_json::from_value::<CounterState>(snapshot.state).unwrap().total
}

/// The pending message `id` with its attempt and lost claims, when it is pending.
async fn pending(server: &ActorSystem, id: &MessageId) -> Option<Message> {
    match status(server, id).await {
        MessageStatus::Pending(m) => Some(m),
        _ => None,
    }
}

/// Runs one manual batch of the worker in the background.
fn run(worker: &Worker) -> tokio::task::JoinHandle<Result<usize, WorkerError>> {
    let worker = worker.clone();
    tokio::spawn(async move { worker.run_due_turns(10).await })
}

/// Scenario: Very short but valid TTL.
#[test]
fn a_source_heartbeat_is_a_third_of_the_ttl_without_a_millisecond_floor_and_tiny_grants_are_refused() {
    let source: Arc<dyn TurnSource> = Scripted::new(&ActorSystem::builder(Store::memory()).build());
    let holder = Holder::Source(source);
    assert_eq!(holder.heartbeat(Duration::from_micros(900)), Duration::from_micros(300));
    assert_eq!(holder.heartbeat(Duration::from_nanos(2)), Duration::from_nanos(1));
    let now = Instant::now();
    assert!(holder.grant(now, Duration::from_micros(900)).is_some());
    assert_eq!(holder.grant(now, MIN_GRANT), Some(now + MIN_GRANT));
    assert_eq!(holder.grant(now, Duration::from_nanos(1)), None);
    assert_eq!(holder.grant(now, Duration::MAX), None, "an unrepresentable deadline");
}

/// Scenario: Different peer TTLs.
#[tokio::test(start_paused = true)]
async fn a_peers_shorter_ttl_keeps_the_later_deadline_and_sets_the_next_heartbeat() {
    let store = virtual_store();
    let peer = server(&store, "peer", secs(3));
    let server = server(&store, "server", TTL);
    setup(&server, &["a"]).await;
    let m = send(&server, "a", CounterMsg::Sleep { millis: 40_000 }).await;
    let source = Scripted::with_renewer(&server, &peer);
    let events = Events::default();
    let worker = worker(&source, &events);
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 1);
    // The first renewal (after 10 s) keeps the deadline of 29 s and makes the heartbeat 1 s: the turn of 40 s is
    // renewed about once a second from then on, and completes.
    let renewals = Scripted::count(&source.renews);
    assert!((25..=35).contains(&renewals), "{renewals} renewals");
    assert_eq!(server.outcome(&m.id).await.unwrap().unwrap().status, OutcomeStatus::Done);
    assert_eq!(events.all(), ["start:Sleep:1", "end:Done:sent=0"]);
}

/// Scenario: Late grant and immediate renewal.
#[tokio::test(start_paused = true)]
async fn a_grant_that_arrives_after_its_deadline_starts_nothing_and_is_released() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a"]).await;
    let m = send(&server, "a", CounterMsg::Add { n: 1 }).await;
    let source = Scripted::new(&server);
    *source.lease_delay.lock().unwrap() = DEADLINE;
    let events = Events::default();
    let worker = worker(&source, &events);
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 1, "a task was started for it");
    assert!(events.all().is_empty(), "no observer, load or handler");
    assert_eq!(Scripted::count(&source.releases), 1);
    assert_eq!(Scripted::count(&source.completes), 0);
    let message = pending(&server, &m.id).await.expect("pending again");
    assert_eq!((message.attempt, message.lost_claims), (1, 0));
    assert_eq!(total(&server, "a").await, 0);
}

/// Scenario: Late grant and immediate renewal.
#[tokio::test(start_paused = true)]
async fn a_grant_with_less_than_a_heartbeat_left_is_renewed_at_once() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a"]).await;
    let m = send(&server, "a", CounterMsg::Sleep { millis: 5_000 }).await;
    let source = Scripted::new(&server);
    *source.lease_delay.lock().unwrap() = secs(25);
    let events = Events::default();
    let worker = worker(&source, &events);
    let batch = run(&worker);
    sleep(secs(25) + Duration::from_millis(1)).await;
    assert_eq!(Scripted::count(&source.renews), 1, "renewed on arrival, not one heartbeat (10 s) later");
    batch.await.unwrap().unwrap();
    assert_eq!(server.outcome(&m.id).await.unwrap().unwrap().status, OutcomeStatus::Done);
}

/// Scenario: Hanging renewal or keeper panic.
#[tokio::test(start_paused = true)]
async fn renewals_that_fail_hang_panic_or_report_the_lease_lost_abort_the_work_without_completion_or_release() {
    for renew in [Renew::Fail, Renew::Hang, Renew::Panic, Renew::Lost] {
        let store = virtual_store();
        let server = server(&store, "server", TTL);
        setup(&server, &["a"]).await;
        let m = send(&server, "a", CounterMsg::Sleep { millis: 50_000 }).await;
        let source = Scripted::new(&server);
        source.set_renew(renew);
        let events = Events::default();
        let worker = worker(&source, &events);
        let started = Instant::now();
        assert_eq!(worker.run_due_turns(10).await.unwrap(), 1);
        let ended = started.elapsed();
        if matches!(renew, Renew::Lost | Renew::Panic) {
            assert_eq!(ended, secs(10), "{renew:?}: at the first renewal");
        } else {
            assert_eq!(ended, DEADLINE, "{renew:?}: at the local deadline");
        }
        assert_eq!(Scripted::count(&source.completes), 0, "{renew:?}");
        assert_eq!(Scripted::count(&source.releases), 0, "{renew:?}");
        assert_eq!(events.all(), ["start:Sleep:1", "dropped:Sleep"], "{renew:?}");
        assert!(matches!(status(&server, &m.id).await, MessageStatus::Running { .. }), "{renew:?}: left to expire");
    }
}

/// Scenario: Renewal and watchdog become ready together.
#[tokio::test(start_paused = true)]
async fn a_renewal_answered_exactly_at_the_deadline_cannot_revive_the_grant() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a"]).await;
    send(&server, "a", CounterMsg::Sleep { millis: 40_000 }).await;
    let source = Scripted::new(&server);
    // The first renewal is sent after 10 s and answered at 29 s, the local deadline, as the watchdog wakes.
    source.set_renew(Renew::Delay(secs(19)));
    let events = Events::default();
    let worker = worker(&source, &events);
    let started = Instant::now();
    worker.run_due_turns(10).await.unwrap();
    assert_eq!(started.elapsed(), DEADLINE);
    assert_eq!(Scripted::count(&source.renews), 1);
    assert_eq!(Scripted::count(&source.completes), 0, "the grant expired: no completion");
    assert_eq!(events.all(), ["start:Sleep:1", "dropped:Sleep"]);
}

/// Scenario: Completion starts at expiry.
#[tokio::test(start_paused = true)]
async fn a_handler_that_ends_at_the_deadline_sends_no_completion_even_with_a_renewal_ready_then() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a"]).await;
    send(&server, "a", CounterMsg::Sleep { millis: 29_000 }).await;
    let source = Scripted::new(&server);
    source.set_renew(Renew::Delay(secs(19)));
    let events = Events::default();
    let worker = worker(&source, &events);
    worker.run_due_turns(10).await.unwrap();
    assert_eq!(Scripted::count(&source.completes), 0);
    assert_eq!(Scripted::count(&source.releases), 0, "never released after the handler ended");
    assert_eq!(events.all(), ["start:Sleep:1", "dropped:Sleep"]);
}

/// Scenario: Malformed batch metadata.
#[tokio::test(start_paused = true)]
async fn a_malformed_batch_runs_nothing_is_released_and_stops_the_worker_with_a_fatal_error() {
    fn twice(leases: &mut Vec<TurnLease>) {
        let copy = leases[0].clone();
        leases.push(copy);
    }
    #[allow(clippy::ptr_arg)]
    fn misaddressed(leases: &mut Vec<TurnLease>) {
        leases[0].message.to = Counter::new("other").address();
    }
    #[allow(clippy::ptr_arg)]
    fn unrequested(leases: &mut Vec<TurnLease>) {
        leases[0].actor = addr(&Echo::new("x"));
    }
    let cases: [(&str, Option<LeaseEdit>, Option<Duration>); 4] = [
        ("twice", Some(twice), None),
        ("misaddressed", Some(misaddressed), None),
        ("unrequested", Some(unrequested), None),
        ("ttl", None, Some(Duration::from_nanos(1))),
    ];
    for (label, edit, ttl) in cases {
        let store = virtual_store();
        let server = server(&store, "server", TTL);
        setup(&server, &["a"]).await;
        let m = send(&server, "a", CounterMsg::Add { n: 1 }).await;
        let source = Scripted::new(&server);
        *source.lease_edit.lock().unwrap() = edit;
        *source.lease_ttl.lock().unwrap() = ttl;
        let events = Events::default();
        let worker = worker(&source, &events);
        let error = worker.run_due_turns(10).await.unwrap_err();
        match &error {
            WorkerError::Source(SourceError::Fatal(why)) => assert!(why.starts_with("Malformed lease batch"), "{why}"),
            other => panic!("{label}: expected Fatal, got {other:?}"),
        }
        assert!(matches!(worker.stopped().await, Some(WorkerError::Source(SourceError::Fatal(_)))), "{label}");
        settle().await;
        assert!(events.all().is_empty(), "{label}: nothing ran");
        assert!(Scripted::count(&source.releases) >= 1, "{label}: released");
        if label == "unrequested" {
            // Its lease names another actor: the release cannot find it, and it expires.
            assert!(matches!(status(&server, &m.id).await, MessageStatus::Running { .. }), "{label}");
        } else {
            assert!(pending(&server, &m.id).await.is_some(), "{label}: pending again");
        }
        assert_eq!(total(&server, "a").await, 0, "{label}");
    }
}

/// Scenario: Lease response after caller disappearance.
#[tokio::test(start_paused = true)]
async fn a_lease_response_after_its_caller_left_is_released_and_its_slots_are_returned() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a", "b"]).await;
    let a = send(&server, "a", CounterMsg::Add { n: 1 }).await;
    let source = Scripted::new(&server);
    *source.lease_delay.lock().unwrap() = secs(1);
    let events = Events::default();
    let worker = Worker::builder(source.clone())
        .host(CounterActor)
        .observe(Recorder::new(&events))
        .config(WorkerConfig { concurrency: 1, ..worker_config() })
        .build();
    let batch = run(&worker);
    sleep(Duration::from_millis(500)).await;
    batch.abort();
    sleep(secs(1)).await;
    assert_eq!(Scripted::count(&source.releases), 1);
    assert!(events.all().is_empty());
    let message = pending(&server, &a.id).await.expect("pending again");
    assert_eq!(message.lost_claims, 0);
    // The slot is free again.
    *source.lease_delay.lock().unwrap() = Duration::ZERO;
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 1);
    assert_eq!(total(&server, "a").await, 1);
}

/// Scenario: Lease response after caller disappearance.
#[tokio::test(start_paused = true)]
async fn a_lease_response_after_a_stop_is_released() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a"]).await;
    let a = send(&server, "a", CounterMsg::Add { n: 1 }).await;
    let source = Scripted::new(&server);
    *source.lease_delay.lock().unwrap() = secs(1);
    let events = Events::default();
    let worker = worker(&source, &events);
    let batch = run(&worker);
    sleep(Duration::from_millis(500)).await;
    worker.stop().await;
    assert!(matches!(batch.await.unwrap(), Err(WorkerError::Start(_))), "the stop won the hand-off");
    settle().await;
    assert_eq!(Scripted::count(&source.releases), 1);
    assert!(events.all().is_empty());
    assert!(pending(&server, &a.id).await.is_some());
}

/// Scenario: Drop one manual batch.
#[tokio::test(start_paused = true)]
async fn dropping_one_manual_batch_cancels_only_its_turns_which_hold_their_slots_until_cleanup() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a", "b"]).await;
    let a = send(&server, "a", CounterMsg::Sleep { millis: 5_000 }).await;
    let source = Scripted::new(&server);
    let events = Events::default();
    let worker = Worker::builder(source.clone())
        .host(CounterActor)
        .observe(Recorder::new(&events))
        .config(WorkerConfig { concurrency: 2, ..worker_config() })
        .build();
    let first = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_due_turns(1).await }
    });
    sleep(Duration::from_millis(10)).await;
    let b = send(&server, "b", CounterMsg::Sleep { millis: 5_000 }).await;
    let second = run(&worker);
    sleep(Duration::from_millis(10)).await;
    assert_eq!(worker.running_turns(), 2);
    first.abort();
    // The release of `a` waits until its handler stopped; its slot stays taken meanwhile.
    settle().await;
    assert_eq!(Scripted::count(&source.releases), 1);
    assert!(pending(&server, &a.id).await.is_some());
    assert_eq!(second.await.unwrap().unwrap(), 1, "the other cohort ran on");
    assert_eq!(server.outcome(&b.id).await.unwrap().unwrap().status, OutcomeStatus::Done);
    assert_eq!(worker.running_turns(), 0);
}

/// Scenario: Stop an unstarted Worker with a manual batch.
#[tokio::test(start_paused = true)]
async fn stopping_a_new_worker_drains_and_cancels_its_manual_turns() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a", "b"]).await;
    let short = send(&server, "a", CounterMsg::Sleep { millis: 1_000 }).await;
    let long = send(&server, "b", CounterMsg::Sleep { millis: 60_000 }).await;
    let source = Scripted::new(&server);
    let events = Events::default();
    let worker = worker(&source, &events);
    let batch = run(&worker);
    sleep(Duration::from_millis(10)).await;
    assert_eq!(worker.state(), NodeState::New);
    assert_eq!(worker.running_turns(), 2);
    worker.stop_with_grace(secs(5)).await;
    assert_eq!(worker.state(), NodeState::Stopped);
    assert_eq!(worker.running_turns(), 0, "drained, not treated as idle");
    assert_eq!(server.outcome(&short.id).await.unwrap().unwrap().status, OutcomeStatus::Done);
    assert!(pending(&server, &long.id).await.is_some(), "the unfinished turn was released");
    assert_eq!(Scripted::count(&source.releases), 1);
    assert_eq!(batch.await.unwrap().unwrap(), 2);
    assert!(worker.stopped().await.is_none());
}

/// A started worker of `source` running the counter turn `a`, which sleeps for a minute, and its message.
async fn worker_with_a_long_turn(server: &ActorSystem, source: &Arc<Scripted>) -> (Worker, Message) {
    setup(server, &["a"]).await;
    let a = send(server, "a", CounterMsg::Sleep { millis: 60_000 }).await;
    let worker = worker(source, &Events::default());
    worker.start().await.unwrap();
    eventually(secs(5), || async { worker.running_turns() == 1 }).await;
    (worker, a)
}

fn stop_in_background(worker: &Worker, grace: Duration) -> tokio::task::JoinHandle<()> {
    let worker = worker.clone();
    tokio::spawn(async move { worker.stop_with_grace(grace).await })
}

/// Scenario: Ending the grace period during a stop.
#[tokio::test(start_paused = true)]
async fn end_grace_cancels_the_running_turns_at_once_and_the_stop_returns_within_the_cancel_timeout() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    let source = Scripted::new(&server);
    let (worker, a) = worker_with_a_long_turn(&server, &source).await;
    let stop = stop_in_background(&worker, secs(30));
    sleep(secs(1)).await;
    assert_eq!(worker.state(), NodeState::Stopping);
    let mark = Instant::now();
    worker.end_grace();
    worker.end_grace(); // a second call changes nothing
    stop.await.unwrap();
    assert!(mark.elapsed() < worker_config().shutdown_cancel_timeout, "stopped after {:?}", mark.elapsed());
    assert_eq!(worker.state(), NodeState::Stopped);
    assert_eq!(Scripted::count(&source.releases), 1, "the cancelled turn gave its lease back");
    assert!(pending(&server, &a.id).await.is_some());
    worker.end_grace(); // after the stop: nothing
    worker.stop().await; // returns at once
    assert_eq!(Scripted::count(&source.releases), 1);
}

/// `end_grace` right after the stop began, before the stop reaches its grace wait, is not lost.
#[tokio::test(start_paused = true)]
async fn end_grace_before_the_stop_reaches_its_grace_wait_still_ends_it() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    let source = Scripted::new(&server);
    let (worker, a) = worker_with_a_long_turn(&server, &source).await;
    let mark = Instant::now();
    let stop = stop_in_background(&worker, secs(30));
    while worker.state() != NodeState::Stopping {
        tokio::task::yield_now().await;
    }
    worker.end_grace();
    stop.await.unwrap();
    assert!(mark.elapsed() < worker_config().shutdown_cancel_timeout, "stopped after {:?}", mark.elapsed());
    assert_eq!(Scripted::count(&source.releases), 1);
    assert!(pending(&server, &a.id).await.is_some());
}

/// `end_grace` while the stop waits for a polling loop that does not end: the wait ends at once, not at the cancel
/// timeout, and the loop is aborted.
#[tokio::test(start_paused = true)]
async fn end_grace_ends_the_wait_for_a_polling_loop_that_hangs_and_aborts_it() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    let source = Scripted::new(&server);
    let (worker, a) = worker_with_a_long_turn(&server, &source).await;
    let (alive, aborted) = tokio::sync::oneshot::channel::<()>();
    worker.replace_polling_loop(tokio::spawn(async move {
        let _alive = alive;
        std::future::pending::<()>().await
    }));
    let stop = stop_in_background(&worker, secs(30));
    sleep(secs(1)).await;
    assert_eq!(worker.state(), NodeState::Stopping, "waiting for the polling loop, whose timeout is 5 s");
    let mark = Instant::now();
    worker.end_grace();
    stop.await.unwrap();
    assert!(mark.elapsed() < secs(1), "stopped {:?} after end_grace", mark.elapsed());
    assert!(aborted.await.is_err(), "the polling loop was aborted");
    assert_eq!(Scripted::count(&source.releases), 1);
    assert!(pending(&server, &a.id).await.is_some());
}

/// Scenario: Before a stop.
#[tokio::test(start_paused = true)]
async fn end_grace_before_a_stop_is_not_remembered_and_the_later_stop_keeps_its_full_grace_period() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a"]).await;
    let a = send(&server, "a", CounterMsg::Sleep { millis: 3_000 }).await;
    let source = Scripted::new(&server);
    let worker = worker(&source, &Events::default());
    worker.end_grace(); // a new worker
    worker.start().await.unwrap();
    eventually(secs(5), || async { worker.running_turns() == 1 }).await;
    worker.end_grace(); // a running worker
    worker.stop_with_grace(secs(5)).await;
    assert_eq!(server.outcome(&a.id).await.unwrap().unwrap().status, OutcomeStatus::Done, "finished in the grace");
    assert_eq!(Scripted::count(&source.releases), 0);
}

/// type `blocker`: a handler that blocks its thread, ignoring cancellation, and fires `started` when it begins.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Blocker {
    id: String,
}

impl crate::address::ActorRef for Blocker {
    const TYPE: &'static str = "blocker";
    type Message = BlockerMsg;
}

#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "type")]
enum BlockerMsg {
    Block { millis: u64 },
}

#[derive(Default)]
struct BlockerActor {
    started: Arc<tokio::sync::Notify>,
}

impl crate::Actor for BlockerActor {
    type Ref = Blocker;
    type State = crate::NoState;

    async fn handle(&self, _turn: &mut crate::Turn<Self>, message: BlockerMsg) -> crate::HandlerResult {
        let BlockerMsg::Block { millis } = message;
        self.started.notify_one();
        std::thread::sleep(Duration::from_millis(millis));
        Ok(())
    }
}

/// Scenario: Non-yielding work and source lifetime.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_returns_while_non_yielding_work_runs_on_and_nothing_is_released_under_it() {
    let store = Store::memory();
    let server = ActorSystem::builder(store.clone()).build();
    server.register_contract(&contract_of::<BlockerActor>().unwrap()).await.unwrap();
    let blocker = Blocker { id: "b".into() };
    server.create(&blocker, &crate::NoState {}).await.unwrap();
    let m = accepted(server.send(&blocker, BlockerMsg::Block { millis: 2_000 }).await.unwrap());
    let source = Scripted::new(&server);
    let blocker_actor = BlockerActor::default();
    let started = blocker_actor.started.clone();
    let worker = Worker::builder(source.clone())
        .host(blocker_actor)
        .config(WorkerConfig {
            poll_interval: Duration::from_millis(20),
            shutdown_cancel_timeout: Duration::from_millis(200),
            ..worker_config()
        })
        .build();
    worker.start().await.unwrap();
    eventually(secs(10), || async { matches!(status(&server, &m.id).await, MessageStatus::Running { .. }) }).await;
    // Only once the handler is in its blocking section, not merely leased.
    tokio::time::timeout(secs(10), started.notified()).await.expect("the handler started");
    let mark = std::time::Instant::now();
    worker.stop_with_grace(Duration::ZERO).await;
    assert!(mark.elapsed() < Duration::from_millis(1_500), "stop returned after {:?}", mark.elapsed());
    assert_eq!(worker.state(), NodeState::Stopped);
    // The handler ends after the stop: its turn was cancelled, and is never released after its handler ended.
    sleep(Duration::from_millis(2_500)).await;
    assert_eq!(Scripted::count(&source.releases), 0);
    assert_eq!(Scripted::count(&source.completes), 0);
    assert!(matches!(status(&server, &m.id).await, MessageStatus::Running { .. }), "left to expire");
    assert_eq!(server.state(), NodeState::New, "the source is not stopped");
}

/// Scenario: Operator cancellation followed by shutdown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_completion_after_a_shutdown_stays_within_the_confirmed_lease() {
    let store = Store::memory();
    let server = ActorSystem::builder(store.clone())
        .config(NodeConfig { lease_ttl: Duration::from_millis(1_500), ..NodeConfig::default() })
        .build();
    server.register_contract(&contract_of::<BlockerActor>().unwrap()).await.unwrap();
    let blocker = Blocker { id: "b".into() };
    server.create(&blocker, &crate::NoState {}).await.unwrap();
    let m = accepted(server.send(&blocker, BlockerMsg::Block { millis: 1_200 }).await.unwrap());
    let source = Scripted::new(&server);
    source.script([Complete::Hang]);
    let blocker_actor = BlockerActor::default();
    let started = blocker_actor.started.clone();
    let worker = Worker::builder(source.clone())
        .host(blocker_actor)
        .config(WorkerConfig {
            poll_interval: Duration::from_millis(20),
            shutdown_cancel_timeout: secs(3),
            ..worker_config()
        })
        .build();
    worker.start().await.unwrap();
    eventually(secs(10), || async { matches!(status(&server, &m.id).await, MessageStatus::Running { .. }) }).await;
    // Only once the handler is in its blocking section, not merely leased.
    tokio::time::timeout(secs(10), started.notified()).await.expect("the handler started");
    let mark = std::time::Instant::now();
    assert!(server.cancel_turn(&addr(&blocker)).await.unwrap());
    // The renewal after 500 ms brings the cancel, which wins (the handler blocks until 1.2 s) and confirms the lease
    // until about 1.85 s; every renewal after it fails.
    eventually(secs(10), || async { Scripted::count(&source.renews) >= 1 }).await;
    source.set_renew(Renew::Fail);
    // The shutdown comes while the work is still stopping; the `cancelled` completion then hangs.
    worker.stop_with_grace(Duration::ZERO).await;
    let ended = mark.elapsed();
    // Cut off at the confirmed deadline by the watchdog, not at the end of the completion budget (1.2 s + 1.5 s).
    assert!(ended > Duration::from_millis(1_500), "the completion ended after {ended:?}");
    assert!(ended < Duration::from_millis(2_400), "the completion outlived the lease: {ended:?}");
    assert_eq!(source.reports(), [TurnReport::Cancelled]);
    assert_eq!(Scripted::count(&source.releases), 0, "never released as unfinished work");
    assert!(matches!(status(&server, &m.id).await, MessageStatus::Running { .. }), "left to expire");
}

/// Scenario: Non-yielding work and source lifetime.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_that_outlives_the_cancel_timeout_is_no_longer_renewed_and_its_lease_expires() {
    let store = Store::memory();
    let server = ActorSystem::builder(store.clone())
        .config(NodeConfig { lease_ttl: Duration::from_millis(600), ..NodeConfig::default() })
        .build();
    let contract = contract_of::<BlockerActor>().unwrap();
    server.register_contract(&contract).await.unwrap();
    let blocker = Blocker { id: "b".into() };
    server.create(&blocker, &crate::NoState {}).await.unwrap();
    // Blocked for more than four TTLs, ignoring cancellation.
    let m = accepted(server.send(&blocker, BlockerMsg::Block { millis: 2_500 }).await.unwrap());
    let source = Scripted::new(&server);
    let blocker_actor = BlockerActor::default();
    let started = blocker_actor.started.clone();
    let worker = Worker::builder(source.clone())
        .host(blocker_actor)
        .config(WorkerConfig {
            poll_interval: Duration::from_millis(20),
            shutdown_cancel_timeout: Duration::from_millis(200),
            ..worker_config()
        })
        .build();
    worker.start().await.unwrap();
    eventually(secs(10), || async { matches!(status(&server, &m.id).await, MessageStatus::Running { .. }) }).await;
    // Stop only once the handler is in its blocking section, not merely leased.
    tokio::time::timeout(secs(10), started.notified()).await.expect("the handler started");
    let mark = std::time::Instant::now();
    worker.stop_with_grace(Duration::ZERO).await;
    assert!(mark.elapsed() < Duration::from_millis(600), "stop returned after {:?}", mark.elapsed());
    // Give an abandonment racing the stop's own timeout a moment, then no renewal follows.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let renewals = Scripted::count(&source.renews);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(Scripted::count(&source.renews), renewals, "renewed after the cancel timeout");
    // The lease expires while the work still blocks: another caller claims the message, with one lost claim.
    let types = std::collections::BTreeMap::from([(contract.type_name().clone(), contract.hash().to_owned())]);
    let other = NodeId::new("other").unwrap();
    let lease = loop {
        match server.lease_turns(&other, &types, 1).await.unwrap() {
            LeaseTurnsResult::Leased(mut leases) if !leases.is_empty() => break leases.remove(0),
            _ => {}
        }
        assert!(mark.elapsed() < Duration::from_millis(2_000), "the lease did not expire while the work blocked");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!((lease.message.id.clone(), lease.message.lost_claims), (m.id.clone(), 1), "expired, not released");
    // Once the work ends, nothing is released or completed under the replaced lease.
    tokio::time::sleep(Duration::from_millis(2_800).saturating_sub(mark.elapsed())).await;
    assert_eq!(Scripted::count(&source.releases), 0);
    assert_eq!(Scripted::count(&source.completes), 0);
    assert!(server.outcome(&m.id).await.unwrap().is_none(), "nothing was committed under the replaced lease");
}

/// type `clock`: records the handler's clocks after a while.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Clock {
    id: String,
}

impl crate::address::ActorRef for Clock {
    const TYPE: &'static str = "clock";
    type Message = ClockMsg;
}

#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "type")]
enum ClockMsg {
    Read { millis: u64 },
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
struct ClockState {
    now: Vec<String>,
    current: Vec<String>,
}

struct ClockActor;

impl crate::Actor for ClockActor {
    type Ref = Clock;
    type State = ClockState;

    async fn handle(&self, turn: &mut crate::Turn<Self>, message: ClockMsg) -> crate::HandlerResult {
        let ClockMsg::Read { millis } = message;
        let before = turn.now();
        sleep(Duration::from_millis(millis)).await;
        let (now, current) = (turn.now(), turn.current_time());
        let state = turn.state_mut();
        state.now.extend([before, now].map(crate::time::format_instant));
        state.current.push(crate::time::format_instant(current));
        Ok(())
    }
}

/// Scenario: Handler time.
#[tokio::test(start_paused = true)]
async fn a_handlers_now_is_the_claim_and_current_time_counts_from_the_worker_lease_request() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    server.register_contract(&contract_of::<ClockActor>().unwrap()).await.unwrap();
    let clock = Clock { id: "c".into() };
    server.create(&clock, &ClockState::default()).await.unwrap();
    accepted(server.send(&clock, ClockMsg::Read { millis: 2_000 }).await.unwrap());
    let source = Scripted::new(&server);
    // The server claims at once; the answer reaches the worker 3 s after its request.
    *source.lease_delay.lock().unwrap() = secs(3);
    let worker = Worker::builder(source.clone()).host(ClockActor).config(worker_config()).build();
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 1);
    let snapshot = server.get_actor(&addr(&clock)).await.unwrap().unwrap();
    let state: ClockState = serde_json::from_value(snapshot.state).unwrap();
    let claimed_at = crate::testing::default_start();
    let format = crate::time::format_instant;
    assert_eq!(state.now, [format(claimed_at), format(claimed_at)], "now stays the claim's store time");
    let expected = crate::time::add(claimed_at, secs(5)).unwrap();
    assert_eq!(state.current, [format(expected)], "3 s of transit plus 2 s of handler, from the request");
}
