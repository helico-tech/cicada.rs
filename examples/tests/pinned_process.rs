//! Example 21: the pinned process run's protocol, its host slots and its observation pump, on virtual time with a
//! fake process backend. The real OS plumbing is checked apart, in `pinned_process_os.rs`.

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::runtime::{
    CompleteTurnResult, LeaseTurnsResult, NodeConfig, RegisterContractResult, RenewTurnResult, TurnReport, WorkerConfig,
};
use cicada::store::DeleteResult;
use cicada::store::{NodeId, OutcomeStatus, TypeDef};
use cicada::{SourceError, TurnSource, TypeName, Worker};
use cicada_examples::pinned_process::{
    ChildProcess, Delivered, Desired, IO_TIMEOUT, MAX_SLOTS, ObservationSink, Observed, ProcessBackend, ProcessHost,
    Refusal, Run, RunActor, RunMsg, RunState, create_run, observation_id, pump, reconcile, run_host,
};
use cicada_test::TestActors;
use futures::future::BoxFuture;
use tokio::sync::watch;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

const NODE: &str = "host-a";

fn node() -> NodeId {
    NodeId::new(NODE).unwrap()
}

fn run(key: &str) -> Run {
    Run { key: key.into() }
}

/// One fake child: its exit, and whether it was asked to end.
struct Ctl {
    exit: watch::Sender<Option<Option<i32>>>,
    killed: AtomicBool,
}

/// A deterministic process backend: counts spawns and kills, and can fail spawns, stall them, or ignore kills.
struct Fake {
    spawns: AtomicUsize,
    fail: AtomicBool,
    ignore_kill: AtomicBool,
    open: watch::Sender<bool>,
    children: Mutex<Vec<Arc<Ctl>>>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            spawns: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            ignore_kill: AtomicBool::new(false),
            open: watch::channel(true).0,
            children: Mutex::new(Vec::new()),
        })
    }

    fn spawns(&self) -> usize {
        self.spawns.load(Ordering::SeqCst)
    }

    fn kills(&self) -> usize {
        self.children.lock().unwrap().iter().filter(|c| c.killed.load(Ordering::SeqCst)).count()
    }

    /// Lets child `n` (in spawn order) exit with `code`.
    fn exit(&self, n: usize, code: i32) {
        self.children.lock().unwrap()[n].exit.send_replace(Some(Some(code)));
    }

    /// Stalls spawns until `stall(false)`.
    fn stall(&self, stalled: bool) {
        self.open.send_replace(!stalled);
    }
}

struct FakeChild {
    pid: u32,
    ctl: Arc<Ctl>,
    ignore_kill: bool,
}

impl ChildProcess for FakeChild {
    fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }

    fn start_kill(&mut self) -> io::Result<()> {
        self.ctl.killed.store(true, Ordering::SeqCst);
        if !self.ignore_kill {
            self.ctl.exit.send_if_modified(|exit| exit.is_none() && exit.replace(None).is_none());
        }
        Ok(())
    }

    fn wait(&mut self) -> BoxFuture<'_, io::Result<Option<i32>>> {
        let mut exit = self.ctl.exit.subscribe();
        Box::pin(async move {
            let code = *exit.wait_for(Option::is_some).await.map_err(io::Error::other)?;
            Ok(code.expect("an exit"))
        })
    }
}

impl ProcessBackend for Fake {
    fn spawn(&self) -> BoxFuture<'_, io::Result<Box<dyn ChildProcess>>> {
        let n = self.spawns.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let _ = self.open.subscribe().wait_for(|open| *open).await;
            if self.fail.load(Ordering::SeqCst) {
                return Err(io::Error::other("no such program"));
            }
            let ctl = Arc::new(Ctl { exit: watch::channel(None).0, killed: AtomicBool::new(false) });
            self.children.lock().unwrap().push(ctl.clone());
            let ignore_kill = self.ignore_kill.load(Ordering::SeqCst);
            Ok(Box::new(FakeChild { pid: 1000 + n as u32, ctl, ignore_kill }) as Box<dyn ChildProcess>)
        })
    }
}

