use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::{Mutex, Notify, OnceCell};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::executor::{HostedType, LeaseSettings, TurnExecutor};
use super::limits;
use super::resolver::{RegisteredType, TypeResolver};
use super::results::{
    ActorSnapshot, BulkResult, BulkSelection, CreateActorResult, MailboxEntry, NodeHealth, NodeInfo, OpError,
    RetryResult, SendResult, WatchResult,
};
use crate::actor::{Actor, ActorType, HostedActor};
use crate::address::{ActorAddress, ActorRef, TypeName};
use crate::message::{self, MessageFamily};
use crate::observer::{Observers, TurnObserver};
use crate::schema::SchemaCheck;
use crate::store::{
    Backend, CancelResult, Claim, CreateResult, DeleteResult, DeliverNowResult, EnqueueResult, Message, MessageId,
    MessageStatus, NodeId, OutcomeRecord, OutcomeStatus, OutgoingMessage, Page, PageCursor, RegisterResult,
    RemoveTypeResult, RunningTurn, SetWatchResult, Store, StoreError, StoreStats,
};
use crate::time::{self, MAX_INSTANT, Timestamp};
use crate::turn::SendOptions;
use crate::watch::{self, Watch, WatchTarget};

const MIN_SLEEP: Duration = Duration::from_millis(1);
/// How often a node refreshes its row in the node table.
const NODE_HEARTBEAT: Duration = Duration::from_secs(10);
/// A node last seen less than this long ago is alive.
const NODE_ALIVE_FOR: Duration = Duration::from_secs(30);
/// A node last seen less than this long ago (and not alive) is suspect; older ones are dead.
const NODE_SUSPECT_FOR: Duration = Duration::from_secs(120);
/// Most running turns read per page when the node view counts them.
const PAGE_OF_TURNS: usize = 1000;

/// The most messages one bulk call acts on, and the most ids it takes.
pub const BULK_LIMIT: usize = 1000;

/// A node could not start: an invalid actor type, an invalid configuration, or a contract conflict in the
/// TypeRegistry. It prints one line per problem.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Actor system cannot start:\n{}", .problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n"))]
pub struct StartError {
    /// Every problem found, one per cause.
    pub problems: Vec<StartProblem>,
}

/// One problem that stopped a node from starting. A contract conflict is a case of its own, so that a host can
/// recognise it; every other problem holds its sentence. New cases may be added: match with a `_` arm.
///
/// Printed, a problem is the sentence a node reports: a conflict as "`actor`: `reason` (registered `type_name`
/// `registered_hash`, this node `node_hash`)".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartProblem {
    /// A hosted type's contract differs from the one registered under its type name.
    #[error("{actor}: {reason} (registered {type_name} {registered_hash}, this node {node_hash})")]
    ContractConflict {
        /// The Rust type name of the actor.
        actor: String,
        /// The type name.
        type_name: TypeName,
        /// The hash of the registered contract.
        registered_hash: String,
        /// The hash of the contract of this node.
        node_hash: String,
        /// Why the contracts conflict.
        reason: String,
    },
    /// Any other problem: a setting out of range, a malformed actor definition (one problem per reason), a type hosted
    /// twice, a store failure while registering, or a node that is not new. The sentence names what is wrong, and the
    /// actor when there is one.
    #[error("{0}")]
    Other(String),
}

/// Life-cycle states of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeState {
    /// Built, not started.
    New = 0,
    /// Running its dispatch loop.
    Started = 1,
    /// Stopping.
    Stopping = 2,
    /// Stopped.
    Stopped = 3,
}

impl NodeState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => NodeState::New,
            1 => NodeState::Started,
            2 => NodeState::Stopping,
            _ => NodeState::Stopped,
        }
    }

    /// The upper-case name, as served by `GET /health`.
    pub fn name(self) -> &'static str {
        match self {
            NodeState::New => "NEW",
            NodeState::Started => "STARTED",
            NodeState::Stopping => "STOPPING",
            NodeState::Stopped => "STOPPED",
        }
    }
}

impl fmt::Display for NodeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Configuration of one node, passed to [`ActorSystemBuilder::config`]. Every value has a default; outcome retention
/// is the store's.
///
/// ```
/// use std::time::Duration;
/// use cicada::runtime::NodeConfig;
///
/// let config = NodeConfig { lease_ttl: Duration::from_secs(60), concurrency: 4, ..NodeConfig::default() };
/// ```
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// This node's identity, recorded as lease owner. Random by default.
    pub node_id: NodeId,
    /// Lease TTL; a turn whose node dies is claimed again after at most this long. Default 30 s.
    pub lease_ttl: Duration,
    /// Heartbeat interval; `None` means `lease_ttl / 3`. At most `lease_ttl / 2`.
    pub heartbeat_interval: Option<Duration>,
    /// Longest sleep of the dispatch loop between claims when nothing wakes it earlier. Default 500 ms.
    pub poll_interval: Duration,
    /// Maximum turn duration for types that do not set one. Default 15 min.
    pub default_max_turn: Duration,
    /// Maximum number of turns running concurrently on this node. Default 16.
    pub concurrency: usize,
    /// On stop, how long running turns may take to finish before they are cancelled. Default 30 s.
    pub shutdown_grace_period: Duration,
    /// On stop, how long cancelled turns may take to end after the grace period. Default 5 s.
    pub shutdown_cancel_timeout: Duration,
    /// How often expired outcomes are swept from the store. Default 1 h.
    pub sweep_interval: Duration,
    /// The claim limit (ADR 0025): a message with this many lost claims ends `failed` with the error
    /// `claim limit: <n> lost claims` when this node claims it, without running its handler, and its watchers are told.
    /// A claim is lost when another claim takes its place although its turn neither committed nor gave the lease back:
    /// its node died, its lease ran out, or its commit could not get through. A node that stops in good order and
    /// gives its claims back loses none. `None` turns the limit off; `Some(0)` is a start problem. Default `Some(5)`.
    /// Every node of a system should have the same; each applies its own.
    pub max_lost_claims: Option<u32>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            node_id: NodeId::random(),
            lease_ttl: Duration::from_secs(30),
            heartbeat_interval: None,
            poll_interval: Duration::from_millis(500),
            default_max_turn: Duration::from_secs(15 * 60),
            concurrency: 16,
            shutdown_grace_period: Duration::from_secs(30),
            shutdown_cancel_timeout: Duration::from_secs(5),
            sweep_interval: Duration::from_secs(3600),
            max_lost_claims: Some(5),
        }
    }
}

impl NodeConfig {
    fn heartbeat(&self) -> Duration {
        self.heartbeat_interval.unwrap_or(self.lease_ttl / 3)
    }

    fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.lease_ttl.is_zero() {
            problems.push("lease_ttl must be positive".to_owned());
        }
        let heartbeat = self.heartbeat();
        if heartbeat.is_zero() || heartbeat > self.lease_ttl / 2 {
            problems.push("heartbeat_interval must be in (0, lease_ttl / 2]".to_owned());
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
        if self.sweep_interval.is_zero() {
            problems.push("sweep_interval must be positive".to_owned());
        }
        if self.max_lost_claims == Some(0) {
            problems.push("max_lost_claims must be at least 1, or None for no limit".to_owned());
        }
        problems
    }
}

/// Builds an [`ActorSystem`].
pub struct ActorSystemBuilder {
    store: Store,
    config: NodeConfig,
    hosted: Vec<Arc<dyn HostedActor>>,
    observers: Observers,
}

impl ActorSystemBuilder {
    /// Hosts an actor type on this node: it registers its contract and runs its turns.
    pub fn host<A: Actor>(mut self, actor_type: impl Into<ActorType<A>>) -> Self {
        self.hosted.push(actor_type.into().into_hosted());
        self
    }

    /// Adds `observer` to this node: it is told when each turn of every type the node hosts starts and ends, and has no
    /// say in them (see [`crate::observer`]). Each call adds one; they are called in the order they were added. Pass
    /// an `Arc` to keep a handle.
    pub fn observe(mut self, observer: impl TurnObserver) -> Self {
        self.observers.push(Arc::new(observer));
        self
    }

    /// Sets the node's configuration.
    pub fn config(mut self, config: NodeConfig) -> Self {
        self.config = config;
        self
    }

    /// Builds the node. It does nothing until [`ActorSystem::start`] (or [`ActorSystem::initialize`]).
    pub fn build(self) -> ActorSystem {
        let resolver = Arc::new(TypeResolver::new(self.store.clone()));
        ActorSystem {
            inner: Arc::new(Inner {
                store: self.store,
                config: self.config,
                hosted: self.hosted,
                observers: self.observers,
                resolver,
                initialized: OnceCell::new(),
                lifecycle: Mutex::new(Lifecycle::default()),
                state: AtomicU8::new(NodeState::New as u8),
                wake: Arc::new(Notify::new()),
                running: Arc::new(AtomicUsize::new(0)),
                stopping: Arc::new(AtomicBool::new(false)),
            }),
        }
    }
}

struct Initialized {
    executor: Arc<TurnExecutor>,
    types: BTreeSet<TypeName>,
}

#[derive(Default)]
struct Lifecycle {
    tracker: TaskTracker,
    cancel_turns: CancellationToken,
    dispatch: Option<JoinHandle<()>>,
    background: Vec<JoinHandle<()>>,
    /// Stops the heartbeat of this node's row; awaited on stop so that no heartbeat follows the row's removal.
    node_stop: CancellationToken,
    node_heartbeat: Option<JoinHandle<()>>,
}

struct Inner {
    store: Store,
    config: NodeConfig,
    hosted: Vec<Arc<dyn HostedActor>>,
    observers: Observers,
    resolver: Arc<TypeResolver>,
    initialized: OnceCell<Arc<Initialized>>,
    lifecycle: Mutex<Lifecycle>,
    state: AtomicU8,
    wake: Arc<Notify>,
    running: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
}

/// A node: hosts actor types (registers their contracts and runs their turns through a dispatch loop) and offers every
/// HTTP operation in-process. A node that hosts no types is send-only. Cheap to clone: clones share the node.
///
/// ```no_run
/// # async fn example(store: cicada::Store) -> Result<(), Box<dyn std::error::Error>> {
/// use cicada::ActorSystem;
///
/// let system = ActorSystem::builder(store)
///     // .host(SyncActor { .. })
///     .build();
/// system.start().await?;
/// // system.send(&Sync { tenant: "acme".into() }, SyncMsg::Run).await?;
/// system.stop().await;
/// # Ok(())
/// # }
/// ```
///
/// Life cycle: [`Self::start`] validates the hosted types, registers their contracts (a conflicting contract fails the
/// start) and starts the dispatch loop; [`Self::stop`] stops claiming, lets running turns finish within the grace
/// period, then cancels the rest, releases their leases and removes the node's row. The store is owned by the caller and
/// is not closed.
///
/// Without `start`, [`Self::initialize`] plus [`Self::run_due_turns`] drive turns manually, as the test harness does.
#[derive(Clone)]
pub struct ActorSystem {
    inner: Arc<Inner>,
}

impl fmt::Debug for ActorSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorSystem")
            .field("node", &self.inner.config.node_id)
            .field("state", &self.state())
            .field("hosts", &self.inner.hosted.iter().map(|h| h.name()).collect::<Vec<_>>())
            .finish()
    }
}

impl ActorSystem {
    /// A builder for a node on `store`.
    pub fn builder(store: Store) -> ActorSystemBuilder {
        ActorSystemBuilder { store, config: NodeConfig::default(), hosted: Vec::new(), observers: Observers::default() }
    }

    /// The store, for the tests of this crate.
    #[cfg(test)]
    pub(crate) fn store(&self) -> &Store {
        &self.inner.store
    }

    /// This node's identity.
    pub fn node_id(&self) -> &NodeId {
        &self.inner.config.node_id
    }

    /// The node's configuration.
    pub fn config(&self) -> &NodeConfig {
        &self.inner.config
    }

    /// Current life-cycle state.
    pub fn state(&self) -> NodeState {
        NodeState::from_u8(self.inner.state.load(Ordering::SeqCst))
    }

    /// Number of turns this node is running right now.
    pub fn running_turns(&self) -> usize {
        self.inner.running.load(Ordering::SeqCst)
    }

    /// The names of the hosted types (valid types only).
    pub fn hosted_types(&self) -> BTreeSet<TypeName> {
        self.inner.hosted.iter().filter_map(|h| h.type_name().cloned()).collect()
    }

    // ---- life cycle ----

