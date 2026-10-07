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
use super::resolver::{RegisteredType, ResolvedActorType, TypeResolver};
use super::results::{ActorSnapshot, CreateActorResult, MessageChain, OpError, SendResult};
use crate::actor::{Actor, ActorType, HostedActor};
use crate::address::{ActorAddress, ActorRef, UriTemplate};
use crate::message::{self, MessageFamily};
use crate::plugin::{Plugin, reserved_headers};
use crate::store::{
    CancelResult, ChainEntry, Claim, CreateResult, DeleteResult, EnqueueResult, Message, MessageId, MessageStatus,
    NodeId, OutcomeRecord, OutgoingMessage, Page, PageCursor, RegisterResult, Store, StoreError,
};
use crate::time::{self, MAX_INSTANT, Timestamp};
use crate::turn::SendOptions;

const MIN_SLEEP: Duration = Duration::from_millis(1);

/// A node could not start: an invalid actor type, an invalid configuration, or a contract conflict in the
/// TypeRegistry.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Actor system cannot start:\n{}", .problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n"))]
pub struct StartError {
    /// Every problem found.
    pub problems: Vec<String>,
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

/// Configuration of one node. Every value has a default; outcome retention is the store's.
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
        problems
    }
}

/// Builds an [`ActorSystem`].
pub struct ActorSystemBuilder {
    store: Arc<dyn Store>,
    config: NodeConfig,
    hosted: Vec<Arc<dyn HostedActor>>,
    plugins: Vec<Arc<dyn Plugin>>,
}

impl ActorSystemBuilder {
    /// Hosts an actor type on this node: it registers its contract and runs its turns.
    pub fn host<A: Actor>(mut self, actor_type: impl Into<ActorType<A>>) -> Self {
        self.hosted.push(actor_type.into().into_hosted());
        self
    }

    /// Installs `plugin` on this node, for every type it hosts.
    pub fn install(mut self, plugin: impl Plugin) -> Self {
        self.plugins.push(Arc::new(plugin));
        self
    }