/// Takes one from a positive `count`; false at zero.
fn take_one(count: &AtomicUsize) -> bool {
    let mut n = count.load(Ordering::SeqCst);
    while n > 0 {
        match count.compare_exchange(n, n - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(now) => n = now,
        }
    }
    false
}

fn node_config(id: &str) -> NodeConfig {
    NodeConfig { node_id: NodeId::new(id).unwrap(), ..NodeConfig::default() }
}

/// A harness whose node is `host-a`, hosting the run type for `host`.
async fn harness(host: &Arc<ProcessHost>) -> TestActors {
    let host = host.clone();
    TestActors::builder()
        .with(move |node| node.host(RunActor::new(host)).config(node_config(NODE)))
        .build()
        .await
        .unwrap()
}

async fn state(test: &TestActors, key: &str) -> RunState {
    test.state(&run(key)).await
}

async fn command(test: &TestActors, key: &str, message: RunMsg) -> RunState {
    test.send(&run(key), message).await;
    test.run_until_idle().await;
    state(test, key).await
}

/// Lets the slot tasks and the pump run, and the node handle what they sent.
async fn settle(test: &TestActors) {
    for _ in 0..5 {
        sleep(Duration::from_millis(10)).await;
        test.run_until_idle().await;
    }
}

fn start_pump(host: &Arc<ProcessHost>, sink: impl ObservationSink) -> CancellationToken {
    let shutdown = CancellationToken::new();
    tokio::spawn(pump(host.clone(), sink, shutdown.clone()));
    shutdown
}

/// A host on `host-a` over a fresh fake, its harness, and a pump into the harness's node.
async fn setup() -> (Arc<Fake>, Arc<ProcessHost>, TestActors) {
    let fake = Fake::new();
    let host = ProcessHost::new(node(), fake.clone());
    let test = harness(&host).await;
    start_pump(&host, test.system().clone());
    (fake, host, test)
}

fn running(pid: u32) -> Observed {
    Observed::Running { pid: Some(pid) }
}

// ---- Task 1: the run actor and the host protocol ----

