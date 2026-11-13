//! A worker: runs the turns of ordinary Rust actors that it leases from a [`TurnSource`] instead of a store (ADR 0039).
//!
//! The worker shares the node's executor: the same handlers, `Turn` buffering, raw sends and creates, timeouts,
//! decode and panic handling, watches, controls, observers and handler-phase arbitration. What differs is where the
//! lease lives: each grant names its own TTL, renewals go through the source, and a finished handler's report is
//! completed through the source, retried with the same report until it is acknowledged, the budget runs out or the
//! lease is lost.
//!
//! **Capacity.** Background polling and [`Worker::run_due_turns`] share one pool of `concurrency` slots: a lease
//! request reserves its slots before it is sent and keeps them until each leased turn's task ended, so requests in
//! flight plus running turns never exceed the limit. A dropped manual call cancels only its own turns, which stay
//! counted until their cleanup ended.
//!
//! **Life cycle.** No node row, heartbeat, sweep or listener: the worker owns none of the source's life cycle and
//! never stops or closes it. A lease request that answers a contract mismatch, an invalid request or a fatal error
//! stops the worker gracefully, and [`Worker::stopped`] reports that first error.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{Notify, OnceCell, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::executor::{Holder, HostedType, LeaseTiming, MIN_GRANT, TurnExecutor, lease_deadline};
use super::limits;
use super::resolver::{Registry, TypeResolver};
use super::results::{LeaseTurnsResult, RegisterContractResult, TurnLease};
use super::source::{SourceError, TurnSource};
use super::system::{MAX_LEASE_BATCH, MAX_LEASE_TYPES, NodeState, StartError, StartProblem};
use crate::actor::{Actor, ActorType, HostedActor};
use crate::address::TypeName;
use crate::observer::{Observers, TurnObserver};
use crate::store::{Claim, NodeId};

/// Configuration of a [`Worker`], passed to [`WorkerBuilder::config`]. Every value has a default. There is no TTL,
/// heartbeat or claim limit: the source decides them for each lease.
///
/// ```
/// use std::time::Duration;
/// use cicada::runtime::WorkerConfig;
///
/// let config = WorkerConfig { concurrency: 4, poll_interval: Duration::from_millis(200), ..WorkerConfig::default() };
/// ```
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// This worker's identity, recorded as the owner of its leases (at most 200 characters, no NUL). Random by default.
    pub worker_id: NodeId,
    /// Longest wait between lease requests when a request came back sparse, empty or unavailable and nothing wakes the
    /// worker earlier. Default 500 ms.
    pub poll_interval: Duration,
    /// Maximum turn duration for types that do not set one. Default 15 min.
    pub default_max_turn: Duration,
    /// Maximum number of turns running, plus slots reserved by lease requests in flight. Default 16.
    pub concurrency: usize,
    /// On stop, how long running turns may take to finish before they are cancelled. Default 30 s.
    pub shutdown_grace_period: Duration,
    /// On stop, how long cancelled turns (and the polling loop) may take to end. Default 5 s.
    pub shutdown_cancel_timeout: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: NodeId::random(),
            poll_interval: Duration::from_millis(500),
            default_max_turn: Duration::from_secs(15 * 60),
            concurrency: 16,
            shutdown_grace_period: Duration::from_secs(30),
            shutdown_cancel_timeout: Duration::from_secs(5),
        }
    }
}

impl WorkerConfig {
    fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let id = self.worker_id.as_str();
        if id.chars().count() > limits::MAX_ID_LENGTH {
            problems.push(format!("worker_id must be at most {} characters long", limits::MAX_ID_LENGTH));
        }
        if id.contains('\0') {
            problems.push("worker_id must not contain a NUL character".to_owned());
        }
        if self.poll_interval.is_zero() {
            problems.push("poll_interval must be positive".to_owned());
        }
        if self.default_max_turn.is_zero() {
            problems.push("default_max_turn must be positive".to_owned());
        }
        if self.concurrency < 1 {
            problems.push("concurrency must be at least 1".to_owned());
        }
        let now = Instant::now();
        for (name, value) in [
            ("poll_interval", self.poll_interval),
            ("default_max_turn", self.default_max_turn),
            ("shutdown_grace_period", self.shutdown_grace_period),
            ("shutdown_cancel_timeout", self.shutdown_cancel_timeout),
        ] {
            if now.checked_add(value).is_none() {
                problems.push(format!("{name} cannot be represented on the local monotonic clock"));
            }
        }
        problems
    }
}