    /// Installs a shared plugin on this node.
    pub fn install_shared(mut self, plugin: Arc<dyn Plugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Replaces the whole configuration.
    pub fn config(mut self, config: NodeConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets this node's identity.
    pub fn node_id(mut self, id: NodeId) -> Self {
        self.config.node_id = id;
        self
    }

    /// Sets the lease TTL.
    pub fn lease_ttl(mut self, ttl: Duration) -> Self {
        self.config.lease_ttl = ttl;
        self
    }

    /// Sets the heartbeat interval.
    pub fn heartbeat_interval(mut self, interval: Duration) -> Self {
        self.config.heartbeat_interval = Some(interval);
        self
    }

    /// Sets the poll interval.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.config.poll_interval = interval;
        self
    }

    /// Sets the default maximum turn duration.
    pub fn default_max_turn(mut self, max_turn: Duration) -> Self {
        self.config.default_max_turn = max_turn;
        self
    }

    /// Sets the turn concurrency.
    pub fn concurrency(mut self, concurrency: usize) -> Self {
        self.config.concurrency = concurrency;
        self
    }

    /// Sets the shutdown grace period.
    pub fn shutdown_grace_period(mut self, grace: Duration) -> Self {
        self.config.shutdown_grace_period = grace;
        self
    }

    /// Sets the shutdown cancel timeout.
    pub fn shutdown_cancel_timeout(mut self, timeout: Duration) -> Self {
        self.config.shutdown_cancel_timeout = timeout;
        self
    }

    /// Sets the sweep interval.
    pub fn sweep_interval(mut self, interval: Duration) -> Self {
        self.config.sweep_interval = interval;
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
                plugins: self.plugins,
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
    templates: BTreeSet<UriTemplate>,
}

#[derive(Default)]
struct Lifecycle {
    tracker: TaskTracker,
    cancel_turns: CancellationToken,
    dispatch: Option<JoinHandle<()>>,
    background: Vec<JoinHandle<()>>,
}

struct Inner {
    store: Arc<dyn Store>,
    config: NodeConfig,
    hosted: Vec<Arc<dyn HostedActor>>,
    plugins: Vec<Arc<dyn Plugin>>,
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
/// # async fn example(store: std::sync::Arc<dyn cicada::store::Store>) -> Result<(), Box<dyn std::error::Error>> {
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
/// period, then cancels the rest and releases their leases. The store is owned by the caller and is not closed.
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
    pub fn builder(store: Arc<dyn Store>) -> ActorSystemBuilder {
        ActorSystemBuilder { store, config: NodeConfig::default(), hosted: Vec::new(), plugins: Vec::new() }
    }

    /// The shared store.
    pub fn store(&self) -> &Arc<dyn Store> {
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

    /// The templates of the hosted types (valid types only).
    pub fn hosted_templates(&self) -> BTreeSet<UriTemplate> {
        self.inner.hosted.iter().filter_map(|h| h.template().cloned()).collect()
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
        let mut problems = inner.config.validate();
        let mut names: Vec<&str> = inner.plugins.iter().map(|p| p.name()).collect();
        names.sort_unstable();
        for pair in names.windows(2) {
            if pair[0] == pair[1] {
                problems.push(format!("Plugin {} is installed more than once on the node", pair[0]));
            }
        }
        for hosted in &inner.hosted {
            problems.extend(hosted.problems().into_iter().map(|p| format!("{}: {p}", hosted.name())));
        }
        if !problems.is_empty() {
            return Err(StartError { problems });
        }
        let mut types: HashMap<UriTemplate, HostedType> = HashMap::new();
        let mut order: Vec<UriTemplate> = Vec::new();
        for hosted in &inner.hosted {
            let template = hosted.template().expect("a valid type has a template").clone();
            if types.contains_key(&template) {
                problems.push(format!("{}: template {template} is hosted twice", hosted.name()));
                continue;
            }
            if let Some(other) = order.iter().find(|t| t.overlaps(&template)) {
                let common = template.common_address(other).map(|a| a.to_string()).unwrap_or_default();
                problems.push(format!("{}: template {template} overlaps {other}: both match {common}", hosted.name()));
                continue;
            }
            // A type installation replaces the node installation of the same plugin.
            let type_plugins = hosted.plugins();
            let mut plugins: Vec<Arc<dyn Plugin>> = inner
                .plugins
                .iter()
                .filter(|node| type_plugins.iter().all(|t| t.name() != node.name()))
                .cloned()
                .collect();
            plugins.extend(type_plugins.iter().cloned());
            let max_turn = hosted.max_turn().unwrap_or(inner.config.default_max_turn);
            order.push(template.clone());
            types.insert(template, HostedType { actor: hosted.clone(), plugins, max_turn });
        }
        if !problems.is_empty() {
            return Err(StartError { problems });
        }
        for template in &order {
            let hosted = &types[template];
            let def = hosted.actor.contract().expect("a valid type has a contract").to_type_def();
            match inner.store.register(def.clone()).await {
                Ok(RegisterResult::Registered(_)) => {}
                Ok(RegisterResult::Conflict { existing, reason }) => problems.push(format!(
                    "{}: {reason} (registered {} {}, this node {})",
                    hosted.actor.name(),
                    existing.template,
                    existing.contract_hash,
                    def.contract_hash
                )),
                Err(e) => problems.push(format!("{}: registering the contract failed: {e}", hosted.actor.name())),
            }
        }
        if !problems.is_empty() {
            return Err(StartError { problems });
        }
        let lease = LeaseSettings { ttl: inner.config.lease_ttl, heartbeat: inner.config.heartbeat() };
        let executor = Arc::new(TurnExecutor {
            store: inner.store.clone(),
            resolver: inner.resolver.clone(),
            hosted: types,
            lease,
            wake: inner.wake.clone(),
        });
        Ok(Arc::new(Initialized { executor, templates: order.into_iter().collect() }))
    }

    /// Initializes (see [`Self::initialize`]) and starts the dispatch loop, the retention sweep and, when the store
    /// offers them, the collection of its wake-ups. Fails when the node was already started or stopped.
    pub async fn start(&self) -> Result<(), StartError> {
        let mut lifecycle = self.inner.lifecycle.lock().await;
        if self.state() != NodeState::New {
            return Err(StartError { problems: vec![format!("Actor system is {}", self.state())] });
        }
        let init = self.initialized().await?.clone();
        self.inner.stopping.store(false, Ordering::SeqCst);
        if !init.templates.is_empty() {
            let this = self.clone();
            let tracker = lifecycle.tracker.clone();
            let cancel = lifecycle.cancel_turns.clone();
            let loop_init = init.clone();
            lifecycle.dispatch = Some(tokio::spawn(async move { this.dispatch_loop(loop_init, tracker, cancel).await }));
            if let Some(mut wake_ups) = self.inner.store.wake_ups(&init.templates) {
                let wake = self.inner.wake.clone();
                lifecycle.background.push(tokio::spawn(async move {
                    while wake_ups.next().await.is_some() {
                        wake.notify_one();
                    }
                    tracing::warn!("Store wake-ups ended; falling back to polling");
                }));
            }
        }
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
            init.templates.iter().map(|t| t.as_str()).collect::<Vec<_>>()
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
        self.inner.state.store(NodeState::Stopped as u8, Ordering::SeqCst);
        tracing::info!("Actors node {} stopped", self.inner.config.node_id);
    }

    /// Wakes the dispatch loop so it claims now instead of after its sleep. Spurious calls are harmless.
    pub fn wake_up(&self) {
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
        if init.templates.is_empty() {
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
        let templates = init.templates.clone();
        let node = self.inner.config.node_id.clone();
        let ttl = self.inner.config.lease_ttl;
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = store.claim_due(&templates, &node, ttl, limit).await;
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
        match store.next_due_at(&init.templates).await {
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

    fn hosted_for<'a>(&self, init: &'a Initialized, template: &UriTemplate) -> Option<&'a HostedType> {
        init.executor.hosted.get(template)
    }

    /// Creates an instance (`PUT /actors/{path}`). Idempotent: an existing instance is left untouched.
    ///
    /// The type is resolved through the TypeRegistry. A node that hosts the type decodes `state` with the state type
    /// and refuses it when that fails; any node refuses a state for a stateless type.
    pub async fn create_actor(&self, address: &ActorAddress, state: Option<Value>) -> Result<CreateActorResult, OpError> {
        let init = self.initialized().await?.clone();
        let Some(resolved) = self.inner.resolver.resolve(address).await? else {
            return Ok(CreateActorResult::NoSuchType);
        };
        let template = resolved.registered.template().clone();
        let given = state.filter(|s| !s.is_null());
        limits::check_json("state", given.as_ref()).map_err(OpError::Invalid)?;
        if resolved.registered.contract.state_schema().is_none() && given.is_some() {
            return Ok(CreateActorResult::InvalidState(format!("{template} is stateless and takes no state")));
        }
        if let Some(hosted) = self.hosted_for(&init, &template) {
            if let Some(problem) = hosted.actor.canonical_problem(address) {
                return Ok(CreateActorResult::InvalidAddress(problem));
            }
            if let Err(e) = hosted.actor.check_state(given.as_ref()) {
                return Ok(CreateActorResult::InvalidState(format!("State cannot be decoded: {e}")));
            }
        }
        Ok(match self.inner.store.create(address, &template, given).await? {
            CreateResult::Created => CreateActorResult::Created,
            CreateResult::Exists(existing) => CreateActorResult::Exists(existing),
        })
    }

    /// Type, params, state and version of an instance (`GET /actors/{path}`).
    pub async fn get_actor(&self, address: &ActorAddress) -> Result<Option<ActorSnapshot>, OpError> {
        Ok(self.inner.store.get(address).await?.map(|r| ActorSnapshot {
            params: r.template.matches(&r.address).unwrap_or_default(),
            address: r.address,
            template: r.template,
            state: r.state,
            version: r.version,
            created_at: r.created_at,
        }))
    }

    /// Admin delete (`DELETE /actors/{path}`): removes the instance and purges its mailbox.
    pub async fn delete_actor(&self, address: &ActorAddress) -> Result<DeleteResult, OpError> {
        Ok(self.inner.store.delete(address).await?)
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

    /// Sends a message (`POST /actors/{path}/messages`). Idempotent on the id within the retention window.
    ///
    /// Checks, in order: the id was not already accepted (then [`SendResult::Duplicate`] for the same target and type,
    /// whatever else changed, and [`SendResult::IdConflict`] for a different message); no reserved header; a registered
    /// type matches `to`; it accepts `message_type`; when this node hosts the type, `payload` decodes. Then the store
    /// enqueues it, refusing an absent instance. Fields beyond the send limits and delivery times out of range are
    /// [`OpError::Invalid`].
    pub async fn send_message(
        &self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: SendOptions,
    ) -> Result<SendResult, OpError> {
        let init = self.initialized().await?.clone();
        let id = options.id.clone().unwrap_or_else(MessageId::random);
        limits::check(&id, message_type, options.correlation_id.as_deref(), &options.headers).map_err(OpError::Invalid)?;
        limits::check_json("payload", Some(&payload)).map_err(OpError::Invalid)?;
        crate::turn::check_delivery(&options.delivery).map_err(|e| OpError::Invalid(e.to_string()))?;
        // Idempotency first: a send repeating an accepted id gets the same answer even when it would be refused now.
        if let Some(existing) = self.existing_envelope(&id).await? {
            return Ok(duplicate_of(Some(existing), to, message_type));
        }
        if let Some(name) = options.headers.keys().find(|n| reserved_headers::is_reserved(n)) {
            return Ok(SendResult::InvalidHeaders(format!(
                "Header '{name}' uses the reserved prefix '{}'",
                reserved_headers::PREFIX
            )));
        }
        let Some(resolved) = self.inner.resolver.resolve(to).await? else {
            return Ok(SendResult::NoSuchInstance);
        };
        let template = resolved.registered.template().clone();
        if !resolved.registered.contract.accepts(message_type) {
            return Ok(SendResult::UnknownType { message_type: message_type.to_owned(), template });
        }
        let hosted = self.hosted_for(&init, &template).is_some();
        if let Some(h) = self.hosted_for(&init, &template)
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

    /// The pending mailbox in delivery order (`GET /actors/{path}/messages`), or `None` when the instance is absent.
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

    /// The retained outcome of a message.
    pub async fn outcome(&self, id: &MessageId) -> Result<Option<OutcomeRecord>, OpError> {
        Ok(self.inner.store.outcome(id).await?)
    }

    /// Admin cancel of a pending message (`DELETE /messages/{id}`).
    pub async fn cancel(&self, id: &MessageId) -> Result<CancelResult, OpError> {
        Ok(self.inner.store.cancel(id).await?)
    }

    /// The chain of message `id` (`GET /messages/{id}/chain`): the messages that led to it, following `caused_by`
    /// upwards, and the messages it caused, directly or not, breadth first, both bounded. `None` when `id` is unknown
    /// or past retention.
    pub async fn chain(&self, id: &MessageId) -> Result<Option<MessageChain>, OpError> {
        let store = &self.inner.store;
        let Some(focus) = store.chain_entries(std::slice::from_ref(id)).await?.into_iter().next() else {
            return Ok(None);
        };
        let focus_id = focus.id().clone();
        let mut order: Vec<MessageId> = vec![focus_id.clone()];
        let mut entries: HashMap<MessageId, ChainEntry> = HashMap::new();
        let mut cause = focus.caused_by().cloned();
        entries.insert(focus_id.clone(), focus);
        let mut truncated = false;
        // Up: what led to it, one cause at a time.
        let mut steps = 0;
        while let Some(parent_id) = cause.take() {
            if entries.contains_key(&parent_id) {
                break;
            }
            if steps == MessageChain::MAX_DEPTH {
                truncated = true;
                break;
            }
            let Some(parent) = store.chain_entries(std::slice::from_ref(&parent_id)).await?.into_iter().next() else {
                break;
            };
            cause = parent.caused_by().cloned();
            order.push(parent.id().clone());
            entries.insert(parent.id().clone(), parent);
            steps += 1;
        }
        // Down: what it caused, level by level.
        let mut frontier = vec![focus_id.clone()];
        let mut depth = 0;
        let mut descendants = 0;
        while !frontier.is_empty() {
            let room = MessageChain::MAX_DESCENDANTS - descendants;
            let children: Vec<ChainEntry> = store
                .chain_children(&frontier, room + 1)
                .await?
                .into_iter()
                .filter(|c| !entries.contains_key(c.id()))
                .collect();
            if children.is_empty() {
                break;
            }
            if depth == MessageChain::MAX_DEPTH || room == 0 {
                truncated = true;
                break;
            }
            if children.len() > room {
                truncated = true;
            }
            frontier = Vec::new();
            for child in children.into_iter().take(room) {
                if entries.contains_key(child.id()) {
                    continue;
                }
                frontier.push(child.id().clone());
                order.push(child.id().clone());
                entries.insert(child.id().clone(), child);
                descendants += 1;
            }
            depth += 1;
        }
        let entries = order.into_iter().filter_map(|id| entries.remove(&id)).collect();
        Ok(Some(MessageChain { focus: focus_id, entries, truncated }))
    }

    /// Failed messages with envelope and error (`GET /failures?prefix=&since=`).
    pub async fn failures(
        &self,
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
        Ok(self.inner.store.failures(prefix, since, cursor, limit).await?)
    }

    /// All registered types with their contracts (`GET /types`), ordered by template.
    pub async fn types(&self) -> Result<Vec<RegisteredType>, OpError> {
        let defs = self.inner.store.list_types().await?;
        let mut types = defs.into_iter().map(|d| self.inner.resolver.registered(d)).collect::<Result<Vec<_>, _>>()?;
        types.sort_by(|a, b| a.template().as_str().cmp(b.template().as_str()));
        Ok(types)
    }

    /// The type matching `address` (`GET /types?uri=`).
    pub async fn type_of(&self, address: &ActorAddress) -> Result<Option<ResolvedActorType>, OpError> {
        Ok(self.inner.resolver.resolve(address).await?)
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
        self.create_actor(&address, Some(state)).await
    }

    /// Creates `r` without state (a stateless type); see [`Self::create_actor`].
    pub async fn create_stateless<R: ActorRef>(&self, r: &R) -> Result<CreateActorResult, OpError> {
        let address = r.address().map_err(|e| OpError::Invalid(e.to_string()))?;
        self.create_actor(&address, None).await
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
        let encoded = message::encode(&msg.into()).map_err(OpError::Invalid)?;
        self.send_message(&address, &encoded.message_type, encoded.payload, options).await
    }

    /// Sends a message of any family to a raw address. See [`Self::send_message`].
    pub async fn send_to<M: MessageFamily>(
        &self,
        to: &ActorAddress,
        msg: &M,
        options: SendOptions,
    ) -> Result<SendResult, OpError> {
        let encoded = message::encode(msg).map_err(OpError::Invalid)?;
        self.send_message(to, &encoded.message_type, encoded.payload, options).await
    }
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
    store: Arc<dyn Store>,
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
