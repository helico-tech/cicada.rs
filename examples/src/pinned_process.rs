//! Example 21 — a pinned process run: durable intent in an actor, a live child process in the host that runs it.
//!
//! A child-process handle cannot live in actor state, and spawning or stopping a process is an outside effect that
//! can happen before the turn that asked for it commits. Here a [`Run`] actor keeps what was asked ([`Desired`]) and
//! what was seen ([`Observed`]); a [`ProcessHost`] in the same process keeps the handles. The actor is created pinned
//! to the host's stable node id ([`create_run`]), so its turns run where its process lives. The protocol is this
//! example's application convention (ADR 0053), not a Cicada guarantee:
//!
//! - **One key, one run.** A fresh key that is never used again names one run of the host's one fixed child program.
//!   Nothing restarts it: a natural exit, a stop and a failed spawn are terminal, and a further `Start` is refused. A
//!   new run needs a new key, created explicitly. No supervisor, flow or event creates or restarts runs.
//! - **Node id and host epoch.** The node id is stable configuration: it pins the actor. The host epoch is new at
//!   every start of a host process and never reused; the actor stores the epoch of the host that created it. A turn
//!   that runs on another node id, or whose epoch is not the host's, does nothing to a process: a run that was not
//!   terminal becomes [`Observed::NeedsReconciliation`], and stays so. A PID is shown for diagnosis only; it is never
//!   adopted or signalled. Unique node ids are the deployment's duty: a second live host with the same id may mark a
//!   live run uncertain, but it cannot act on it.
//! - **Slots keep the evidence.** The host keeps at most [`MAX_SLOTS`] slots, one per run key, for its whole life: a
//!   slot is reserved before anything is spawned, starts its child at most once, and is never evicted or reused, so
//!   a turn replayed after a lost commit finds the effect of the first one instead of repeating it. `Stop` before
//!   `Start` reserves a terminal slot too, so stop intent survives a lost `Stop` commit; it uses one of the slots.
//!   When the slots run out, a new run is refused with nothing spawned.
//! - **Observations are versioned.** Each slot numbers its changes. A pump ([`pump`]) sends the latest one of each
//!   slot as an ordinary [`RunMsg::Observed`] with a stable message id per run and revision, retrying an uncertain
//!   send. The actor applies only the current epoch's newer revisions, and never after a terminal or uncertain
//!   status; an observation never changes the desired intent. Messages are trusted application input: nothing here
//!   authenticates their sender.
//! - **Deleting is not cleanup.** Deleting a run's actor leaves its child running; the host still owns it until a
//!   `Stop` or shutdown. Stop a run and see it terminal before deleting it. A run that needs reconciliation can only
//!   be deleted once its old process was checked or cleaned up outside Cicada; deleting it proves nothing.
//! - **Host lifetime only.** All of this holds while one host process lives. A host that dies abruptly may leave an
//!   orphan child (kill-on-drop is best effort), and its successor cannot know what happened: it marks the old runs
//!   for reconciliation ([`reconcile`]). No exactly-once spawn is promised.
//!
//! [`OsBackend`] runs a real child, the example executable in its inert child mode ([`child_main`]); the tests use a
//! fake backend. The runnable host is `examples/examples/pinned_process.rs`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use cicada::prelude::*;
use cicada::store::NodeId;
use cicada::{CreateActorResult, OpError, SendResult, Store};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Notify, watch};
use tokio::time::{Instant, sleep, timeout, timeout_at};
use tokio_util::sync::CancellationToken;

/// The most slots, live or terminal, one host keeps in its life.
pub const MAX_SLOTS: usize = 32;

/// The longest key of a run.
pub const MAX_KEY_CHARS: usize = 64;

/// The bound of a wait for a command's result, of one observation send and of the shutdown cleanup.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the pump waits before it sends an uncertain observation again.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// One run of the host's fixed child program; its key is never used again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Run {
    /// The run key: 1 to [`MAX_KEY_CHARS`] ASCII letters, digits, `.`, `_` or `-`.
    pub key: String,
}

impl ActorRef for Run {
    const TYPE: &'static str = "process.runs";
    type Message = RunMsg;
}