/// Why a [`Worker`] call failed, or why it stopped by itself. Cloneable: [`Worker::stopped`] hands the same error to
/// every waiter.
#[derive(Debug, Clone, thiserror::Error)]
pub enum WorkerError {
    /// The worker cannot initialize (a setting, an actor definition or a contract), or it is stopping or stopped.
    #[error(transparent)]
    Start(#[from] StartError),
    /// The source failed.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// The source has a hosted type registered with another contract hash, or not at all: an old worker for a replaced
    /// contract.
    #[error(
        "The source has {type_name} registered {}, this worker has {requested}",
        .registered.as_deref().map(|h| format!("as {h}")).unwrap_or_else(|| "nowhere".to_owned())
    )]
    ContractMismatch {
        /// The type.
        type_name: TypeName,
        /// The registered hash, or `None` when the type is not registered.
        registered: Option<String>,
        /// The hash this worker leased with.
        requested: String,
    },
}

impl WorkerError {
    /// True for an error that stops the worker when a lease request gets it: anything but an unavailable source.
    fn is_permanent(&self) -> bool {
        !matches!(self, WorkerError::Source(SourceError::Unavailable(_)))
    }
}

fn state_error(state: NodeState) -> StartError {
    StartError { problems: vec![StartProblem::Other(format!("Worker is {state}"))] }
}

/// Builds a [`Worker`].
pub struct WorkerBuilder {
    source: Arc<dyn TurnSource>,
    config: WorkerConfig,
    hosted: Vec<Arc<dyn HostedActor>>,
    observers: Observers,
}

impl WorkerBuilder {
    /// Hosts an actor type on this worker: it registers its contract through the source and runs its turns.
    pub fn host<A: Actor>(mut self, actor_type: impl Into<ActorType<A>>) -> Self {
        self.hosted.push(actor_type.into().into_hosted());
        self
    }

    /// Adds `observer`, told when each turn this worker runs starts and ends, as on a node (see [`crate::observer`]).
    /// An observer's turn ends only when the source acknowledged its completion.
    pub fn observe(mut self, observer: impl TurnObserver) -> Self {
        self.observers.push(Arc::new(observer));
        self
    }

    /// Sets the worker's configuration.
    pub fn config(mut self, config: WorkerConfig) -> Self {
        self.config = config;
        self
    }

    /// Builds the worker. It does nothing until [`Worker::start`], [`Worker::initialize`] or
    /// [`Worker::run_due_turns`].
    pub fn build(self) -> Worker {
        let resolver = Arc::new(TypeResolver::new(Registry::Source(self.source.clone())));
        let capacity = Arc::new(Capacity { limit: self.config.concurrency, used: Mutex::new(0) });
        Worker {
            inner: Arc::new(Inner {
                source: self.source,
                config: self.config,
                hosted: self.hosted,
                observers: self.observers,
                resolver,
                initialized: OnceCell::new(),
                lifecycle: Mutex::new(Lifecycle::default()),
                state: AtomicU8::new(NodeState::New as u8),
                wake: Arc::new(Notify::new()),
                running: Arc::new(AtomicUsize::new(0)),
                capacity,
                tracker: TaskTracker::new(),
                cancel_turns: CancellationToken::new(),
                stop_polling: CancellationToken::new(),
                stopped: watch::Sender::new(false),
            }),
        }
    }
}

struct Initialized {
    executor: Arc<TurnExecutor>,
    /// The hosted types with the contract hash each was registered with.
    types: BTreeMap<TypeName, String>,
}

#[derive(Default)]
struct Lifecycle {
    poll: Option<JoinHandle<()>>,
    /// The first error that stopped the worker by itself.
    error: Option<WorkerError>,
}

struct Inner {
    source: Arc<dyn TurnSource>,
    config: WorkerConfig,
    hosted: Vec<Arc<dyn HostedActor>>,
    observers: Observers,
    resolver: Arc<TypeResolver>,
    initialized: OnceCell<Arc<Initialized>>,
    /// Guards the state transitions and the hand-off of leased turns; never held across an `.await`.
    lifecycle: Mutex<Lifecycle>,
    state: AtomicU8,
    /// Wakes the polling loop: a commit that sent messages, a turn that ended, a slot given back.
    wake: Arc<Notify>,
    running: Arc<AtomicUsize>,
    capacity: Arc<Capacity>,
    /// Every turn task, of polling and of manual batches.
    tracker: TaskTracker,
    /// Cancels every turn after the grace period of a stop; a manual batch cancels a child of it.
    cancel_turns: CancellationToken,
    stop_polling: CancellationToken,
    stopped: watch::Sender<bool>,
}