    /// Validates the hosted types and the configuration and registers every hosted type's contract. Idempotent.
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
            types.insert(name, HostedType { actor: hosted.clone(), max_turn });
        }
        if !problems.is_empty() {
            return Err(StartError { problems });
        }
        for hosted in types.values() {
            let def = hosted.actor.contract().expect("a valid type has a contract").to_type_def();
            match inner.store.register(def.clone()).await {
                Ok(RegisterResult::Registered(_)) => {}
                Ok(RegisterResult::Conflict { existing, reason }) => problems.push(StartProblem::ContractConflict {
                    actor: hosted.actor.name().to_owned(),
                    type_name: existing.name,
                    registered_hash: existing.contract_hash,
                    node_hash: def.contract_hash,
                    reason,
                }),
                Err(e) => problems.push(StartProblem::Other(format!(
                    "{}: registering the contract failed: {e}",
                    hosted.actor.name()
                ))),
            }
        }
        if !problems.is_empty() {
            return Err(StartError { problems });
        }
        let names: BTreeSet<TypeName> = types.keys().cloned().collect();
        let lease = LeaseSettings { ttl: inner.config.lease_ttl, heartbeat: inner.config.heartbeat() };
        let executor = Arc::new(TurnExecutor {
            store: inner.store.clone(),
            resolver: inner.resolver.clone(),
            hosted: types,
            observers: inner.observers.clone(),
            max_lost_claims: inner.config.max_lost_claims,
            lease,
            wake: inner.wake.clone(),
        });
        Ok(Arc::new(Initialized { executor, types: names }))
    }

    /// Initializes (see [`Self::initialize`]) and starts the dispatch loop, the heartbeat of its node-table row, the
    /// retention sweep and, when the store offers them, the collection of its wake-ups. Fails when the node was already
    /// started or stopped.
    pub async fn start(&self) -> Result<(), StartError> {
        let mut lifecycle = self.inner.lifecycle.lock().await;
        if self.state() != NodeState::New {
            return Err(StartError {
                problems: vec![StartProblem::Other(format!("Actor system is {}", self.state()))],
            });
        }
        let init = self.initialized().await?.clone();
        self.inner.stopping.store(false, Ordering::SeqCst);
        if !init.types.is_empty() {
            let this = self.clone();
            let tracker = lifecycle.tracker.clone();
            let cancel = lifecycle.cancel_turns.clone();
            let loop_init = init.clone();
            lifecycle.dispatch =
                Some(tokio::spawn(async move { this.dispatch_loop(loop_init, tracker, cancel).await }));
            if let Some(mut wake_ups) = self.inner.store.wake_ups(&init.types) {
                let wake = self.inner.wake.clone();
                lifecycle.background.push(tokio::spawn(async move {
                    while wake_ups.next().await.is_some() {
                        wake.notify_one();
                    }
                    tracing::warn!("Store wake-ups ended; falling back to polling");
                }));
            }
        }
        lifecycle.node_heartbeat = Some(tokio::spawn({
            let store = self.inner.store.clone();
            let node = self.inner.config.node_id.clone();
            let hosted = init.types.clone();
            let stop = lifecycle.node_stop.clone();
            async move {
                loop {
                    if let Err(e) = store.heartbeat_node(&node, &hosted).await {
                        tracing::warn!("The heartbeat of node {node} failed; retrying after {NODE_HEARTBEAT:?}: {e}");
                    }
                    tokio::select! {
                        biased;
                        _ = stop.cancelled() => return,
                        _ = sleep(NODE_HEARTBEAT) => {}
                    }
                }
            }
        }));
        let store = self.inner.store.clone();
        let interval = self.inner.config.sweep_interval;
        lifecycle.background.push(tokio::spawn(async move {
            loop {
                sleep(interval).await;
                match store.sweep_expired().await {
                    Ok(removed) if removed > 0 => tracing::debug!("Swept {removed} expired outcomes"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Retention sweep failed: {e}"),
                }
            }
        }));
        self.inner.state.store(NodeState::Started as u8, Ordering::SeqCst);
        tracing::info!(
            "Actors node {} started; hosting {:?}",
            self.inner.config.node_id,
            init.types.iter().map(|t| t.as_str()).collect::<Vec<_>>()
        );
        Ok(())
    }

    /// Stops the node with the configured grace period; see [`Self::stop_with_grace`].
    pub async fn stop(&self) {
        self.stop_with_grace(self.inner.config.shutdown_grace_period).await
    }

    /// Stops the node: no new claims; running turns get `grace` to finish and commit; the rest are cancelled and their
    /// leases released, so their messages stay pending and are claimed again. A turn that does not end within the
    /// cancel timeout after its cancellation is left behind: its lease is not released but expires. Idempotent.
    pub async fn stop_with_grace(&self, grace: Duration) {
        let mut lifecycle = self.inner.lifecycle.lock().await;
        match self.state() {
            NodeState::New => {
                self.inner.state.store(NodeState::Stopped as u8, Ordering::SeqCst);
                return;
            }
            NodeState::Stopping | NodeState::Stopped => return,
            NodeState::Started => {}
        }
        self.inner.state.store(NodeState::Stopping as u8, Ordering::SeqCst);
        self.inner.stopping.store(true, Ordering::SeqCst);
        self.inner.wake.notify_one();
        let cancel_timeout = self.inner.config.shutdown_cancel_timeout;
        if let Some(dispatch) = lifecycle.dispatch.take() {
            let abort = dispatch.abort_handle();
            if timeout(cancel_timeout, dispatch).await.is_err() {
                tracing::warn!("The dispatch loop did not stop within {cancel_timeout:?}; leaving it behind");
                abort.abort();
            }
        }
        lifecycle.tracker.close();
        if timeout(grace, lifecycle.tracker.wait()).await.is_err() {
            tracing::info!("Cancelling {} turns after the grace period of {grace:?}", self.running_turns());
            lifecycle.cancel_turns.cancel();
            if timeout(cancel_timeout, lifecycle.tracker.wait()).await.is_err() {
                tracing::warn!(
                    "{} turns ignored cancellation for {cancel_timeout:?}; leaving them behind, their leases expire on \
                     their own",
                    self.running_turns()
                );
            }
        }
        for task in lifecycle.background.drain(..) {
            task.abort();
        }
        lifecycle.node_stop.cancel();
        if let Some(heartbeat) = lifecycle.node_heartbeat.take() {
            let abort = heartbeat.abort_handle();
            if timeout(cancel_timeout, heartbeat).await.is_err() {
                abort.abort();
            }
        }
        if let Err(e) = self.inner.store.remove_node(&self.inner.config.node_id).await {
            tracing::warn!("Removing the row of node {} failed: {e}", self.inner.config.node_id);
        }
        self.inner.state.store(NodeState::Stopped as u8, Ordering::SeqCst);
        tracing::info!("Actors node {} stopped", self.inner.config.node_id);
    }

    /// Wakes the dispatch loop so it claims now instead of after its sleep. Spurious calls are harmless.
    fn wake_up(&self) {
        self.inner.wake.notify_one();
    }

    /// Claims up to `limit` due turns of the hosted types and runs them concurrently, returning when they ended. For
    /// tests and manual scheduling; a started node does this in its dispatch loop. Returns the number of turns run.
    ///
    /// Dropping the returned future cancels the turns, like a node stopping after its grace period: a turn whose
    /// handler has not ended releases its lease (in the background, once the handler stopped), one whose handler
    /// ended leaves it to expire (G6); claims still in flight are released when they arrive.
    pub async fn run_due_turns(&self, limit: usize) -> Result<usize, OpError> {
        let init = self.initialized().await?.clone();
        if init.types.is_empty() {
            return Ok(0);
        }
        let claims = self.claim(&init, limit).await?;
        let count = claims.len();
        let cancel = CancellationToken::new();
        let _cancel_on_drop = cancel.clone().drop_guard();
        // Every turn is a task of its own, started at once, so a turn cancelled right away still releases its lease.
        let turns: Vec<JoinHandle<()>> = claims
            .into_iter()
            .map(|(claim, sent)| {
                let executor = init.executor.clone();
                let cancel = cancel.clone();
                let guard = RunningGuard::new(self.inner.running.clone());
                tokio::spawn(async move {
                    executor.execute(claim, sent, cancel).await;
                    drop(guard);
                })
            })
            .collect();
        for turn in turns {
            if let Err(e) = turn.await {
                tracing::error!("A turn task failed: {e}");
            }
        }
        Ok(count)
    }

    /// Claims up to `limit` turns, paired with the local time the claim was sent (the start of their local lease
    /// deadline).
    ///
    /// The store call itself is not cancellable: it runs in a task of its own, and claims the store granted are never
    /// dropped on the floor. When the caller is gone by the time they arrive (cancelled, or the node stopped and left
    /// its dispatch loop behind), they are released.
    async fn claim(&self, init: &Initialized, limit: usize) -> Result<Vec<(Claim, Instant)>, StoreError> {
        let sent = Instant::now();
        let store = self.inner.store.clone();
        let types = init.types.clone();
        let node = self.inner.config.node_id.clone();
        let ttl = self.inner.config.lease_ttl;
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = store.claim_due(&types, &node, ttl, limit).await;
            // Unsent or never received, the batch releases its claims when dropped.
            let _ = tx.send(result.map(|claims| ClaimBatch { store, claims }));
        });
        let mut batch = rx.await.map_err(|_| StoreError::unavailable("claiming failed: the claim task ended"))??;
        Ok(std::mem::take(&mut batch.claims).into_iter().map(|c| (c, sent)).collect())
    }

    async fn dispatch_loop(&self, init: Arc<Initialized>, tracker: TaskTracker, cancel: CancellationToken) {
        let config = &self.inner.config;
        while !self.inner.stopping.load(Ordering::SeqCst) {
            let free = config.concurrency.saturating_sub(self.running_turns());
            let mut claimed = 0;
            let mut failed = false;
            if free > 0 {
                match self.claim(&init, free).await {
                    Ok(claims) => {
                        claimed = claims.len();
                        if self.inner.stopping.load(Ordering::SeqCst) {
                            // stop() began while claiming: hand the claims straight back, nothing is run any more.
                            for (c, _) in claims {
                                let _ = self.inner.store.release(&c.actor, c.epoch).await;
                            }
                            break;
                        }
                        for (claim, sent) in claims {
                            let executor = init.executor.clone();
                            let cancel = cancel.clone();
                            let guard = RunningGuard::new(self.inner.running.clone());
                            let wake = self.inner.wake.clone();
                            tracker.spawn(async move {
                                executor.execute(claim, sent, cancel).await;
                                drop(guard);
                                wake.notify_one();
                            });
                        }
                    }
                    Err(e) => {
                        failed = true;
                        tracing::warn!("Claiming failed; retrying after {:?}: {e}", config.poll_interval);
                    }
                }
            }
            if claimed > 0 && claimed == free {
                continue; // every free slot filled: more may be due
            }
            let pause = if free > 0 && !failed { self.sleep_time(&init).await } else { config.poll_interval };
            if !pause.is_zero() {
                tokio::select! {
                    biased;
                    _ = self.inner.wake.notified() => {}
                    _ = sleep(pause) => {}
                }
            }
        }
    }

    /// Time until the next known due message, bounded by the poll interval (with a small floor against spinning).
    async fn sleep_time(&self, init: &Initialized) -> Duration {
        let poll = self.inner.config.poll_interval;
        let store = &self.inner.store;
        match store.next_due_at(&init.types).await {
            Ok(None) => poll,
            Ok(Some(next)) => match store.now().await {
                Ok(now) => time::between(now, next).clamp(MIN_SLEEP, poll),
                Err(_) => poll,
            },
            Err(e) => {
                tracing::warn!("Reading the next due time failed: {e}");
                poll
            }
        }
    }

    // ---- operations ----

    fn hosted_for<'a>(&self, init: &'a Initialized, name: &TypeName) -> Option<&'a HostedType> {
        init.executor.hosted.get(name)
    }

    /// Creates an instance with its initial `state` (`PUT /actors/{type}/{key}`). Idempotent: an existing instance is
    /// left untouched.
    ///
    /// The type is resolved through the TypeRegistry. Any node refuses a state that is `null`; a node that hosts the
    /// type also decodes `state` with the state type and refuses it when that fails. The state is not checked against
    /// its schema; [`Self::create_actor_checked`] is this call with that check.
    pub async fn create_actor(&self, address: &ActorAddress, state: Value) -> Result<CreateActorResult, OpError> {
        self.create_actor_checked(address, state, &|_, _| Ok(())).await
    }

    /// [`Self::create_actor`] with `check`, which gets the state schema of the registered contract and `state`. It runs
    /// on every node, after the registered type and the `null` check, and on a node that hosts the type after the check
    /// of the address and before the decoder. Its refusal is [`CreateActorResult::InvalidState`], "State does not match
    /// the schema: " and its reason, and nothing is created. The HTTP door of `cicada-axum` calls this (ADR 0026).
    pub async fn create_actor_checked(
        &self,
        address: &ActorAddress,
        state: Value,
        check: SchemaCheck<'_>,
    ) -> Result<CreateActorResult, OpError> {
        let init = self.initialized().await?.clone();
        let Some(resolved) = self.inner.resolver.resolve(address).await? else {
            return Ok(CreateActorResult::NoSuchType);
        };
        limits::check_json("state", Some(&state)).map_err(OpError::Invalid)?;
        if let Err(e) = message::check_not_null("state", &state) {
            return Ok(CreateActorResult::InvalidState(e));
        }
        let hosted = self.hosted_for(&init, address.type_name());
        if let Some(problem) = hosted.and_then(|h| h.actor.address_problem(address)) {
            return Ok(CreateActorResult::InvalidAddress(problem));
        }
        if let Err(reason) = check(resolved.contract.state_schema(), &state) {
            return Ok(CreateActorResult::InvalidState(format!("State does not match the schema: {reason}")));
        }
        if let Some(Err(e)) = hosted.map(|h| h.actor.check_state(&state)) {
            return Ok(CreateActorResult::InvalidState(format!("State cannot be decoded: {e}")));
        }
        Ok(match self.inner.store.create(address, state).await? {
            CreateResult::Created => CreateActorResult::Created,
            CreateResult::Exists(existing) => CreateActorResult::Exists(existing),
            // The type was removed between the lookup and the insert; the store refused atomically.
            CreateResult::NoSuchType => CreateActorResult::NoSuchType,
        })
    }

    /// Address, state, version and parent of an instance (`GET /actors/{type}/{key}`).
    pub async fn get_actor(&self, address: &ActorAddress) -> Result<Option<ActorSnapshot>, OpError> {
        let Some(record) = self.inner.store.get(address).await? else {
            return Ok(None);
        };
        Ok(Some(ActorSnapshot {
            address: record.address,
            state: record.state,
            version: record.version,
            created_at: record.created_at,
            paused: record.paused,
            parent: record.parent,
        }))
    }

    /// Admin delete (`DELETE /actors/{type}/{key}`): removes the instance and every descendant (the actors its turns
    /// created with [`Turn::create_child`](crate::Turn::create_child), theirs, and so on) in one transaction, and purges
    /// their mailboxes. [`DeleteResult::Deleted`] counts the purged messages of the whole tree. The watchers of each
    /// removed actor get a [`Stopped`](crate::Stopped) in the same transaction, and the dispatch loop is woken for
    /// them.
    pub async fn delete_actor(&self, address: &ActorAddress) -> Result<DeleteResult, OpError> {
        let result = self.inner.store.delete(address, &MessageId::random()).await?;
        if matches!(result, DeleteResult::Deleted { .. }) {
            self.wake_up();
        }
        Ok(result)
    }

    /// Sets the watch of `watcher` on `target` from outside a turn (`PUT /watches`; ADR 0023). Idempotent. Refused when
    /// no registered type matches the watcher ([`WatchResult::NoSuchType`]), when the watcher does not exist
    /// ([`WatchResult::NoSuchInstance`]), and when its type's message family does not declare `Failed(cicada::Failed)`
    /// and `Stopped(cicada::Stopped)` ([`WatchResult::NotAWatcher`]). The target is not checked: it need not exist.
    pub async fn watch(&self, target: &WatchTarget, watcher: &ActorAddress) -> Result<WatchResult, OpError> {
        let Some(registered) = self.inner.resolver.resolve(watcher).await? else {
            return Ok(WatchResult::NoSuchType);
        };
        if self.inner.store.get(watcher).await?.is_none() {
            return Ok(WatchResult::NoSuchInstance);
        }
        if let Some(problem) = watch::watcher_problem(&registered.contract) {
            return Ok(WatchResult::NotAWatcher(problem));
        }
        Ok(match self.inner.store.watch(target, watcher).await? {
            SetWatchResult::Created => WatchResult::Created,
            SetWatchResult::Exists => WatchResult::Exists,
            SetWatchResult::NoSuchWatcher => WatchResult::NoSuchInstance,
        })
    }

    /// Removes the watch of `watcher` on `target` (`DELETE /watches`); false when it was not stored.
    pub async fn unwatch(&self, target: &WatchTarget, watcher: &ActorAddress) -> Result<bool, OpError> {
        Ok(self.inner.store.unwatch(target, watcher).await?)
    }

    /// The stored watches in the order of target and watcher, paged (`GET /watches`): of one `watcher`, of exactly one
    /// stored `target` (not every target that would match an actor), of both, or all.
    pub async fn watches(
        &self,
        watcher: Option<&ActorAddress>,
        target: Option<&WatchTarget>,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Page<Watch>, OpError> {
        Ok(self.inner.store.watches(watcher, target, cursor, limit).await?)
    }

    /// The direct children of `address` in address order, paged (`GET /actors/{type}/{key}/children`); `None` when
    /// the instance is absent. A client walks a tree level by level.
    pub async fn list_children(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Option<Page<ActorAddress>>, OpError> {
        if self.inner.store.get(address).await?.is_none() {
            return Ok(None);
        }
        Ok(Some(self.inner.store.children(address, cursor, limit).await?))
    }

    /// Instance URIs starting with `prefix`, paged (`GET /actors?prefix=`).
    pub async fn list_actors(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Page<ActorAddress>, OpError> {
        Ok(self.inner.store.list(prefix, cursor, limit).await?)
    }

    /// Sends a message (`POST /actors/{type}/{key}/messages`). Idempotent on the id within the retention window.
    ///
    /// Checks, in order: the id was not already accepted (then [`SendResult::Duplicate`] for the same target and type,
    /// whatever else changed, and [`SendResult::IdConflict`] for a different message); a registered type matches `to`
    /// (else [`SendResult::NoSuchType`], whether or not the instance exists); it accepts `message_type`; when this node
    /// hosts the type, `payload` decodes. Then the store enqueues it, refusing an absent instance
    /// ([`SendResult::NoSuchInstance`]). Fields beyond the send limits and delivery times out of range are
    /// [`OpError::Invalid`]. The payload is not checked against its schema; [`Self::send_message_checked`] is this call
    /// with that check.
    pub async fn send_message(
        &self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: SendOptions,
    ) -> Result<SendResult, OpError> {
        self.send_message_checked(to, message_type, payload, options, &|_, _| Ok(())).await
    }

    /// [`Self::send_message`] with `check`, which gets the registered schema of `message_type` and `payload`. It runs on
    /// every node, after the repeated id, the registered type and the message type, and on a node that hosts the type
    /// before the decoder. Its refusal is [`SendResult::InvalidPayload`], "Payload of '`message_type`' does not match
    /// the schema: " and its reason, and nothing is stored. The HTTP door of `cicada-axum` calls this (ADR 0026).
    pub async fn send_message_checked(
        &self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: SendOptions,
        check: SchemaCheck<'_>,
    ) -> Result<SendResult, OpError> {
        let init = self.initialized().await?.clone();
        let id = options.id.clone().unwrap_or_else(MessageId::random);
        limits::check(&id, message_type, options.correlation_id.as_deref(), &options.headers)
            .map_err(OpError::Invalid)?;
        limits::check_json("payload", Some(&payload)).map_err(OpError::Invalid)?;
        crate::turn::check_delivery(&options.delivery).map_err(|e| OpError::Invalid(e.to_string()))?;
        // Idempotency first: a send repeating an accepted id gets the same answer even when it would be refused now.
        if let Some(existing) = self.existing_envelope(&id).await? {
            return Ok(duplicate_of(Some(existing), to, message_type));
        }
        let Some(resolved) = self.inner.resolver.resolve(to).await? else {
            return Ok(SendResult::NoSuchType);
        };
        let name = to.type_name();
        let Some(schema) = resolved.contract.message_schemas().get(message_type) else {
            return Ok(SendResult::UnknownType { message_type: message_type.to_owned(), type_name: name.clone() });
        };
        if let Err(reason) = check(schema, &payload) {
            let error = format!("Payload of '{message_type}' does not match the schema: {reason}");
            return Ok(SendResult::InvalidPayload { message_type: message_type.to_owned(), error });
        }
        let hosted = self.hosted_for(&init, name).is_some();
        if let Some(h) = self.hosted_for(&init, name)
            && let Err(e) = h.actor.check_payload(message_type, &payload)
        {
            return Ok(SendResult::InvalidPayload { message_type: message_type.to_owned(), error: e });
        }
        let outgoing = OutgoingMessage {
            to: to.clone(),
            message_type: message_type.to_owned(),
            payload,
            id: id.clone(),
            from: None,
            delivery: options.delivery,
            correlation_id: options.correlation_id,
            headers: options.headers,
            retry: options.retry,
            caused_by: None,
            retention: options.retention,
        };
        Ok(match self.inner.store.enqueue(outgoing).await? {
            EnqueueResult::Enqueued(message) => {
                if hosted {
                    self.wake_up();
                }
                SendResult::Accepted(message)
            }
            EnqueueResult::Duplicate(id) => duplicate_of(self.existing_envelope(&id).await?, to, message_type),
            EnqueueResult::NoSuchInstance => SendResult::NoSuchInstance,
        })
    }

    async fn existing_envelope(&self, id: &MessageId) -> Result<Option<Message>, StoreError> {
        Ok(self.inner.store.status(id).await?.map(|s| s.envelope().clone()))
    }

    /// The pending mailbox in delivery order, or `None` when the instance is absent. [`Self::mailbox`] adds which message
    /// is running.
    pub async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Option<Page<Message>>, OpError> {
        if self.inner.store.get(address).await?.is_none() {
            return Ok(None);
        }
        Ok(Some(self.inner.store.pending(address, cursor, limit).await?))
    }

    /// The mailbox of `address` in delivery order, each message marked running or pending (`GET
    /// /actors/{type}/{key}/mailbox`); `None` when the instance is absent. Marking is approximate: the turn is read
    /// separately from the page.
    pub async fn mailbox(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Option<Page<MailboxEntry>>, OpError> {
        let Some(page) = self.pending(address, cursor, limit).await? else {
            return Ok(None);
        };
        let running = self.inner.store.running_turn(address).await?.map(|t| t.message_id);
        let items = page
            .items
            .into_iter()
            .map(|message| MailboxEntry { running: running.as_ref() == Some(&message.id), message })
            .collect();
        Ok(Some(Page::new(items, page.next)))
    }

    /// Pauses the mailbox of `address` (`POST /actors/{type}/{key}/mailbox/pause`): messages still queue up, no new turn
    /// is claimed, a running turn finishes. False when the instance is absent.
    pub async fn pause_mailbox(&self, address: &ActorAddress) -> Result<bool, OpError> {
        Ok(self.inner.store.set_paused(address, true).await?)
    }

    /// Resumes the mailbox of `address` (`POST /actors/{type}/{key}/mailbox/resume`) and wakes the dispatch loop. False
    /// when the instance is absent.
    pub async fn resume_mailbox(&self, address: &ActorAddress) -> Result<bool, OpError> {
        let found = self.inner.store.set_paused(address, false).await?;
        if found {
            self.wake_up();
        }
        Ok(found)
    }

    /// Gives outcome `purged` to every pending message of `address` except one being handled, and keeps the instance
    /// (`POST /actors/{type}/{key}/mailbox/purge`). `None` when the instance is absent.
    pub async fn purge_mailbox(&self, address: &ActorAddress) -> Result<Option<u64>, OpError> {
        Ok(self.inner.store.purge(address).await?)
    }

    /// The pending messages of every instance whose URI starts with `prefix` (`GET /messages?prefix=`).
    pub async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Page<Message>, OpError> {
        Ok(self.inner.store.list_pending(prefix, cursor, limit).await?)
    }

    /// Status of a message (`GET /messages/{id}`): pending, running or finished; `None` when unknown or expired.
    pub async fn status(&self, id: &MessageId) -> Result<Option<MessageStatus>, OpError> {
        Ok(self.inner.store.status(id).await?)
    }

    /// The messages sent by the turn that handled `id`, one level down: pending ones and retained outcomes, in message-id
    /// order (`GET /messages/{id}/caused`). `None` when `id` itself is unknown or past retention.
    pub async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Option<Page<MessageStatus>>, OpError> {
        if self.inner.store.status(id).await?.is_none() {
            return Ok(None);
        }
        Ok(Some(self.inner.store.caused_by(id, cursor, limit).await?))
    }

    /// The retained outcome of a message.
    pub async fn outcome(&self, id: &MessageId) -> Result<Option<OutcomeRecord>, OpError> {
        Ok(match self.inner.store.status(id).await? {
            Some(MessageStatus::Finished(outcome)) => Some(outcome),
            _ => None,
        })
    }

    /// Admin cancel of a pending message (`POST /messages/{id}/cancel`).
    pub async fn cancel(&self, id: &MessageId) -> Result<CancelResult, OpError> {
        Ok(self.inner.store.cancel(id).await?)
    }

    /// Makes a pending, not running message due at store now with a new `seq` (`POST /messages/{id}/deliver-now`) and
    /// wakes the dispatch loop.
    pub async fn deliver_now(&self, id: &MessageId) -> Result<DeliverNowResult, OpError> {
        let result = self.inner.store.deliver_now(id).await?;
        if matches!(result, DeliverNowResult::Delivered(_)) {
            self.wake_up();
        }
        Ok(result)
    }

    /// Sends a message that ended `failed` or `cancelled` again (`POST /messages/{id}/retry`): a new message with a new
    /// id and `caused_by` the original, otherwise equal in target, type, payload, correlation id, headers, retry and
    /// retention. A retry is not idempotent: every call sends another message.
    pub async fn retry_message(&self, id: &MessageId) -> Result<RetryResult, OpError> {
        let original = match self.inner.store.status(id).await? {
            None => return Ok(RetryResult::NotFound),
            Some(MessageStatus::Pending(_)) => return Ok(RetryResult::NotRetryable { state: "pending".to_owned() }),
            Some(MessageStatus::Running { .. }) => {
                return Ok(RetryResult::NotRetryable { state: "running".to_owned() });
            }
            Some(MessageStatus::Finished(o)) => match o.status {
                OutcomeStatus::Failed | OutcomeStatus::Cancelled => o.envelope,
                status => return Ok(RetryResult::NotRetryable { state: status.wire_name().to_owned() }),
            },
        };
        let to = original.to.clone();
        let outgoing = OutgoingMessage {
            correlation_id: original.correlation_id,
            headers: original.headers,
            retry: original.retry,
            retention: original.retention,
            caused_by: Some(original.id),
            ..OutgoingMessage::new(original.to, original.message_type, original.payload)
        };
        Ok(match self.inner.store.enqueue(outgoing).await? {
            EnqueueResult::Enqueued(message) => {
                self.wake_up();
                RetryResult::Accepted(message)
            }
            EnqueueResult::NoSuchInstance => RetryResult::NoSuchInstance(to),
            EnqueueResult::Duplicate(id) => {
                return Err(OpError::Store(StoreError::Corrupt(format!("the new message id {id} is already in use"))));
            }
        })
    }

    /// Cancels at most [`BULK_LIMIT`] messages (`POST /messages/bulk-cancel`), each as [`Self::cancel`] does; ones that
    /// are not pending are skipped and not counted. Not one transaction: a store failure half way reports it as an
    /// error, with the work done so far kept. Paging is the listing's keyset, not a snapshot.
    pub async fn bulk_cancel(&self, selection: BulkSelection) -> Result<BulkResult, OpError> {
        match selection {
            BulkSelection::Ids(ids) => {
                check_bulk_ids(&ids)?;
                let mut affected = 0;
                for id in &ids {
                    if matches!(self.cancel(id).await?, CancelResult::Cancelled(_)) {
                        affected += 1;
                    }
                }
                Ok(BulkResult { affected, more: false })
            }
            BulkSelection::Filter { prefix, message_type } => {
                let mut affected = 0;
                let mut cursor = None;
                loop {
                    let page = self.list_pending(&prefix, cursor.as_ref(), BULK_LIMIT).await?;
                    for message in page.items.iter().filter(|m| of_type(&message_type, &m.message_type)) {
                        if affected < BULK_LIMIT as u64 {
                            if matches!(self.cancel(&message.id).await?, CancelResult::Cancelled(_)) {
                                affected += 1;
                            }
                        } else if matches!(self.inner.store.status(&message.id).await?, Some(MessageStatus::Pending(_)))
                        {
                            return Ok(BulkResult { affected, more: true });
                        }
                    }
                    match page.next {
                        Some(next) => cursor = Some(next),
                        None => return Ok(BulkResult { affected, more: false }),
                    }
                }
            }
        }
    }

    /// Retries at most [`BULK_LIMIT`] messages (`POST /messages/bulk-retry`), each as [`Self::retry_message`] does; ones
    /// that are not eligible are skipped and not counted. With a filter it walks the `failed` outcomes and also skips one
    /// already retried (it has a `caused_by` child), so that calling again until `more` is false does not send a message
    /// twice. Not one transaction: a store failure half way reports it as an error, with the work done so far kept.
    pub async fn bulk_retry(&self, selection: BulkSelection) -> Result<BulkResult, OpError> {
        match selection {
            BulkSelection::Ids(ids) => {
                check_bulk_ids(&ids)?;
                let mut affected = 0;
                for id in &ids {
                    if matches!(self.retry_message(id).await?, RetryResult::Accepted(_)) {
                        affected += 1;
                    }
                }
                Ok(BulkResult { affected, more: false })
            }
            BulkSelection::Filter { prefix, message_type } => {
                let mut affected = 0;
                let mut cursor = None;
                loop {
                    let page =
                        self.outcomes(Some(OutcomeStatus::Failed), &prefix, None, cursor.as_ref(), BULK_LIMIT).await?;
                    for outcome in page.items.iter().filter(|o| of_type(&message_type, &o.envelope.message_type)) {
                        let retried = !self.inner.store.caused_by(&outcome.message_id, None, 1).await?.items.is_empty();
                        if retried {
                            continue;
                        }
                        if affected < BULK_LIMIT as u64 {
                            if matches!(self.retry_message(&outcome.message_id).await?, RetryResult::Accepted(_)) {
                                affected += 1;
                            }
                        } else {
                            return Ok(BulkResult { affected, more: true });
                        }
                    }
                    match page.next {
                        Some(next) => cursor = Some(next),
                        None => return Ok(BulkResult { affected, more: false }),
                    }
                }
            }
        }
    }

    /// Finished messages with envelope and error (`GET /outcomes?status=&prefix=&since=`), of one `status` or, with
    /// `None`, of every status.
    pub async fn outcomes(
        &self,
        status: Option<OutcomeStatus>,
        prefix: &str,
        since: Option<Timestamp>,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Page<OutcomeRecord>, OpError> {
        let since = since.unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);
        if since > MAX_INSTANT {
            return Err(OpError::Invalid(format!(
                "since must not be later than {}, was {}",
                time::format_instant(MAX_INSTANT),
                time::format_instant(since)
            )));
        }
        Ok(self.inner.store.outcomes(status, prefix, since, cursor, limit).await?)
    }

    /// The number of actors and pending messages under `prefix`, and the delivery time of the oldest due message
    /// (`GET /stats?prefix=`).
    pub async fn stats(&self, prefix: &str) -> Result<StoreStats, OpError> {
        Ok(self.inner.store.stats(prefix).await?)
    }

    /// All registered types with their contracts (`GET /types`), ordered by name.
    pub async fn types(&self) -> Result<Vec<RegisteredType>, OpError> {
        let defs = self.inner.store.list_types().await?;
        let mut types = defs.into_iter().map(|d| self.inner.resolver.registered(d)).collect::<Result<Vec<_>, _>>()?;
        types.sort_by(|a, b| a.name().cmp(b.name()));
        Ok(types)
    }

    /// Removes the registration of the type `name` (`DELETE /types/{name}`), unless actors of the type exist. A node that
    /// still hosts the type registers it again at its next start.
    pub async fn remove_type(&self, name: &TypeName) -> Result<RemoveTypeResult, OpError> {
        Ok(self.inner.store.remove_type(name).await?)
    }

    /// Every node in the node table (`GET /nodes`), by id, with its status from store time and its running turns.
    /// Display only: leases, not this table, decide who runs what.
    pub async fn nodes(&self) -> Result<Vec<NodeInfo>, OpError> {
        let records = self.inner.store.list_nodes().await?;
        let now = self.inner.store.now().await?;
        let mut turns: HashMap<NodeId, Vec<RunningTurn>> = HashMap::new();
        for turn in self.all_running_turns().await? {
            turns.entry(turn.node.clone()).or_default().push(turn);
        }
        Ok(records
            .into_iter()
            .map(|r| {
                let age = time::between(r.last_seen, now);
                let status = if age < NODE_ALIVE_FOR {
                    NodeHealth::Alive
                } else if age < NODE_SUSPECT_FOR {
                    NodeHealth::Suspect
                } else {
                    NodeHealth::Dead
                };
                let running_turns = turns.remove(&r.id).unwrap_or_default();
                NodeInfo {
                    id: r.id,
                    hosted: r.hosted,
                    started_at: r.started_at,
                    last_seen: r.last_seen,
                    status,
                    running_turns,
                }
            })
            .collect())
    }

    /// One node as [`Self::nodes`] reports it (`GET /nodes/{node}`), or `None`.
    pub async fn node(&self, id: &NodeId) -> Result<Option<NodeInfo>, OpError> {
        Ok(self.nodes().await?.into_iter().find(|n| &n.id == id))
    }

    async fn all_running_turns(&self) -> Result<Vec<RunningTurn>, OpError> {
        let mut turns = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.inner.store.running_turns(None, "", cursor.as_ref(), PAGE_OF_TURNS).await?;
            turns.extend(page.items);
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(turns),
            }
        }
    }

    /// The turns running now, with a valid lease, of `node` or of any node, under the address `prefix` (`GET
    /// /turns?node=&prefix=`), in address order. A snapshot of running work, approximate by nature.
    pub async fn list_running_turns(
        &self,
        node: Option<&NodeId>,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> Result<Page<RunningTurn>, OpError> {
        Ok(self.inner.store.running_turns(node, prefix, cursor, limit).await?)
    }

    /// The turn running now on `address` (`GET /actors/{type}/{key}/turn`), or `None`.
    pub async fn running_turn(&self, address: &ActorAddress) -> Result<Option<RunningTurn>, OpError> {
        Ok(self.inner.store.running_turn(address).await?)
    }

    /// Asks the node running the turn on `address` to cancel it (`POST /actors/{type}/{key}/turn/cancel`); it does so
    /// within one heartbeat. False when no turn is running. A turn that ends first is not affected.
    pub async fn cancel_turn(&self, address: &ActorAddress) -> Result<bool, OpError> {
        Ok(self.inner.store.request_cancel(address).await?)
    }

    /// The store's time: the one clock of every node (S9).
    pub async fn now(&self) -> Result<Timestamp, OpError> {
        Ok(self.inner.store.now().await?)
    }

    /// The earliest store time at which an instance of a type this node hosts may become claimable: the delivery time
    /// of the head of its mailbox, or the end of its lease when that is later. `None` when no message is pending.
    /// Paused instances do not count.
    pub async fn next_due_at(&self) -> Result<Option<Timestamp>, OpError> {
        let types = self.hosted_types();
        if types.is_empty() {
            return Ok(None);
        }
        Ok(self.inner.store.next_due_at(&types).await?)
    }

    /// True when the node is running (or usable) and the store is reachable (`GET /health`).
    pub async fn health(&self) -> bool {
        !matches!(self.state(), NodeState::Stopping | NodeState::Stopped) && self.inner.store.now().await.is_ok()
    }

    // ---- typed conveniences ----

    /// Creates `r` with `initial_state`; see [`Self::create_actor`].
    pub async fn create<R: ActorRef, S: serde::Serialize + ?Sized>(
        &self,
        r: &R,
        initial_state: &S,
    ) -> Result<CreateActorResult, OpError> {
        let address = r.address().map_err(|e| OpError::Invalid(e.to_string()))?;
        let state = serde_json::to_value(initial_state)
            .map_err(|e| OpError::Invalid(format!("The initial state cannot be encoded: {e}")))?;
        self.create_actor(&address, state).await
    }

    /// Sends `msg` to `to`, due now; the compiler checks that `to` accepts it. See [`Self::send_message`].
    pub async fn send<R: ActorRef>(&self, to: &R, msg: impl Into<R::Message>) -> Result<SendResult, OpError> {
        self.send_with(to, msg, SendOptions::default()).await
    }

    /// Sends `msg` to `to` with `options`. See [`Self::send_message`].
    pub async fn send_with<R: ActorRef>(
        &self,
        to: &R,
        msg: impl Into<R::Message>,
        options: SendOptions,
    ) -> Result<SendResult, OpError> {
        let address = to.address().map_err(|e| OpError::Invalid(e.to_string()))?;
        let encoded = message::encode(&msg.into()).map_err(|e| OpError::Invalid(e.to_string()))?;
        self.send_message(&address, &encoded.message_type, encoded.payload, options).await
    }

    /// Sends a message of any family to a raw address. See [`Self::send_message`].
    pub async fn send_to<M: MessageFamily>(
        &self,
        to: &ActorAddress,
        msg: &M,
        options: SendOptions,
    ) -> Result<SendResult, OpError> {
        let encoded = message::encode(msg).map_err(|e| OpError::Invalid(e.to_string()))?;
        self.send_message(to, &encoded.message_type, encoded.payload, options).await
    }
}

fn check_bulk_ids(ids: &[MessageId]) -> Result<(), OpError> {
    if ids.len() > BULK_LIMIT {
        return Err(OpError::Invalid(format!("At most {BULK_LIMIT} ids per call, got {}", ids.len())));
    }
    Ok(())
}

/// True when `actual` is the wanted message type, or none is wanted.
fn of_type(wanted: &Option<String>, actual: &str) -> bool {
    wanted.as_deref().is_none_or(|w| w == actual)
}

/// A repeated id: the same send again, or a different message that happens to reuse the id.
fn duplicate_of(existing: Option<Message>, to: &ActorAddress, message_type: &str) -> SendResult {
    match existing {
        Some(m) if &m.to != to || m.message_type != message_type => SendResult::IdConflict(m),
        other => SendResult::Duplicate(other),
    }
}

/// Claims on their way from the store to the caller. Claims still in the batch when it is dropped (the caller was gone
/// when they arrived) are released in the background, so their messages are claimable again at once.
struct ClaimBatch {
    store: Store,
    claims: Vec<Claim>,
}

impl Drop for ClaimBatch {
    fn drop(&mut self) {
        let claims = std::mem::take(&mut self.claims);
        if claims.is_empty() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("{} claims dropped outside a runtime; their leases expire", claims.len());
            return;
        };
        let store = self.store.clone();
        runtime.spawn(async move {
            for c in claims {
                tracing::info!("Releasing the claim of {} for {}: nobody takes it any more", c.actor, c.message.id);
                let _ = store.release(&c.actor, c.epoch).await;
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
    check::<ActorSystem>();
}