/// Messages of [`Run`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum RunMsg {
    /// Starts the run once; a repeat, whatever its message id, refers to the same run.
    Start,
    /// Records the stop intent and stops the run's child, or keeps it from ever starting.
    Stop,
    /// Reads the host's slot again; after a host restart it marks a run that was not terminal for reconciliation.
    Refresh,
    /// The host saw `status` as change `revision` of this run's slot; sent by the pump.
    Observed {
        /// The epoch of the host that saw it.
        host_epoch: String,
        /// The slot's change number, from 1.
        revision: u64,
        /// What it saw.
        status: Observed,
    },
}

/// What was asked of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Desired {
    /// Nothing yet.
    #[default]
    Idle,
    /// Started.
    Run,
    /// Stopped, also before a start.
    Stop,
}

/// What was seen of a run's process.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status")]
pub enum Observed {
    /// Never started.
    #[default]
    Idle,
    /// The slot is reserved; the child is being spawned, or the outcome is not known yet.
    Starting,
    /// The child runs.
    Running {
        /// Its process id: for diagnosis only, never used to signal or adopt a process.
        pid: Option<u32>,
    },
    /// A stop was requested; exit is not confirmed yet.
    Stopping,
    /// The child exited and was reaped. Terminal.
    Exited {
        /// Its exit code; `None` when a signal ended it.
        code: Option<i32>,
    },
    /// The spawn failed; it is not tried again. Terminal.
    SpawnFailed,
    /// Stopped before a child was spawned; none will be. Terminal.
    StoppedBeforeStart,
    /// A host that did not create this run, or a host process started since, handled it: what happened to its process
    /// is unknown here. Absorbing: check or clean up outside Cicada, then delete the actor.
    NeedsReconciliation,
}

impl Observed {
    /// True for a status a host confirmed as the end of the run: exited, spawn failed, stopped before start.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Exited { .. } | Self::SpawnFailed | Self::StoppedBeforeStart)
    }

    /// True when nothing changes this status any more: terminal, or needing reconciliation.
    pub fn is_final(&self) -> bool {
        self.is_terminal() || *self == Self::NeedsReconciliation
    }
}

/// Why a command did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Refusal {
    /// `Start` after `Stop`.
    Stopped,
    /// `Start` after the run ended or was marked for reconciliation; a new run needs a new key.
    Ended,
    /// The host's [`MAX_SLOTS`] slots are taken; nothing was spawned.
    Capacity,
    /// The host is shutting down and takes no new run.
    HostClosed,
}

/// State of a [`Run`]. It holds no handle: the host keeps those.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunState {
    /// The epoch of the host that created the run; only that host acts on it.
    pub host_epoch: String,
    /// What was asked.
    pub desired: Desired,
    /// What was seen.
    pub observed: Observed,
    /// The slot revision `observed` came from; 0 before the first.
    pub revision: u64,
    /// The last command that did nothing, and why.
    pub refusal: Option<Refusal>,
}

impl RunState {
    /// The initial state of a run created by the host of `host_epoch`.
    pub fn new(host_epoch: impl Into<String>) -> Self {
        Self {
            host_epoch: host_epoch.into(),
            desired: Desired::Idle,
            observed: Observed::Idle,
            revision: 0,
            refusal: None,
        }
    }

    /// Takes `snapshot` when it is this run's host epoch, newer, and the status can still change.
    pub fn apply(&mut self, snapshot: &Snapshot) {
        if !self.observed.is_final() && snapshot.host_epoch == self.host_epoch && snapshot.revision > self.revision {
            self.observed = snapshot.status.clone();
            self.revision = snapshot.revision;
        }
    }
}

/// A slot's status as one change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The host's epoch.
    pub host_epoch: String,
    /// The slot's change number, from 1.
    pub revision: u64,
    /// What the host saw.
    pub status: Observed,
}

/// A spawned child, owned by one slot's task.
pub trait ChildProcess: Send + 'static {
    /// Its process id, when it has one; for diagnosis only.
    fn pid(&self) -> Option<u32>;

    /// Asks the child to end. A request is not an exit: wait for it.
    fn start_kill(&mut self) -> io::Result<()>;

    /// Waits for the child to exit, reaps it and returns its exit code (`None` when a signal ended it). Cancel-safe.
    fn wait(&mut self) -> BoxFuture<'_, io::Result<Option<i32>>>;
}

/// Starts the host's one fixed child program, chosen when the host is built, never by a message.
pub trait ProcessBackend: Send + Sync + 'static {
    /// Spawns one child.
    fn spawn(&self) -> BoxFuture<'_, io::Result<Box<dyn ChildProcess>>>;
}