/// Runs the turns of ordinary Rust actors through a [`TurnSource`], without a store (ADR 0039). Cheap to clone: clones
/// share the worker.
///
/// ```no_run
/// # async fn example(server: cicada::ActorSystem) -> Result<(), Box<dyn std::error::Error>> {
/// use cicada::Worker;
///
/// let worker = Worker::builder(server) // any TurnSource; an ActorSystem in process
///     // .host(SyncActor { .. })
///     .build();
/// worker.start().await?;
/// // ...
/// worker.stop().await;
/// # Ok(())
/// # }
/// ```
///
/// Life cycle: [`Self::start`] validates the configuration and the hosted types, registers their contracts through the
/// source and starts polling; [`Self::stop`] stops polling, lets running turns finish within the grace period, then
/// cancels the rest and releases the leases of turns whose handler had not ended. A worker cannot start again once
/// stopped. Without `start`, [`Self::initialize`] plus [`Self::run_due_turns`] drive turns manually.
#[derive(Clone)]
pub struct Worker {
    inner: Arc<Inner>,
}

impl fmt::Debug for Worker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Worker")
            .field("worker", &self.inner.config.worker_id)
            .field("state", &self.state())
            .field("hosts", &self.inner.hosted.iter().map(|h| h.name()).collect::<Vec<_>>())
            .finish()
    }
}

impl Worker {
    /// A builder for a worker that leases its turns from `source`.
    pub fn builder(source: impl TurnSource + 'static) -> WorkerBuilder {
        WorkerBuilder {
            source: Arc::new(source),
            config: WorkerConfig::default(),
            hosted: Vec::new(),
            observers: Observers::default(),
        }
    }

    /// The worker's configuration.
    pub fn config(&self) -> &WorkerConfig {
        &self.inner.config
    }

    /// Current life-cycle state.
    pub fn state(&self) -> NodeState {
        match self.inner.state.load(Ordering::SeqCst) {
            0 => NodeState::New,
            1 => NodeState::Started,
            2 => NodeState::Stopping,
            _ => NodeState::Stopped,
        }
    }

    fn set_state(&self, state: NodeState) {
        self.inner.state.store(state as u8, Ordering::SeqCst);
    }

    fn lifecycle(&self) -> MutexGuard<'_, Lifecycle> {
        self.inner.lifecycle.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Number of turn tasks running right now; slots reserved by lease requests in flight are not counted.
    pub fn running_turns(&self) -> usize {
        self.inner.running.load(Ordering::SeqCst)
    }

    /// The names of the hosted types (valid types only).
    pub fn hosted_types(&self) -> BTreeSet<TypeName> {
        self.inner.hosted.iter().filter_map(|h| h.type_name().cloned()).collect()
    }

    // ---- life cycle ----

    /// Validates the configuration and the hosted types (at most [`MAX_LEASE_TYPES`]) and registers every hosted
    /// type's contract through the source, requiring the stored hash to be this worker's. A failure can be retried; a
    /// success is kept. It runs no turn.
    pub async fn initialize(&self) -> Result<(), StartError> {
        self.initialized().await.map(|_| ())
    }

    async fn initialized(&self) -> Result<&Arc<Initialized>, StartError> {
        self.inner.initialized.get_or_try_init(|| self.do_initialize()).await
    }