#[tokio::test(start_paused = true)]
async fn a_start_spawns_once_and_a_repeat_refers_to_the_same_run() {
    let (fake, host, test) = setup().await;
    let created = create_run(test.system(), &host, "r1").await.unwrap();
    assert_eq!(created, run("r1"));
    let snapshot = test.system().get_actor(&run("r1").address().unwrap()).await.unwrap().unwrap();
    assert_eq!(snapshot.pinned_to, Some(node()), "created pinned to the host's node id");

    let after = command(&test, "r1", RunMsg::Start).await;
    assert_eq!((after.desired, &after.observed), (Desired::Run, &running(1000)));
    assert_eq!(after.host_epoch, host.epoch());
    test.send_with(&run("r1"), RunMsg::Start, SendOptions::new().id(MessageId::random())).await;
    settle(&test).await;
    let again = state(&test, "r1").await;
    assert_eq!((again.observed, again.revision), (after.observed, after.revision), "a repeat changes nothing");
    assert_eq!((fake.spawns(), host.slots()), (1, 1));
    assert!(test.failures().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_run_key_is_never_reused_and_must_be_plain() {
    let (_, host, test) = setup().await;
    create_run(test.system(), &host, "r1").await.unwrap();
    assert!(create_run(test.system(), &host, "r1").await.unwrap_err().contains("needs a new key"));
    for bad in ["", "a/b", "..", &"k".repeat(65)] {
        assert!(create_run(test.system(), &host, bad).await.is_err(), "{bad:?} is refused");
    }
}

#[tokio::test(start_paused = true)]
async fn a_natural_exit_is_terminal_and_nothing_restarts_it() {
    let (fake, host, test) = setup().await;
    create_run(test.system(), &host, "r1").await.unwrap();
    command(&test, "r1", RunMsg::Start).await;
    fake.exit(0, 3);
    settle(&test).await;
    assert_eq!(state(&test, "r1").await.observed, Observed::Exited { code: Some(3) });

    let refused = command(&test, "r1", RunMsg::Start).await;
    assert_eq!(refused.refusal, Some(Refusal::Ended));
    assert_eq!(refused.desired, Desired::Run, "the intent stays; the run does not restart");
    assert_eq!(fake.spawns(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_stop_before_start_reserves_a_terminal_slot_that_no_start_spawns() {
    let (fake, host, test) = setup().await;
    create_run(test.system(), &host, "r1").await.unwrap();
    let stopped = command(&test, "r1", RunMsg::Stop).await;
    assert_eq!((stopped.desired, &stopped.observed), (Desired::Stop, &Observed::StoppedBeforeStart));
    assert_eq!(host.slots(), 1, "the tombstone takes a slot");

    assert_eq!(command(&test, "r1", RunMsg::Start).await.refusal, Some(Refusal::Stopped));
    // A Start whose turn never saw the Stop commit (it was lost) finds the tombstone in the host.
    let snapshot = host.start(&run("r1")).await.unwrap();
    assert_eq!(snapshot.status, Observed::StoppedBeforeStart);
    assert_eq!(fake.spawns(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_stop_while_running_kills_once_and_waits_for_the_exit() {
    let (fake, host, test) = setup().await;
    create_run(test.system(), &host, "r1").await.unwrap();
    command(&test, "r1", RunMsg::Start).await;
    let stopped = command(&test, "r1", RunMsg::Stop).await;
    assert_eq!((stopped.desired, &stopped.observed), (Desired::Stop, &Observed::Exited { code: None }));
    command(&test, "r1", RunMsg::Stop).await;
    assert_eq!((fake.spawns(), fake.kills()), (1, 1));
}

#[tokio::test(start_paused = true)]
async fn a_stop_racing_a_stalled_spawn_is_sticky_and_a_timeout_is_unknown() {
    let (fake, host, test) = setup().await;
    create_run(test.system(), &host, "r1").await.unwrap();
    fake.stall(true);
    let starting = command(&test, "r1", RunMsg::Start).await;
    assert_eq!(starting.observed, Observed::Starting, "after {IO_TIMEOUT:?} the outcome is unknown, not refused");
    assert_eq!(command(&test, "r1", RunMsg::Start).await.observed, Observed::Starting);
    assert_eq!(fake.spawns(), 1, "a repeat during the stall spawns nothing");

    let stopping = command(&test, "r1", RunMsg::Stop).await;
    assert_eq!(stopping.observed, Observed::Stopping);
    fake.stall(false);
    settle(&test).await;
    let ended = state(&test, "r1").await;
    assert_eq!((ended.desired, ended.observed), (Desired::Stop, Observed::Exited { code: None }));
    assert_eq!((fake.spawns(), fake.kills()), (1, 1), "the child spawned late is stopped at once");
}

#[tokio::test(start_paused = true)]
async fn a_failed_spawn_is_terminal_and_not_tried_again() {
    let (fake, host, test) = setup().await;
    fake.fail.store(true, Ordering::SeqCst);
    create_run(test.system(), &host, "r1").await.unwrap();
    assert_eq!(command(&test, "r1", RunMsg::Start).await.observed, Observed::SpawnFailed);
    fake.fail.store(false, Ordering::SeqCst);
    assert_eq!(command(&test, "r1", RunMsg::Start).await.refusal, Some(Refusal::Ended));
    assert_eq!(fake.spawns(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_slots_are_bounded_and_tombstones_are_never_evicted() {
    let (fake, host, test) = setup().await;
    for n in 0..MAX_SLOTS {
        create_run(test.system(), &host, &format!("r{n}")).await.unwrap();
        command(&test, &format!("r{n}"), RunMsg::Start).await;
    }
    for n in 0..MAX_SLOTS / 2 {
        fake.exit(n, 0);
    }
    settle(&test).await;
    assert_eq!(state(&test, "r0").await.observed, Observed::Exited { code: Some(0) });

    create_run(test.system(), &host, "late").await.unwrap();
    let refused = command(&test, "late", RunMsg::Start).await;
    assert_eq!((refused.observed, refused.refusal), (Observed::Idle, Some(Refusal::Capacity)));
    create_run(test.system(), &host, "late-stop").await.unwrap();
    assert_eq!(command(&test, "late-stop", RunMsg::Stop).await.refusal, Some(Refusal::Capacity));
    assert_eq!((fake.spawns(), host.slots()), (MAX_SLOTS, MAX_SLOTS));
}

#[tokio::test(start_paused = true)]
async fn a_turn_on_another_node_id_or_epoch_acts_on_no_process() {
    let fake = Fake::new();
    // A host configured as host-b serving turns that run as host-a: a misconfigured deployment.
    let host = ProcessHost::new(NodeId::new("host-b").unwrap(), fake.clone());
    let test = harness(&host).await;
    let pin = CreateOptions::new().pinned_to(node());
    test.system().create_with(&run("wrong-node"), &RunState::new(host.epoch()), pin.clone()).await.unwrap();
    test.system().create_with(&run("old-epoch"), &RunState::new("an-old-epoch"), pin.clone()).await.unwrap();
    let ended = RunState { observed: Observed::Exited { code: Some(0) }, ..RunState::new("an-old-epoch") };
    test.system().create_with(&run("ended"), &ended, pin).await.unwrap();

    assert_eq!(command(&test, "wrong-node", RunMsg::Start).await.observed, Observed::NeedsReconciliation);
    let stopped = command(&test, "old-epoch", RunMsg::Stop).await;
    assert_eq!((stopped.desired, stopped.observed), (Desired::Stop, Observed::NeedsReconciliation));
    assert_eq!(command(&test, "ended", RunMsg::Refresh).await, ended, "a confirmed end stays terminal");
    assert_eq!((fake.spawns(), host.slots()), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn stale_and_reordered_observations_are_ignored() {
    let (_, host, test) = setup().await;
    create_run(test.system(), &host, "r1").await.unwrap();
    let started = command(&test, "r1", RunMsg::Start).await;
    assert_eq!(started.revision, 2, "Starting is revision 1, Running 2");
    let observed =
        |host_epoch: &str, revision, status| RunMsg::Observed { host_epoch: host_epoch.into(), revision, status };
    let epoch = host.epoch().to_owned();

    assert_eq!(command(&test, "r1", observed("other", 9, Observed::SpawnFailed)).await, started, "another epoch");
    assert_eq!(command(&test, "r1", observed(&epoch, 2, Observed::Stopping)).await, started, "a repeated revision");
    assert_eq!(command(&test, "r1", observed(&epoch, 1, Observed::Starting)).await, started, "an older revision");
    let exited = command(&test, "r1", observed(&epoch, 5, Observed::Exited { code: Some(1) })).await;
    assert_eq!((exited.observed.clone(), exited.revision), (Observed::Exited { code: Some(1) }, 5));
    assert_eq!(command(&test, "r1", observed(&epoch, 6, running(7))).await, exited, "Running after the end");
    assert_eq!(exited.desired, Desired::Run, "an observation never changes the intent");
}

// ---- Task 2: lost commits, delivery, deletion, restart and shutdown ----

/// A turn source in front of the harness's node that withholds completions: the first `first` ones, and every one of
/// the message `message`. A withheld completion never answers, so its lease runs out and the turn is claimed again.
struct Withhold {
    inner: ActorSystem,
    first: AtomicUsize,
    message: Mutex<Option<MessageId>>,
    withheld: AtomicUsize,
}

impl Withhold {
    fn new(inner: &ActorSystem, first: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: inner.clone(),
            first: AtomicUsize::new(first),
            message: Mutex::new(None),
            withheld: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl TurnSource for Withhold {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        TurnSource::register_contract(&self.inner, contract).await
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        TurnSource::list_types(&self.inner).await
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        TurnSource::lease_turns(&self.inner, owner, types, limit).await
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        TurnSource::renew_turn(&self.inner, actor, epoch).await
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        TurnSource::release_turn(&self.inner, actor, epoch).await
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        let first = take_one(&self.first);
        if first || self.message.lock().unwrap().as_ref() == Some(message_id) {
            self.withheld.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
        TurnSource::complete_turn(&self.inner, actor, epoch, message_id, report).await
    }
}

/// A server on `host-a`'s store that hosts nothing, and a worker with the id `host-a` running the run type for
/// `host` through `source`.
async fn worker_setup(
    max_lost_claims: Option<u32>,
    first: usize,
) -> (Arc<Fake>, Arc<ProcessHost>, TestActors, Arc<Withhold>, Worker) {
    let fake = Fake::new();
    let host = ProcessHost::new(node(), fake.clone());
    let config = NodeConfig { lease_ttl: Duration::from_secs(30), max_lost_claims, ..node_config("server") };
    let test = TestActors::builder().with(move |node| node.config(config)).build().await.unwrap();
    let source = Withhold::new(test.system(), first);
    let config = WorkerConfig { worker_id: node(), ..WorkerConfig::default() };
    let worker = Worker::builder(source.clone()).host(RunActor::new(host.clone())).config(config).build();
    worker.start().await.unwrap();
    (fake, host, test, source, worker)
}

/// Waits, on virtual time, until `message` has an outcome.
async fn outcome(test: &TestActors, message: &cicada::store::Message) -> cicada::store::OutcomeRecord {
    for _ in 0..600 {
        if let Some(outcome) = test.outcome(message).await {
            return outcome;
        }
        sleep(Duration::from_secs(1)).await;
    }
    panic!("no outcome for {}", message.id)
}

#[tokio::test(start_paused = true)]
async fn a_spawn_whose_commit_is_lost_is_found_by_the_replayed_turn() {
    let (fake, host, test, source, _worker) = worker_setup(None, 1).await;
    create_run(test.system(), &host, "r1").await.unwrap();
    let start = test.send(&run("r1"), RunMsg::Start).await;
    let done = outcome(&test, &start).await;
    assert_eq!((done.status, done.envelope.attempt), (OutcomeStatus::Done, 2), "the first commit was lost");
    assert_eq!(source.withheld.load(Ordering::SeqCst), 1);
    assert_eq!(state(&test, "r1").await.observed, running(1000));
    assert_eq!(fake.spawns(), 1, "the replay found the slot of the first attempt");
}

#[tokio::test(start_paused = true)]
async fn a_stop_whose_commit_is_lost_still_keeps_the_run_from_starting() {
    let (fake, host, test, source, _worker) = worker_setup(Some(1), 0).await;
    create_run(test.system(), &host, "r1").await.unwrap();
    let stop = test.system().send(&run("r1"), RunMsg::Stop).await.unwrap();
    let cicada::SendResult::Accepted(stop) = stop else { panic!("accepted") };
    *source.message.lock().unwrap() = Some(stop.id.clone());
    let start = test.send(&run("r1"), RunMsg::Start).await;

    assert_eq!(outcome(&test, &stop).await.status, OutcomeStatus::Failed, "no Stop commit ever applied");
    assert_eq!(outcome(&test, &start).await.status, OutcomeStatus::Done);
    let after = state(&test, "r1").await;
    assert_eq!(after.desired, Desired::Run, "the actor never saw the Stop");
    assert_eq!(after.observed, Observed::StoppedBeforeStart, "the host kept it");
    assert_eq!(fake.spawns(), 0);
}

/// A sink in front of the node: it can lose the answer of the first `lose` sends after they applied, stall sends, or
/// refuse them as down; it records the statuses that reached the node.
struct Flaky {
    inner: ActorSystem,
    lose: AtomicUsize,
    stall: AtomicBool,
    down: AtomicBool,
    applied: Mutex<Vec<(MessageId, Observed)>>,
}

/// The pump's handle on a [`Flaky`].
#[derive(Clone)]
struct FlakySink(Arc<Flaky>);

impl Flaky {
    fn new(inner: &ActorSystem) -> Arc<Self> {
        Arc::new(Self {
            inner: inner.clone(),
            lose: AtomicUsize::new(0),
            stall: AtomicBool::new(false),
            down: AtomicBool::new(false),
            applied: Mutex::new(Vec::new()),
        })
    }
}

impl ObservationSink for FlakySink {
    fn deliver<'a>(&'a self, run: &'a Run, id: MessageId, message: RunMsg) -> BoxFuture<'a, Result<Delivered, String>> {
        let this = &self.0;
        Box::pin(async move {
            if this.stall.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if this.down.load(Ordering::SeqCst) {
                return Err("the store is down".into());
            }
            let RunMsg::Observed { status, .. } = &message else { panic!("only observations") };
            let status = status.clone();
            let delivered = this.inner.deliver(run, id.clone(), message).await?;
            this.applied.lock().unwrap().push((id, status));
            if take_one(&this.lose) {
                return Err("the answer was lost".into());
            }
            Ok(delivered)
        })
    }
}

#[tokio::test(start_paused = true)]
async fn an_uncertain_or_stalled_send_is_retried_with_the_same_id() {
    let fake = Fake::new();
    let host = ProcessHost::new(node(), fake.clone());
    let test = harness(&host).await;
    let sink = Flaky::new(test.system());
    start_pump(&host, FlakySink(sink.clone()));
    create_run(test.system(), &host, "r1").await.unwrap();
    command(&test, "r1", RunMsg::Start).await;
    settle(&test).await;

    sink.lose.store(1, Ordering::SeqCst);
    fake.exit(0, 0);
    sleep(Duration::from_secs(3)).await;
    let delivery = host.delivery();
    assert_eq!((delivery.failures, delivery.last_failure.as_deref()), (1, Some("the answer was lost")));
    test.run_until_idle().await;
    assert_eq!(state(&test, "r1").await.observed, Observed::Exited { code: Some(0) });
    let exit_id = observation_id(host.epoch(), &run("r1"), 3);
    let sent = sink.applied.lock().unwrap().iter().filter(|(id, _)| *id == exit_id).count();
    assert_eq!(sent, 2, "the observation whose answer was lost was sent again under its id");
    assert!(test.failures().await.is_empty(), "the repeat was a duplicate, not a second message");

    sink.stall.store(true, Ordering::SeqCst);
    create_run(test.system(), &host, "r2").await.unwrap();
    command(&test, "r2", RunMsg::Start).await;
    sleep(IO_TIMEOUT + Duration::from_millis(100)).await;
    assert_eq!(host.delivery().failures, 2, "a stalled send is cut off after {IO_TIMEOUT:?}");
    sink.stall.store(false, Ordering::SeqCst);
    sleep(Duration::from_secs(2)).await;
    let r2 = run("r2");
    assert!(sink.applied.lock().unwrap().iter().any(|(id, _)| *id == observation_id(host.epoch(), &r2, 2)));
    assert!(host.shutdown(IO_TIMEOUT).await.is_clean());
}

#[tokio::test(start_paused = true)]
async fn the_latest_observation_replaces_one_not_yet_sent() {
    let fake = Fake::new();
    let host = ProcessHost::new(node(), fake.clone());
    let test = harness(&host).await;
    let sink = Flaky::new(test.system());
    sink.down.store(true, Ordering::SeqCst);
    start_pump(&host, FlakySink(sink.clone()));
    create_run(test.system(), &host, "r1").await.unwrap();
    fake.stall(true);
    assert_eq!(command(&test, "r1", RunMsg::Start).await.observed, Observed::Starting);
    fake.stall(false);
    sleep(Duration::from_millis(10)).await;
    fake.exit(0, 4);
    sleep(Duration::from_secs(3)).await;
    assert!(host.delivery().failures >= 2);
    sink.down.store(false, Ordering::SeqCst);
    settle(&test).await;
    sleep(Duration::from_secs(1)).await;
    test.run_until_idle().await;

    let applied: Vec<Observed> = sink.applied.lock().unwrap().iter().map(|(_, s)| s.clone()).collect();
    assert_eq!(applied, [Observed::Exited { code: Some(4) }], "Running was never queued behind it");
    assert_eq!(state(&test, "r1").await.observed, Observed::Exited { code: Some(4) });
}

#[tokio::test(start_paused = true)]
async fn deleting_a_run_does_not_stop_its_child_and_the_host_keeps_the_evidence() {
    let (fake, host, test) = setup().await;
    create_run(test.system(), &host, "r1").await.unwrap();
    command(&test, "r1", RunMsg::Start).await;
    let deleted = test.system().delete_actor(&run("r1").address().unwrap()).await.unwrap();
    assert!(matches!(deleted, DeleteResult::Deleted { .. }));
    settle(&test).await;
    assert_eq!(host.snapshot(&run("r1")).unwrap().status, running(1000), "still owned by the host");
    assert_eq!(fake.kills(), 0);

    fake.exit(0, 0);
    settle(&test).await;
    assert_eq!(host.snapshot(&run("r1")).unwrap().status, Observed::Exited { code: Some(0) });
    assert!(host.delivery().absent >= 1, "the pump stopped sending to the absent actor");
    assert!(host.shutdown(IO_TIMEOUT).await.is_clean());
}

async fn run_all(system: &ActorSystem) {
    while system.run_due_turns(10).await.unwrap() > 0 {}
}

#[tokio::test(start_paused = true)]
async fn a_new_host_epoch_marks_old_runs_for_reconciliation_and_acts_on_none() {
    let (fake, host, test) = setup().await;
    for key in ["live", "idle", "ended"] {
        create_run(test.system(), &host, key).await.unwrap();
    }
    command(&test, "live", RunMsg::Start).await;
    command(&test, "ended", RunMsg::Start).await;
    fake.exit(1, 0);
    settle(&test).await;
    let ended = state(&test, "ended").await;
    assert_eq!(ended.observed, Observed::Exited { code: Some(0) });

    // The host process restarts with the same node id: a new epoch, a new backend, the same store.
    let fake2 = Fake::new();
    let host2 = ProcessHost::new(node(), fake2.clone());
    let system2 =
        ActorSystem::builder(test.store().clone()).config(node_config(NODE)).host(RunActor::new(host2.clone())).build();
    system2.initialize().await.unwrap();
    assert_ne!(host2.epoch(), host.epoch());
    assert_eq!(reconcile(&system2, &node()).await.unwrap(), 3);
    run_all(&system2).await;
    assert_eq!(state(&test, "live").await.observed, Observed::NeedsReconciliation);
    assert_eq!(state(&test, "idle").await.observed, Observed::NeedsReconciliation, "even a run never started");
    assert_eq!(state(&test, "ended").await, ended, "a confirmed end stays terminal");

    for key in ["live", "idle"] {
        for message in [RunMsg::Start, RunMsg::Stop, RunMsg::Refresh] {
            system2.send(&run(key), message).await.unwrap();
        }
    }
    run_all(&system2).await;
    assert_eq!((fake2.spawns(), fake2.kills(), host2.slots()), (0, 0, 0), "no spawn, kill or adoption");
    assert_eq!(fake.kills(), 0, "the old child was not signalled");

    // The original host is still alive (two hosts shared the id): its late exit report does not clear the mark.
    fake.exit(0, 0);
    settle(&test).await;
    let live = state(&test, "live").await;
    assert_eq!((live.observed, live.revision), (Observed::NeedsReconciliation, 2));

    // The operator checked the old process outside Cicada; deleting the actor proves nothing, a replacement is new.
    let deleted = system2.delete_actor(&run("live").address().unwrap()).await.unwrap();
    assert!(matches!(deleted, DeleteResult::Deleted { .. }));
    assert!(!test.exists(&run("live")).await, "nothing replaces it on its own");
    create_run(&system2, &host2, "live-2").await.unwrap();
    system2.send(&run("live-2"), RunMsg::Start).await.unwrap();
    run_all(&system2).await;
    assert_eq!(state(&test, "live-2").await.observed, running(1000));
    assert_eq!(fake2.spawns(), 1);
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_reaps_and_flushes_before_the_system_stops() {
    let (fake, host, test) = setup().await;
    for key in ["a", "b"] {
        create_run(test.system(), &host, key).await.unwrap();
        command(&test, key, RunMsg::Start).await;
    }
    let report = host.shutdown(IO_TIMEOUT).await;
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(fake.kills(), 2);
    test.run_until_idle().await;
    assert_eq!(state(&test, "a").await.observed, Observed::Exited { code: None }, "flushed to the actor");

    create_run(test.system(), &host, "c").await.unwrap();
    assert_eq!(command(&test, "c", RunMsg::Start).await.refusal, Some(Refusal::HostClosed));
    assert_eq!(fake.spawns(), 2);
}

#[tokio::test(start_paused = true)]
async fn shutdown_reports_an_exit_it_cannot_confirm_and_an_observation_it_cannot_flush() {
    let fake = Fake::new();
    fake.ignore_kill.store(true, Ordering::SeqCst);
    let host = ProcessHost::new(node(), fake.clone());
    let test = harness(&host).await;
    let sink = Flaky::new(test.system());
    start_pump(&host, FlakySink(sink.clone()));
    create_run(test.system(), &host, "stuck").await.unwrap();
    command(&test, "stuck", RunMsg::Start).await;
    settle(&test).await;
    sink.down.store(true, Ordering::SeqCst);

    let report = host.shutdown(IO_TIMEOUT).await;
    assert_eq!(report.unconfirmed, ["stuck"], "a kill request is not an exit");
    assert_eq!(report.unflushed, ["stuck"], "its Stopping never reached the actor");
    assert_eq!(fake.kills(), 1);
    assert_eq!(host.snapshot(&run("stuck")).unwrap().status, Observed::Stopping, "never claimed Stopped");
}

#[tokio::test(start_paused = true)]
async fn a_start_on_an_old_epoch_marks_a_stopping_run_for_reconciliation_before_any_refusal() {
    let fake = Fake::new();
    let host = ProcessHost::new(node(), fake.clone());
    let test = harness(&host).await;
    let pin = CreateOptions::new().pinned_to(node());
    // A stop the old host never confirmed before it died.
    let stopping =
        RunState { desired: Desired::Stop, observed: Observed::Stopping, revision: 3, ..RunState::new("an-old-epoch") };
    test.system().create_with(&run("stopping"), &stopping, pin.clone()).await.unwrap();
    let running =
        RunState { desired: Desired::Run, observed: running(42), revision: 2, ..RunState::new("an-old-epoch") };
    test.system().create_with(&run("running"), &running, pin).await.unwrap();

    let after = command(&test, "stopping", RunMsg::Start).await;
    assert_eq!((after.desired, after.observed, after.refusal), (Desired::Stop, Observed::NeedsReconciliation, None));
    assert_eq!(command(&test, "running", RunMsg::Start).await.observed, Observed::NeedsReconciliation);
    assert_eq!(command(&test, "stopping", RunMsg::Start).await.refusal, Some(Refusal::Stopped), "then refused");
    assert_eq!((fake.spawns(), fake.kills(), host.slots()), (0, 0, 0), "no process action");
}

#[tokio::test(start_paused = true)]
async fn an_error_while_a_child_runs_still_cleans_up_in_order_and_is_returned() {
    let fake = Fake::new();
    let host = ProcessHost::new(node(), fake.clone());
    let test = harness(&host).await;
    let body = async {
        create_run(test.system(), &host, "r1").await?;
        command(&test, "r1", RunMsg::Start).await;
        assert_eq!(host.snapshot(&run("r1")).unwrap().status, running(1000), "a live child");
        // As when listing the runs fails while a child runs.
        Err::<(), String>("listing the runs failed".into())
    };
    let (result, report) = run_host(test.store(), test.system(), &host, body).await;
    assert_eq!(result, Err("listing the runs failed".into()), "the original error is kept");
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(fake.kills(), 1);
    assert_eq!(host.snapshot(&run("r1")).unwrap().status, Observed::Exited { code: None }, "reaped");
    assert_eq!(host.delivery().failures, 0, "flushed before the system and the store stopped");
    assert_eq!(host.start(&run("r2")).await, Err(Refusal::HostClosed));
}