struct Slot {
    run: Run,
    revision: u64,
    status: Observed,
    /// Sticky: once set, the slot never spawns, and a spawned child is stopped.
    stop: bool,
    /// The highest revision the pump delivered or gave up on (the actor is gone).
    delivered: u64,
    changes: watch::Sender<Snapshot>,
    stop_signal: Arc<Notify>,
}

impl Slot {
    fn new(run: &Run, epoch: &str, status: Observed, stop: bool) -> Self {
        let snapshot = Snapshot { host_epoch: epoch.to_owned(), revision: 1, status: status.clone() };
        let (changes, _) = watch::channel(snapshot);
        Self {
            run: run.clone(),
            revision: 1,
            status,
            stop,
            delivered: 0,
            changes,
            stop_signal: Arc::new(Notify::new()),
        }
    }

    fn set(&mut self, status: Observed) {
        self.revision += 1;
        self.status = status;
        let revision = self.revision;
        let status = self.status.clone();
        self.changes.send_modify(|s| {
            s.revision = revision;
            s.status = status;
        });
    }

    /// Records the stop intent; the signal for a slot whose task must stop its child.
    fn request_stop(&mut self) -> Option<Arc<Notify>> {
        if self.stop {
            return None;
        }
        self.stop = true;
        match self.status {
            Observed::Starting | Observed::Running { .. } => {
                self.set(Observed::Stopping);
                Some(self.stop_signal.clone())
            }
            _ => None,
        }
    }
}

/// How the pump's sends went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeliveryStats {
    /// Observations accepted by the store (or found there already).
    pub accepted: u64,
    /// Observations not sent because the run's actor does not exist; the slot keeps them.
    pub absent: u64,
    /// Failed or timed-out sends, each retried.
    pub failures: u64,
    /// The last failure.
    pub last_failure: Option<String>,
}

struct Table {
    open: bool,
    slots: BTreeMap<String, Slot>,
    delivery: DeliveryStats,
}

/// What a bounded shutdown could not confirm.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    /// The run keys whose child was not confirmed exited in time.
    pub unconfirmed: Vec<String>,
    /// The run keys whose latest observation was not delivered in time.
    pub unflushed: Vec<String>,
    /// Why the pump's task did not end cleanly, when [`run_host`] joined it.
    pub pump: Option<String>,
}

impl CleanupReport {
    /// True when every child was confirmed exited and every observation delivered.
    pub fn is_clean(&self) -> bool {
        self.unconfirmed.is_empty() && self.unflushed.is_empty() && self.pump.is_none()
    }
}

/// The live side of runs on one node: the slot table, its child tasks, and the epoch of this host process.
///
/// Build it before the [`ActorSystem`] and give it to [`RunActor`]; attach the [`pump`] once the system is built.
pub struct ProcessHost {
    node_id: NodeId,
    epoch: String,
    backend: Arc<dyn ProcessBackend>,
    table: Mutex<Table>,
    /// Wakes the pump.
    changed: Notify,
}

impl ProcessHost {
    /// A host for the node `node_id` with a fresh epoch, spawning children through `backend`.
    pub fn new(node_id: NodeId, backend: Arc<dyn ProcessBackend>) -> Arc<Self> {
        Arc::new(Self {
            node_id,
            epoch: MessageId::random().to_string(),
            backend,
            table: Mutex::new(Table { open: true, slots: BTreeMap::new(), delivery: DeliveryStats::default() }),
            changed: Notify::new(),
        })
    }