    async fn do_initialize(&self) -> Result<Arc<Initialized>, StartError> {
        let inner = &self.inner;
        let mut problems: Vec<StartProblem> = inner.config.validate().into_iter().map(StartProblem::Other).collect();
        for hosted in &inner.hosted {
            problems
                .extend(hosted.problems().into_iter().map(|p| StartProblem::Other(format!("{}: {p}", hosted.name()))));
        }
        if inner.hosted.len() > MAX_LEASE_TYPES {
            problems.push(StartProblem::Other(format!(
                "A worker hosts at most {MAX_LEASE_TYPES} types, got {}",
                inner.hosted.len()
            )));
        }
        if !problems.is_empty() {
            return Err(StartError { problems });
        }
        let mut types: HashMap<TypeName, HostedType> = HashMap::new();
        for hosted in &inner.hosted {
            let name = hosted.type_name().expect("a valid type has a name").clone();
            if types.contains_key(&name) {
                problems.push(StartProblem::Other(format!("{}: type {name} is hosted twice", hosted.name())));
                continue;
            }
            let max_turn = hosted.max_turn().unwrap_or(inner.config.default_max_turn);
            if Instant::now().checked_add(max_turn).is_none() {
                problems.push(StartProblem::Other(format!(
                    "{}: MAX_TURN cannot be represented on the local monotonic clock",
                    hosted.name()
                )));
            }
            types.insert(name, HostedType { actor: hosted.clone(), max_turn });
        }
        if !problems.is_empty() {
            return Err(StartError { problems });
        }
        let mut hashes = BTreeMap::new();
        for (name, hosted) in &types {
            let contract = hosted.actor.contract().expect("a valid type has a contract");
            let actor = hosted.actor.name();
            let conflict = |registered_hash: String, reason: &str| StartProblem::ContractConflict {
                actor: actor.to_owned(),
                type_name: name.clone(),
                registered_hash,
                node_hash: contract.hash().to_owned(),
                reason: reason.to_owned(),
            };
            match inner.source.register_contract(contract).await {
                Ok(RegisterContractResult::Registered(stored))
                    if stored.definition.contract_hash == contract.hash() =>
                {
                    hashes.insert(name.clone(), contract.hash().to_owned());
                }
                Ok(RegisterContractResult::Registered(stored)) => problems.push(conflict(
                    stored.definition.contract_hash,
                    "the source answered the registration with another contract hash",
                )),
                Ok(RegisterContractResult::Conflict { existing, .. }) => problems.push(conflict(
                    existing.definition.contract_hash,
                    "the contract differs from the one registered under its type name",
                )),
                Ok(RegisterContractResult::InvalidContract(why)) => {
                    problems.push(StartProblem::Other(format!("{actor}: the source refused the contract: {why}")))
                }
                Err(e) => problems.push(StartProblem::Other(format!("{actor}: registering the contract failed: {e}"))),
            }
        }
        if !problems.is_empty() {
            return Err(StartError { problems });
        }
        let executor = Arc::new(TurnExecutor {
            holder: Holder::Source(inner.source.clone()),
            resolver: inner.resolver.clone(),
            hosted: Arc::new(types),
            observers: inner.observers.clone(),
            max_lost_claims: None,
            renew_while_stopping: Some(inner.config.shutdown_cancel_timeout),
            wake: inner.wake.clone(),
        });
        Ok(Arc::new(Initialized { executor, types: hashes }))
    }

    /// Initializes (see [`Self::initialize`]) and starts polling. Fails when the worker was already started or stopped,
    /// also when it was stopped while its contracts were being registered. A worker that hosts nothing starts and never
    /// sends a lease request.
    pub async fn start(&self) -> Result<(), StartError> {
        let state = self.state();
        if state != NodeState::New {
            return Err(state_error(state));
        }
        // No lock is held while the source registers: a stop meanwhile marks the worker stopped.
        let init = self.initialized().await?.clone();
        let mut lifecycle = self.lifecycle();
        let state = self.state();
        if state != NodeState::New {
            return Err(state_error(state));
        }
        if !init.types.is_empty() {
            lifecycle.poll = Some(tokio::spawn(self.clone().poll_loop(init.clone())));
        }
        self.set_state(NodeState::Started);
        drop(lifecycle);
        tracing::info!(
            "Worker {} started; hosting {:?}",
            self.inner.config.worker_id,
            init.types.keys().map(|t| t.as_str()).collect::<Vec<_>>()
        );
        Ok(())
    }

    /// Stops the worker with the configured grace period; see [`Self::stop_with_grace`].
    pub async fn stop(&self) {
        self.stop_with_grace(self.inner.config.shutdown_grace_period).await
    }

    /// Stops the worker: no new lease requests (a response that comes after is released); running turns, of polling
    /// and of manual batches, get `grace` to finish; the rest are cancelled, and a turn whose handler had not ended
    /// releases its lease once it stopped. A cancelled turn keeps renewing while its work stops, for at most the cancel
    /// timeout; one that ignores cancellation beyond it is left behind, no longer renewed: its lease is not released
    /// but expires. A turn whose handler ended still completes within its budget. Works in
    /// every state, also on a worker that was never started; the source is neither stopped nor closed. A second call
    /// waits for the first.
    pub async fn stop_with_grace(&self, grace: Duration) {
        let (stopping, poll) = {
            let mut lifecycle = self.lifecycle();
            match self.state() {
                NodeState::Stopping | NodeState::Stopped => (true, None),
                NodeState::New | NodeState::Started => {
                    self.set_state(NodeState::Stopping);
                    (false, lifecycle.poll.take())
                }
            }
        };
        if stopping {
            let _ = self.inner.stopped.subscribe().wait_for(|stopped| *stopped).await;
            return;
        }
        self.inner.stop_polling.cancel();
        self.inner.wake.notify_one();
        let cancel_timeout = self.inner.config.shutdown_cancel_timeout;
        if let Some(poll) = poll {
            let abort = poll.abort_handle();
            if timeout(cancel_timeout, poll).await.is_err() {
                tracing::warn!("The polling loop did not stop within {cancel_timeout:?}; leaving it behind");
                abort.abort();
            }
        }
        let tracker = &self.inner.tracker;
        tracker.close();
        if timeout(grace, tracker.wait()).await.is_err() {
            tracing::info!("Cancelling {} turns after the grace period of {grace:?}", self.running_turns());
            self.inner.cancel_turns.cancel();
            if timeout(cancel_timeout, tracker.wait()).await.is_err() {
                tracing::warn!(
                    "{} turns ignored cancellation for {cancel_timeout:?}; leaving them behind, their leases expire on \
                     their own",
                    self.running_turns()
                );
            }
        }
        self.set_state(NodeState::Stopped);
        self.inner.stopped.send_replace(true);
        tracing::info!("Worker {} stopped", self.inner.config.worker_id);
    }

    /// Waits until the worker is stopped. `None` after an explicit stop; the first error when a lease request stopped
    /// it by itself (a contract mismatch, an invalid request or a fatal source error), the same for every waiter.
    pub async fn stopped(&self) -> Option<WorkerError> {
        let _ = self.inner.stopped.subscribe().wait_for(|stopped| *stopped).await;
        self.lifecycle().error.clone()
    }

    /// Records `error` as the reason the worker stops, unless it is stopping already or has one, and stops it in a
    /// task of its own (never in the turn or loop that saw the error).
    fn fail(&self, error: WorkerError) {
        {
            let mut lifecycle = self.lifecycle();
            if !matches!(self.state(), NodeState::New | NodeState::Started) {
                return;
            }
            if lifecycle.error.is_none() {
                tracing::error!("Worker {} stops: {error}", self.inner.config.worker_id);
                lifecycle.error = Some(error);
            }
        }
        let this = self.clone();
        tokio::spawn(async move { this.stop().await });
    }

    // ---- turns ----

    /// Leases up to `limit` due turns (at most [`MAX_LEASE_BATCH`] and the free capacity) in one request and runs them
    /// concurrently, returning the number of turn tasks started once they ended. Zero without a request when `limit` is
    /// zero, no slot is free or nothing is hosted. Works on a new or started worker, sharing capacity with polling; a
    /// stopping or stopped worker answers [`WorkerError::Start`]. An unavailable source is returned, not retried; any
    /// other lease failure also stops the worker.
    ///
    /// Dropping the returned future cancels this call's turns only, as a stop after its grace period would: a turn
    /// whose handler has not ended releases its lease once it stopped, or after the cancel timeout stops renewing it
    /// to expire; one whose handler ended still completes. Their slots stay taken until then. A lease response that comes after the caller left is released.
    pub async fn run_due_turns(&self, limit: usize) -> Result<usize, WorkerError> {
        let state = self.state();
        if !matches!(state, NodeState::New | NodeState::Started) {
            return Err(state_error(state).into());
        }
        let init = self.initialized().await?.clone();
        if init.types.is_empty() || limit == 0 {
            return Ok(0);
        }
        let reservation = self.inner.capacity.reserve(limit.min(MAX_LEASE_BATCH));
        if reservation.slots == 0 {
            return Ok(0);
        }
        let cohort = self.inner.cancel_turns.child_token();
        let _cancel_on_drop = cohort.clone().drop_guard();
        let batch = match self.lease(&init, reservation).await {
            Ok(batch) => batch,
            Err(e) => {
                if e.is_permanent() {
                    self.fail(e.clone());
                }
                return Err(e);
            }
        };
        let Some(turns) = self.hand_off(&init, batch, &cohort) else {
            return Err(state_error(self.state()).into());
        };
        let count = turns.len();
        for turn in turns {
            if let Err(e) = turn.await {
                tracing::error!("A turn task failed: {e}");
            }
        }
        Ok(count)
    }