    /// The stable node id the host's runs are pinned to.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// This host process's epoch, never reused.
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().expect("the slot table is never poisoned")
    }

    /// The slot of `run`, when it has one.
    pub fn snapshot(&self, run: &Run) -> Option<Snapshot> {
        self.table().slots.get(&run.key).map(|slot| slot.changes.borrow().clone())
    }

    /// How many slots the host keeps.
    pub fn slots(&self) -> usize {
        self.table().slots.len()
    }

    /// How the pump's sends went.
    pub fn delivery(&self) -> DeliveryStats {
        self.table().delivery.clone()
    }

    /// Starts `run` once: reserves its slot and hands it to a task that spawns the child, then waits up to
    /// [`IO_TIMEOUT`] for the outcome. A repeat returns the slot as it is, spawning nothing. `Starting` after the wait
    /// means unknown, never "not started". Cancelling the caller cancels neither the task nor the reservation.
    pub async fn start(self: &Arc<Self>, run: &Run) -> Result<Snapshot, Refusal> {
        let changes = {
            let mut table = self.table();
            match table.slots.get(&run.key) {
                Some(slot) => slot.changes.subscribe(),
                None => {
                    let slot = self.reserve(&mut table, run, Observed::Starting, false)?;
                    let changes = slot.changes.subscribe();
                    tokio::spawn(self.clone().own(run.key.clone()));
                    changes
                }
            }
        };
        self.changed.notify_one();
        Ok(settle(changes, |status| *status != Observed::Starting).await)
    }

    /// Stops `run`: records the sticky stop intent, signals its child's task, and waits up to [`IO_TIMEOUT`] for a
    /// confirmed exit. A run without a slot gets a terminal one, so that no later `Start` spawns it.
    pub async fn stop(&self, run: &Run) -> Result<Snapshot, Refusal> {
        let (changes, signal) = {
            let mut table = self.table();
            match table.slots.get_mut(&run.key) {
                Some(slot) => (slot.changes.subscribe(), slot.request_stop()),
                None => (self.reserve(&mut table, run, Observed::StoppedBeforeStart, true)?.changes.subscribe(), None),
            }
        };
        if let Some(signal) = signal {
            signal.notify_one();
        }
        self.changed.notify_one();
        Ok(settle(changes, Observed::is_terminal).await)
    }

    fn reserve<'t>(&self, table: &'t mut Table, run: &Run, status: Observed, stop: bool) -> Result<&'t Slot, Refusal> {
        if !table.open {
            return Err(Refusal::HostClosed);
        }
        if table.slots.len() >= MAX_SLOTS {
            return Err(Refusal::Capacity);
        }
        Ok(table.slots.entry(run.key.clone()).or_insert(Slot::new(run, &self.epoch, status, stop)))
    }

    /// Changes the slot of `key` with `change`, and wakes the pump.
    fn update<T>(&self, key: &str, change: impl FnOnce(&mut Slot) -> T) -> T {
        let result = change(self.table().slots.get_mut(key).expect("a slot is never removed"));
        self.changed.notify_one();
        result
    }

    /// The task that owns the slot of `key`: spawns its child at most once, waits for its exit, and stops and reaps
    /// it when asked.
    async fn own(self: Arc<Self>, key: String) {
        let stopped = self.update(&key, |slot| {
            if slot.stop {
                slot.set(Observed::StoppedBeforeStart);
            }
            slot.stop
        });
        if stopped {
            return;
        }
        let mut child = match self.backend.spawn().await {
            Ok(child) => child,
            Err(_) => return self.update(&key, |slot| slot.set(Observed::SpawnFailed)),
        };
        let pid = child.pid();
        let (stop, signal) = self.update(&key, |slot| {
            if !slot.stop {
                slot.set(Observed::Running { pid });
            }
            (slot.stop, slot.stop_signal.clone())
        });
        if !stop {
            tokio::select! {
                exit = child.wait() => {
                    // An exit that cannot be read stays unconfirmed: the slot keeps its status.
                    if let Ok(code) = exit {
                        self.update(&key, |slot| slot.set(Observed::Exited { code }));
                    }
                    return;
                }
                () = signal.notified() => {}
            }
        }
        // A failed request (the child exited already, say) still ends in the wait that confirms the exit.
        let _ = child.start_kill();
        if let Ok(code) = child.wait().await {
            self.update(&key, |slot| slot.set(Observed::Exited { code }));
        }
    }

    /// The latest undelivered observation of each slot.
    fn undelivered(&self) -> Vec<(Run, Snapshot)> {
        let table = self.table();
        let pending = table.slots.values().filter(|slot| slot.revision > slot.delivered);
        pending.map(|slot| (slot.run.clone(), slot.changes.borrow().clone())).collect()
    }

    fn delivered(&self, run: &Run, revision: u64, outcome: Result<Delivered, String>) {
        let mut table = self.table();
        let delivery = &mut table.delivery;
        match outcome {
            Ok(Delivered::Accepted) => delivery.accepted += 1,
            Ok(Delivered::Absent) => delivery.absent += 1,
            Err(e) => {
                delivery.failures += 1;
                delivery.last_failure = Some(e);
                return;
            }
        }
        if let Some(slot) = table.slots.get_mut(&run.key) {
            slot.delivered = slot.delivered.max(revision);
        }
    }

    /// Bounded shutdown, with the actor system, the pump and the store still open: closes admission, stops every
    /// child, and waits for each exit and for the pump to deliver every latest observation, all within `budget`.
    /// What it could not confirm is in the report.
    pub async fn shutdown(&self, budget: Duration) -> CleanupReport {
        let deadline = Instant::now() + budget;
        let (changes, signals): (Vec<_>, Vec<_>) = {
            let mut table = self.table();
            table.open = false;
            table.slots.values_mut().map(|slot| (slot.changes.subscribe(), slot.request_stop())).unzip()
        };
        signals.into_iter().flatten().for_each(|signal| signal.notify_one());
        self.changed.notify_one();
        for mut changes in changes {
            let _ = timeout_at(deadline, changes.wait_for(|s| s.status.is_terminal())).await;
        }
        while !self.undelivered().is_empty() && Instant::now() < deadline {
            sleep(Duration::from_millis(50)).await;
        }
        let table = self.table();
        let keys = |keep: fn(&Slot) -> bool| -> Vec<String> {
            table.slots.iter().filter(|(_, s)| keep(s)).map(|(k, _)| k.clone()).collect()
        };
        CleanupReport {
            unconfirmed: keys(|s| !s.status.is_terminal()),
            unflushed: keys(|s| s.revision > s.delivered),
            pump: None,
        }
    }
}