    async fn poll_loop(self, init: Arc<Initialized>) {
        let stop = self.inner.stop_polling.clone();
        let poll = self.inner.config.poll_interval;
        loop {
            if stop.is_cancelled() {
                return;
            }
            let reservation = self.inner.capacity.reserve(MAX_LEASE_BATCH);
            let asked = reservation.slots;
            let mut pause = true;
            if asked > 0 {
                let leased = tokio::select! {
                    biased;
                    _ = stop.cancelled() => return,
                    leased = self.lease(&init, reservation) => leased,
                };
                match leased {
                    Ok(batch) => {
                        let leased = batch.leases.len();
                        if self.hand_off(&init, batch, &self.inner.cancel_turns).is_none() {
                            return;
                        }
                        pause = leased < asked; // a full batch: more may be due
                    }
                    Err(e) if e.is_permanent() => {
                        self.fail(e);
                        return;
                    }
                    Err(e) => tracing::warn!("Leasing failed; retrying after {poll:?}: {e}"),
                }
            }
            if pause {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => return,
                    _ = self.inner.wake.notified() => {}
                    _ = sleep(poll) => {}
                }
            }
        }
    }

    /// Sends one lease request for the slots of `reservation`, in a task of its own: a response that nobody receives
    /// any more releases its leases. The request is admitted under the lifecycle lock: a stopping or stopped worker
    /// sends none ([`WorkerError::Start`]), and once a stop began no request starts. The batch is checked before it is
    /// handed over; a malformed one is released and is a fatal error.
    async fn lease(&self, init: &Initialized, reservation: Reservation) -> Result<LeasedBatch, WorkerError> {
        let source = self.inner.source.clone();
        let owner = self.inner.config.worker_id.clone();
        let types = init.types.clone();
        let (tx, rx) = oneshot::channel();
        {
            let _lifecycle = self.lifecycle();
            let state = self.state();
            if !matches!(state, NodeState::New | NodeState::Started) {
                return Err(state_error(state).into());
            }
            tokio::spawn(async move {
                let limit = reservation.slots;
                let sent = Instant::now();
                let result = match source.lease_turns(&owner, &types, limit).await {
                    Ok(LeaseTurnsResult::Leased(leases)) => checked_batch(source, leases, &types, reservation, sent),
                    Ok(LeaseTurnsResult::ContractMismatch { type_name, registered, requested }) => {
                        Err(WorkerError::ContractMismatch { type_name, registered, requested })
                    }
                    Err(e) => Err(WorkerError::Source(e)),
                };
                // Unsent or never received, the batch releases its leases when dropped.
                let _ = tx.send(result);
            });
        }
        rx.await.map_err(|_| SourceError::Unavailable("leasing failed: the lease task ended".to_owned()))?
    }

    /// Starts a turn task for every lease of `batch`, each with a slot of its reservation and cancelled by `cancel`;
    /// `None` (and the batch released) when the worker is stopping or stopped.
    fn hand_off(
        &self,
        init: &Initialized,
        mut batch: LeasedBatch,
        cancel: &CancellationToken,
    ) -> Option<Vec<JoinHandle<()>>> {
        let _lifecycle = self.lifecycle();
        if !matches!(self.state(), NodeState::New | NodeState::Started) {
            return None;
        }
        let leases = std::mem::take(&mut batch.leases);
        let turns = leases
            .into_iter()
            .map(|(claim, timing)| {
                let slot = batch.reservation.slot();
                let executor = init.executor.clone();
                let cancel = cancel.clone();
                let running = RunningGuard::new(self.inner.running.clone());
                let wake = self.inner.wake.clone();
                self.inner.tracker.spawn(async move {
                    executor.execute(claim, timing, cancel).await;
                    drop(running);
                    drop(slot);
                    wake.notify_one();
                })
            })
            .collect();
        Some(turns)
    }
}