/// The slot's snapshot once `done` holds for its status, or as it is after [`IO_TIMEOUT`].
async fn settle(mut changes: watch::Receiver<Snapshot>, done: impl Fn(&Observed) -> bool) -> Snapshot {
    if let Ok(Ok(snapshot)) = timeout(IO_TIMEOUT, changes.wait_for(|s| done(&s.status))).await {
        return snapshot.clone();
    }
    changes.borrow().clone()
}

/// The run type, with the host that owns its processes.
pub struct RunActor {
    host: Arc<ProcessHost>,
}

impl RunActor {
    /// The run type for `host`.
    pub fn new(host: Arc<ProcessHost>) -> Self {
        Self { host }
    }
}

impl Actor for RunActor {
    type Ref = Run;
    type State = RunState;

    async fn handle(&self, turn: &mut Turn<Self>, message: RunMsg) -> HandlerResult {
        let state = turn.state_mut();
        match message {
            RunMsg::Observed { host_epoch, revision, status } => {
                state.apply(&Snapshot { host_epoch, revision, status });
                return Ok(());
            }
            RunMsg::Stop => state.desired = Desired::Stop,
            RunMsg::Start | RunMsg::Refresh => {}
        }
        // A confirmed end stays terminal, and a run needing reconciliation stays so, whatever handles it.
        if state.observed.is_final() {
            if message == RunMsg::Start {
                state.refusal = Some(if state.desired == Desired::Stop { Refusal::Stopped } else { Refusal::Ended });
            }
            return Ok(());
        }
        // Another node, or another host process: what happened to the process is unknown here. No OS action. This
        // comes before any refusal, so that no command leaves an old run looking settled.
        let mismatch = state.host_epoch != self.host.epoch();
        if mismatch || turn.node_id() != self.host.node_id() {
            turn.state_mut().observed = Observed::NeedsReconciliation;
            return Ok(());
        }
        let state = turn.state_mut();
        if message == RunMsg::Start {
            if state.desired == Desired::Stop {
                state.refusal = Some(Refusal::Stopped);
                return Ok(());
            }
            state.desired = Desired::Run;
        }
        let run = turn.self_ref().clone();
        let result = match message {
            RunMsg::Start => self.host.start(&run).await,
            RunMsg::Stop => self.host.stop(&run).await,
            _ => match self.host.snapshot(&run) {
                Some(snapshot) => Ok(snapshot),
                None => return Ok(()),
            },
        };
        let state = turn.state_mut();
        match result {
            Ok(snapshot) => state.apply(&snapshot),
            Err(refusal) => state.refusal = Some(refusal),
        }
        Ok(())
    }
}

/// Creates a new run pinned to `host`'s node, in the state `host` acts on, and checks the stored pin. A key that
/// exists is refused: a run key is never reused, not even after its actor was deleted (that is the caller's duty).
pub async fn create_run(system: &ActorSystem, host: &ProcessHost, key: &str) -> Result<Run, String> {
    let valid = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    if key.is_empty() || key.chars().count() > MAX_KEY_CHARS || !key.chars().all(valid) || key == "." || key == ".." {
        return Err(format!("Invalid run key '{key}'"));
    }
    let run = Run { key: key.to_owned() };
    let options = CreateOptions::new().pinned_to(host.node_id().clone());
    match system.create_with(&run, &RunState::new(host.epoch()), options).await.map_err(|e| e.to_string())? {
        CreateActorResult::Created => {}
        CreateActorResult::Exists(_) => return Err(format!("The run '{key}' exists; a new run needs a new key")),
        other => return Err(format!("The run '{key}' was not created: {other:?}")),
    }
    let address = run.address().map_err(|e| e.to_string())?;
    let snapshot = system.get_actor(&address).await.map_err(|e| e.to_string())?;
    match snapshot.and_then(|s| s.pinned_to) {
        Some(pin) if pin == *host.node_id() => Ok(run),
        pin => Err(format!("The run '{key}' is pinned to {pin:?}, not {}", host.node_id())),
    }
}

/// Sends `Refresh` to every run pinned to `node`, paging through the type; returns how many. Run at host startup: a
/// run of an earlier host process that was not terminal shows [`Observed::NeedsReconciliation`] once refreshed.
pub async fn reconcile(system: &ActorSystem, node: &NodeId) -> Result<usize, OpError> {
    let prefix = format!("{}/", Run::TYPE);
    let (mut cursor, mut sent) = (None, 0);
    loop {
        let page = system.list_actors_with_state(&prefix, cursor.as_ref(), 100).await?;
        for snapshot in page.items.into_iter().filter(|s| s.pinned_to.as_ref() == Some(node)) {
            let run = Run { key: snapshot.address.name() };
            // A run deleted since the listing has nothing to refresh.
            if let SendResult::Accepted(_) = system.send(&run, RunMsg::Refresh).await? {
                sent += 1;
            }
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(sent),
        }
    }
}

/// How a send of an observation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivered {
    /// The store holds the message (accepted now or before).
    Accepted,
    /// The run's actor does not exist.
    Absent,
}

/// Where the pump sends observations. [`ActorSystem`] sends them as ordinary messages; a test wraps it to fail or
/// stall sends.
pub trait ObservationSink: Send + Sync + 'static {
    /// Sends `message` to `run` with the message id `id`. `Err` means the outcome is unknown.
    fn deliver<'a>(&'a self, run: &'a Run, id: MessageId, message: RunMsg) -> BoxFuture<'a, Result<Delivered, String>>;
}

impl ObservationSink for ActorSystem {
    fn deliver<'a>(&'a self, run: &'a Run, id: MessageId, message: RunMsg) -> BoxFuture<'a, Result<Delivered, String>> {
        Box::pin(async move {
            match self.send_with(run, message, SendOptions::new().id(id)).await.map_err(|e| e.to_string())? {
                SendResult::Accepted(_) | SendResult::Duplicate(_) => Ok(Delivered::Accepted),
                SendResult::NoSuchInstance => Ok(Delivered::Absent),
                refused => Err(format!("The observation was refused: {refused:?}")),
            }
        })
    }
}

/// The message id of the observation `revision` of `run` under `epoch`: the same for every retry.
pub fn observation_id(epoch: &str, run: &Run, revision: u64) -> MessageId {
    MessageId::new(format!("obs-{epoch}-{}-{revision}", run.key)).expect("an observation id is not blank")
}

/// Sends the latest observation of each slot of `host` to its run until `shutdown`: at once on a change, and every
/// [`RETRY_INTERVAL`] while a send failed or did not end within [`IO_TIMEOUT`], with the same id. A slot keeps only
/// its latest status, so a terminal one replaces a `Running` not yet sent. Failures are counted in
/// [`ProcessHost::delivery`]. Start it after the system is built; stop it after the system stopped.
pub async fn pump(host: Arc<ProcessHost>, sink: impl ObservationSink, shutdown: CancellationToken) {
    loop {
        let mut failed = false;
        for (run, snapshot) in host.undelivered() {
            let id = observation_id(&host.epoch, &run, snapshot.revision);
            let revision = snapshot.revision;
            let message = RunMsg::Observed { host_epoch: snapshot.host_epoch, revision, status: snapshot.status };
            let outcome = tokio::select! {
                outcome = timeout(IO_TIMEOUT, sink.deliver(&run, id, message)) => outcome,
                () = shutdown.cancelled() => return,
            };
            let outcome = outcome.unwrap_or_else(|_| Err(format!("The send did not end within {IO_TIMEOUT:?}")));
            failed |= outcome.is_err();
            host.delivered(&run, revision, outcome);
        }
        tokio::select! {
            () = host.changed.notified(), if !failed => {}
            () = sleep(RETRY_INTERVAL), if failed => {}
            () = shutdown.cancelled() => return,
        }
    }
}