/// Checks a leased batch before anything runs: at most `reservation` leases, of requested types only, one per actor,
/// each message addressed to its actor, and a TTL the worker can use from `sent`. A malformed batch is released, best
/// effort, and is a fatal source error.
fn checked_batch(
    source: Arc<dyn TurnSource>,
    leases: Vec<TurnLease>,
    types: &BTreeMap<TypeName, String>,
    reservation: Reservation,
    sent: Instant,
) -> Result<LeasedBatch, WorkerError> {
    let limit = reservation.slots;
    let mut problem = None;
    let mut actors = BTreeSet::new();
    if leases.len() > limit {
        problem = Some(format!("the source leased {} turns, {limit} were asked for", leases.len()));
    }
    for lease in &leases {
        if problem.is_some() {
            break;
        }
        let actor = &lease.actor;
        problem = if !types.contains_key(actor.type_name()) {
            Some(format!("the source leased {actor}, whose type was not asked for"))
        } else if !actors.insert(actor.clone()) {
            Some(format!("the source leased {actor} twice in one batch"))
        } else if &lease.message.to != actor {
            Some(format!("the source leased {actor} with a message for {}", lease.message.to))
        } else if lease.ttl < MIN_GRANT || lease_deadline(sent, lease.ttl).is_none() {
            Some(format!("the source leased {actor} for a TTL of {:?}, which cannot be used", lease.ttl))
        } else {
            None
        };
    }
    let leases = leases
        .into_iter()
        .map(|lease| {
            let timing = LeaseTiming { sent, ttl: lease.ttl };
            let claim = Claim {
                actor: lease.actor,
                epoch: lease.epoch,
                message: lease.message,
                state: lease.state,
                version: lease.version,
                claimed_at: lease.claimed_at,
                lease_until: lease.lease_until,
            };
            (claim, timing)
        })
        .collect();
    let batch = LeasedBatch { source, leases, reservation };
    match problem {
        None => Ok(batch),
        // Dropping the batch releases what it holds.
        Some(why) => Err(WorkerError::Source(SourceError::Fatal(format!("Malformed lease batch: {why}")))),
    }
}

/// The slots of the worker's concurrency: taken by reservations of lease requests and by the turns they started.
struct Capacity {
    limit: usize,
    used: Mutex<usize>,
}

impl Capacity {
    /// Reserves up to `wanted` free slots.
    fn reserve(self: &Arc<Self>, wanted: usize) -> Reservation {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        let slots = wanted.min(self.limit.saturating_sub(*used));
        *used += slots;
        Reservation { capacity: self.clone(), slots }
    }

    fn give_back(&self, slots: usize) {
        if slots > 0 {
            let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
            *used = used.saturating_sub(slots);
        }
    }
}

/// Slots reserved for a lease request; those not handed to a turn are given back when it is dropped.
struct Reservation {
    capacity: Arc<Capacity>,
    slots: usize,
}

impl Reservation {
    /// Moves one reserved slot to a turn. A batch never holds more leases than its reservation has slots.
    fn slot(&mut self) -> Slot {
        debug_assert!(self.slots > 0, "a batch holds at most its reservation");
        self.slots = self.slots.saturating_sub(1);
        Slot(self.capacity.clone())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.capacity.give_back(self.slots);
    }
}

/// The slot of one running turn, given back when its task ends.
struct Slot(Arc<Capacity>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.give_back(1);
    }
}

/// Leases on their way from the source to their turn tasks, with the slots reserved for them. Leases still in the batch
/// when it is dropped (the caller left, the worker stopped, or the batch was malformed) get one release attempt in the
/// background, best effort.
struct LeasedBatch {
    source: Arc<dyn TurnSource>,
    leases: Vec<(Claim, LeaseTiming)>,
    reservation: Reservation,
}

impl Drop for LeasedBatch {
    fn drop(&mut self) {
        let leases = std::mem::take(&mut self.leases);
        if leases.is_empty() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("{} leases dropped outside a runtime; they expire", leases.len());
            return;
        };
        let source = self.source.clone();
        runtime.spawn(async move {
            for (claim, _) in leases {
                tracing::info!(
                    "Releasing the lease of {} for {}: nobody takes it any more",
                    claim.actor,
                    claim.message.id
                );
                if let Err(e) = source.release_turn(&claim.actor, claim.epoch).await {
                    tracing::warn!("Releasing the lease of {} failed; it expires: {e}", claim.actor);
                }
            }
        });
    }
}

/// Counts a running turn for as long as it lives.
struct RunningGuard(Arc<AtomicUsize>);

impl RunningGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[allow(dead_code)]
fn _assert_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<Worker>();
    check::<Arc<dyn TurnSource>>();
}