/// Runs `body` on a started `system` with the [`pump`] attached, then shuts down in order however `body` ended:
/// [`ProcessHost::shutdown`] within [`IO_TIMEOUT`] while the system, pump and store still run, then the system's
/// bounded stop, then the pump's stop and join, and the store's close last. Returns `body`'s result, an error
/// included, with what the cleanup could not confirm.
pub async fn run_host<T, E>(
    store: &Store,
    system: &ActorSystem,
    host: &Arc<ProcessHost>,
    body: impl Future<Output = Result<T, E>>,
) -> (Result<T, E>, CleanupReport) {
    let stop_pump = CancellationToken::new();
    let pump_task = tokio::spawn(pump(host.clone(), system.clone(), stop_pump.clone()));
    let result = body.await;
    let mut report = host.shutdown(IO_TIMEOUT).await;
    system.stop().await;
    stop_pump.cancel();
    if let Err(e) = pump_task.await {
        report.pump = Some(e.to_string());
    }
    store.close().await;
    (result, report)
}

/// Spawns the example executable's inert child mode: `program` with `args`, fixed when the host is built, with
/// stdout and stderr discarded and no shell. A child spawned with [`Self::exit_with`] exits normally with that code;
/// one without waits until it is stopped. Children are killed when their handle is dropped, best effort.
pub struct OsBackend {
    program: PathBuf,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    exit: Option<i32>,
}

impl OsBackend {
    /// Runs `program` with `args`.
    pub fn new(program: impl Into<PathBuf>, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        Self { program: program.into(), args: args.into_iter().map(Into::into).collect(), envs: Vec::new(), exit: None }
    }

    /// Adds the environment variable `key`.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Makes each child exit with `code` at once instead of waiting for a stop.
    pub fn exit_with(mut self, code: i32) -> Self {
        self.exit = Some(code);
        self
    }
}

impl ProcessBackend for OsBackend {
    fn spawn(&self) -> BoxFuture<'_, io::Result<Box<dyn ChildProcess>>> {
        Box::pin(async move {
            let mut child = tokio::process::Command::new(&self.program)
                .args(&self.args)
                .envs(self.envs.iter().map(|(k, v)| (k, v)))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()?;
            let mut stdin = child.stdin.take();
            if let (Some(code), Some(input)) = (self.exit, stdin.as_mut()) {
                // A child that cannot read its control line has exited already; its wait says how.
                let _ = input.write_all(format!("exit {code}\n").as_bytes()).await;
            }
            Ok(Box::new(OsChild { child, _stdin: stdin }) as Box<dyn ChildProcess>)
        })
    }
}

struct OsChild {
    child: tokio::process::Child,
    /// Held open: the child waits on it.
    _stdin: Option<tokio::process::ChildStdin>,
}

impl ChildProcess for OsChild {
    fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    fn start_kill(&mut self) -> io::Result<()> {
        self.child.start_kill()
    }

    fn wait(&mut self) -> BoxFuture<'_, io::Result<Option<i32>>> {
        Box::pin(async move { Ok(self.child.wait().await?.code()) })
    }
}

/// The inert child mode: reads one control line of at most 32 bytes from stdin. `exit N` exits with `N`, the end of
/// the input (the host is gone) exits with 0, anything else with 2. Until the line arrives it waits, so a child
/// whose host writes nothing runs until it is stopped. It touches nothing else and spawns nothing.
pub fn child_main() -> ! {
    let mut line = String::new();
    let code = match io::BufReader::new(io::Read::take(io::stdin(), 32)).read_line(&mut line) {
        Ok(0) => 0,
        Ok(_) => line.trim().strip_prefix("exit ").and_then(|code| code.parse().ok()).unwrap_or(2),
        Err(_) => 2,
    };
    std::process::exit(code)
}
