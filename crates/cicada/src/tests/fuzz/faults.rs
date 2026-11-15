//! A store wrapper for one simulated node, and the log of what really happened.
//!
//! A node reaches the shared store only through its own [`FaultStore`], and every store method goes through one gate
//! that applies the node's fault rules. Faults are keyed by call count, not by time, as call counts stay exact on replay.
//!
//! The gate writes what the *inner* store answered to the [`Log`], including answers the node never received. An
//! `InMemoryStore` call never yields, so on one thread the extra reads around a call see exactly the state of that call.
//!
//! The notifications of a commit or a delete are accepted messages like the sends: the layer finds them by the event id
//! they are named after. The runtime draws a random event id for every commit; a layer that [names events]
//! (FaultStore::name_events) names it after the claim instead (`<message>#<epoch>`), so that a replay writes the same
//! notifications.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::address::{ActorAddress, TypeName};
use crate::retry::RetryPolicy;
use crate::store::{
    ActorRecord, Backend, CancelResult, Claim, CommitRequest, CommitResult, ControlEffect, CreateResult, DeleteResult,
    DeliverNowResult, EnqueueResult, Message, MessageId, MessageStatus, NodeDetails, NodeId, NodeRecord, OutcomeRecord,
    OutcomeStatus, OutgoingMessage, Page, PageCursor, RegisterResult, RemoveTypeResult, RenewResult, RunningTurn,
    SetWatchResult, StateChange, StoreError, StoreResult, StoreStats, TurnOutcome, TypeDef,
};
use crate::time::Timestamp;
use crate::watch::{Watch, WatchTarget};
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;
use tokio::time::sleep;

/// The methods of [`Store`] that go through the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum Method {
    Now,
    Register,
    ListTypes,
    RemoveType,
    Create,
    Get,
    List,
    ListRecords,
    Children,
    Delete,
    Watch,
    Unwatch,
    Watches,
    Enqueue,
    ClaimDue,
    Renew,
    Release,
    NextDueAt,
    Pending,
    ListPending,
    DeliverNow,
    Cancel,
    SetPaused,
    Purge,
    Status,
    Outcomes,
    CausedBy,
    Stats,
    SweepExpired,
    HeartbeatNode,
    ListNodes,
    RemoveNode,
    RunningTurns,
    RunningTurn,
    RequestCancel,
    Commit,
}

impl Method {
    /// Every method.
    pub const ALL: [Method; 36] = [
        Method::Now,
        Method::Register,
        Method::ListTypes,
        Method::RemoveType,
        Method::Create,
        Method::Get,
        Method::List,
        Method::ListRecords,
        Method::Children,
        Method::Delete,
        Method::Watch,
        Method::Unwatch,
        Method::Watches,
        Method::Enqueue,
        Method::ClaimDue,
        Method::Renew,
        Method::Release,
        Method::NextDueAt,
        Method::Pending,
        Method::ListPending,
        Method::DeliverNow,
        Method::Cancel,
        Method::SetPaused,
        Method::Purge,
        Method::Status,
        Method::Outcomes,
        Method::CausedBy,
        Method::Stats,
        Method::SweepExpired,
        Method::HeartbeatNode,
        Method::ListNodes,
        Method::RemoveNode,
        Method::RunningTurns,
        Method::RunningTurn,
        Method::RequestCancel,
        Method::Commit,
    ];
}

/// What goes wrong with one store call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The call fails with `Unavailable`; nothing is applied.
    Before,
    /// The call is applied and then fails with `Unavailable`: a lost acknowledgement.
    After,
    /// The call waits on virtual time, then runs.
    Delay(Duration),
    /// This call and every later one from this node fail: a crash, or a network split that does not heal.
    Dead,
    /// This call and the next calls of the same method fail, this many in all, with nothing applied: a store that is
    /// unreachable for a while. Renewals that fail in a row lose a lease, so that messages lose claims.
    Burst(u32),
}

/// Applies `fault` to the `nth` call (from 0) of `method`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    /// The method.
    pub method: Method,
    /// Which call of it, counted from 0 after [`FaultStore::arm`].
    pub nth: u32,
    /// What happens to that call.
    pub fault: Fault,
}

/// How a handler run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerEnd {
    /// It returned `Ok`.
    Returned,
    /// It returned an error.
    Failed,
    /// It panicked.
    Panicked,
    /// It was dropped at an `.await`: a lost lease, a turn timeout or a cancel.
    Dropped,
}

/// How a commit ended the turn's message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// Outcome `done`.
    Done,
    /// Outcome `failed`.
    Failed,
    /// Outcome `failed` with the error of the claim limit (ADR 0025), which no handler of the simulation returns.
    ClaimLimit,
    /// Outcome `cancelled`.
    Cancelled,
    /// Back in the mailbox.
    Rescheduled,
}

impl From<&TurnOutcome> for Ending {
    fn from(outcome: &TurnOutcome) -> Self {
        match outcome {
            TurnOutcome::Done => Ending::Done,
            TurnOutcome::Failed(error) if error.starts_with("claim limit: ") => Ending::ClaimLimit,
            TurnOutcome::Failed(_) => Ending::Failed,
            TurnOutcome::Cancelled => Ending::Cancelled,
            TurnOutcome::Rescheduled { .. } => Ending::Rescheduled,
        }
    }
}

/// One thing that happened in a run.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A fault rule fired.
    Fault {
        /// The node.
        node: String,
        /// The method.
        method: Method,
        /// Which call of it.
        nth: u32,
        /// The fault.
        fault: Fault,
    },
    /// The store accepted a message, from a client or in a commit.
    Accepted {
        /// The message.
        id: String,
        /// Its target.
        to: String,
    },
    /// The store granted a claim, even when the node did not get the answer.
    Claimed {
        /// The node.
        node: String,
        /// The instance.
        address: String,
        /// The fencing token.
        epoch: i64,
        /// The claimed message.
        id: String,
        /// The attempt of this claim.
        attempt: u32,
        /// The message's failed runs that were retried so far.
        failures: u32,
        /// The message's lost claims, this claim's count included.
        lost_claims: u32,
        /// The message's retry policy.
        retry: Option<RetryPolicy>,
        /// The message's delivery time.
        deliver_at: Timestamp,
        /// Store time at the claim.
        at: Timestamp,
        /// The head of the mailbox, read straight after the claim.
        head: Option<String>,
    },
    /// A commit was applied, even when the node did not get the answer.
    Committed {
        /// The node.
        node: String,
        /// The instance.
        address: String,
        /// The fencing token.
        epoch: i64,
        /// The turn's message.
        id: String,
        /// How the message ended.
        ending: Ending,
    },
    /// The store answered a commit without applying it.
    CommitRefused {
        /// The node.
        node: String,
        /// The instance.
        address: String,
        /// The fencing token.
        epoch: i64,
        /// The store's answer.
        answer: String,
    },
    /// A node asked to give up a lease.
    Released {
        /// The node.
        node: String,
        /// The instance.
        address: String,
        /// The fencing token.
        epoch: i64,
    },
    /// A message left the mailbox for good.
    Ended {
        /// The message.
        id: String,
    },
    /// An instance was removed, by a delete or by a stopping turn.
    Removed {
        /// The instance.
        address: String,
    },
    /// A handler started.
    HandlerStart {
        /// The node.
        node: String,
        /// The instance.
        address: String,
        /// The message.
        id: String,
        /// The message's attempt.
        attempt: u32,
    },
    /// A handler ended.
    HandlerEnd {
        /// The node.
        node: String,
        /// The instance.
        address: String,
        /// The message.
        id: String,
        /// The message's attempt.
        attempt: u32,
        /// How.
        how: HandlerEnd,
    },
    /// After the drain: a retained outcome.
    Outcome {
        /// The message.
        id: String,
        /// Its status.
        status: OutcomeStatus,
    },
    /// After the drain: a message still in a mailbox.
    StillPending {
        /// The message.
        id: String,
        /// True when it is due.
        due: bool,
    },
    /// After the drain: the message ids in an instance's state.
    State {
        /// The instance.
        address: String,
        /// The ids.
        ids: Vec<String>,
    },
    /// After the drain: a notification, pending or finished.
    Notified {
        /// The notification.
        id: String,
        /// `Failed` or `Stopped`.
        kind: String,
        /// The failed message of a `Failed`, the actor of a `Stopped`.
        of: String,
    },
}

/// The history of a run, shared by the fault layers and the handlers.
#[derive(Debug, Clone, Default)]
pub struct Log(Arc<Mutex<Vec<Event>>>);

impl Log {
    /// Adds an event.
    pub fn push(&self, event: Event) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(event);
    }

    /// Every event so far.
    pub fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The number of handlers that started and have not ended.
    pub fn open_handlers(&self) -> usize {
        let events = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let started = events.iter().filter(|e| matches!(e, Event::HandlerStart { .. })).count();
        let ended = events.iter().filter(|e| matches!(e, Event::HandlerEnd { .. })).count();
        started - ended
    }
}

#[derive(Default)]
struct Plan {
    rules: Vec<Rule>,
    calls: BTreeMap<Method, u32>,
    /// The calls of a method that a [`Fault::Burst`] still fails.
    bursts: BTreeMap<Method, u32>,
    armed: bool,
    dead: bool,
}

/// One node's view of the shared store. See the module documentation.
pub struct FaultStore {
    inner: Arc<dyn Backend>,
    node: String,
    log: Log,
    plan: Mutex<Plan>,
    name_events: std::sync::atomic::AtomicBool,
}

impl FaultStore {
    /// A layer for `node` around `inner`, with `rules`. Starts unarmed: until [`Self::arm`], calls are not counted and
    /// no rule fires, so that a node can start.
    pub fn new(inner: Arc<dyn Backend>, node: impl Into<String>, rules: Vec<Rule>, log: Log) -> Arc<Self> {
        let plan = Mutex::new(Plan { rules, ..Plan::default() });
        Arc::new(Self { inner, node: node.into(), log, plan, name_events: Default::default() })
    }

    /// Names the event of every commit after its claim, `<message>#<epoch>`, in place of the random id the runtime
    /// drew: a replay then writes the same notification ids.
    pub fn name_events(&self) {
        self.name_events.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Starts counting calls and applying the rules.
    pub fn arm(&self) {
        self.plan.lock().unwrap_or_else(PoisonError::into_inner).armed = true;
    }

    /// Stops applying the rules. A node that went dead stays dead.
    pub fn heal(&self) {
        self.plan.lock().unwrap_or_else(PoisonError::into_inner).armed = false;
    }

    /// The fault of this call, if it has one.
    fn fault_of(&self, method: Method) -> Option<Fault> {
        let mut plan = self.plan.lock().unwrap_or_else(PoisonError::into_inner);
        if plan.dead {
            return Some(Fault::Dead);
        }
        if !plan.armed {
            return None;
        }
        let count = plan.calls.entry(method).or_default();
        let nth = *count;
        *count += 1;
        let fault = match plan.bursts.get_mut(&method).filter(|left| **left > 0) {
            Some(left) => {
                *left -= 1;
                Fault::Before
            }
            None => plan.rules.iter().find(|r| r.method == method && r.nth == nth)?.fault,
        };
        plan.dead = fault == Fault::Dead;
        if let Fault::Burst(calls) = fault {
            plan.bursts.insert(method, calls.saturating_sub(1));
        }
        drop(plan);
        self.log.push(Event::Fault { node: self.node.clone(), method, nth, fault });
        Some(fault)
    }

    /// The gate: every method of the store runs its call through this.
    async fn gate<T, Fut>(&self, method: Method, call: impl FnOnce() -> Fut + Send) -> StoreResult<T>
    where
        T: Send,
        Fut: Future<Output = StoreResult<T>> + Send,
    {
        match self.fault_of(method) {
            None => call().await,
            Some(Fault::Before | Fault::Dead | Fault::Burst(_)) => {
                Err(StoreError::unavailable("injected: not applied"))
            }
            Some(Fault::After) => {
                let _ = call().await;
                Err(StoreError::unavailable("injected: applied, acknowledgement lost"))
            }
            Some(Fault::Delay(by)) => {
                sleep(by).await;
                call().await
            }
        }
    }

    async fn head_of(&self, address: &ActorAddress) -> Option<String> {
        let page = self.inner.pending(address, None, 1).await.ok()?;
        page.items.first().map(|m| m.id.to_string())
    }

    /// Every pending message of `address`.
    async fn pending_ids(&self, address: &ActorAddress) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        let mut cursor = None;
        while let Ok(page) = self.inner.pending(address, cursor.as_ref(), Page::<Message>::DEFAULT_SIZE).await {
            ids.extend(page.items.iter().map(|m| m.id.to_string()));
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        ids
    }

    /// `root` and every descendant, when `root` exists; nothing otherwise.
    async fn tree_of(&self, root: &ActorAddress) -> BTreeSet<ActorAddress> {
        let mut tree = BTreeSet::new();
        if !matches!(self.inner.get(root).await, Ok(Some(_))) {
            return tree;
        }
        let mut level = vec![root.clone()];
        while let Some(parent) = level.pop() {
            if !tree.insert(parent.clone()) {
                continue;
            }
            let mut cursor = None;
            while let Ok(page) = self.inner.children(&parent, cursor.as_ref(), Page::<Message>::DEFAULT_SIZE).await {
                level.extend(page.items);
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
        }
        tree
    }

    /// Logs every id of `before` that is no longer pending at `address`.
    async fn log_ended(&self, address: &ActorAddress, before: BTreeSet<String>) {
        let after = self.pending_ids(address).await;
        for id in before.difference(&after) {
            self.log.push(Event::Ended { id: id.clone() });
        }
    }

    fn log_accepted(&self, message: &Message) {
        self.log.push(Event::Accepted { id: message.id.to_string(), to: message.to.to_string() });
    }

    /// Logs the notifications that the event `event` wrote as accepted: the pending messages named after it.
    async fn log_notifications(&self, event: &MessageId) {
        let prefix = format!("{event}.");
        let mut cursor = None;
        while let Ok(page) = self.inner.list_pending("", cursor.as_ref(), Page::<Message>::DEFAULT_SIZE).await {
            for message in page.items.iter().filter(|m| m.notification && m.id.as_str().starts_with(&prefix)) {
                self.log_accepted(message);
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
    }

    async fn do_claim(
        &self,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        let claims = self.inner.claim_due(types, node, ttl, limit).await?;
        for claim in &claims {
            let head = self.head_of(&claim.actor).await;
            self.log.push(Event::Claimed {
                node: self.node.clone(),
                address: claim.actor.to_string(),
                epoch: claim.epoch,
                id: claim.message.id.to_string(),
                attempt: claim.message.attempt,
                failures: claim.message.failures,
                lost_claims: claim.message.lost_claims,
                retry: claim.message.retry.clone(),
                deliver_at: claim.message.deliver_at,
                at: claim.claimed_at,
                head,
            });
        }
        Ok(claims)
    }

    async fn do_commit(&self, commit: CommitRequest) -> StoreResult<CommitResult> {
        let commit = if self.name_events.load(std::sync::atomic::Ordering::SeqCst) {
            let event = MessageId::new(format!("{}#{}", commit.message_id, commit.epoch)).expect("an id is not blank");
            CommitRequest { event, ..commit }
        } else {
            commit
        };
        let event = commit.event.clone();
        let (address, epoch, id) = (commit.address.clone(), commit.epoch, commit.message_id.clone());
        let ending = Ending::from(&commit.outcome);
        let stops = matches!(commit.state, StateChange::Stop);
        let mut before = self.pending_ids(&address).await;
        // The actors the commit's deletes would remove, and the mailboxes its deletes and purges would empty.
        let mut deleted = BTreeSet::new();
        for target in commit.deletes() {
            deleted.extend(self.tree_of(target).await);
        }
        let purged = commit.controls.iter().filter(|c| c.effect == ControlEffect::Purge).map(|c| c.actor.clone());
        let mut others = BTreeMap::new();
        for other in deleted.iter().cloned().chain(purged).filter(|a| *a != address) {
            let ids = self.pending_ids(&other).await;
            others.insert(other, ids);
        }
        let answer = self.inner.commit(commit).await?;
        let refused = |answer: String| Event::CommitRefused {
            node: self.node.clone(),
            address: address.to_string(),
            epoch,
            answer,
        };
        match &answer {
            CommitResult::Committed { sent, .. } => {
                self.log.push(Event::Committed {
                    node: self.node.clone(),
                    address: address.to_string(),
                    epoch,
                    id: id.to_string(),
                    ending,
                });
                for message in sent {
                    self.log_accepted(message);
                    // A stopping turn can send to itself: the message is accepted and purged in the same commit.
                    if message.to == address {
                        before.insert(message.id.to_string());
                    }
                }
                self.log_ended(&address, before).await;
                for (other, ids) in others {
                    self.log_ended(&other, ids).await;
                }
                for removed in &deleted {
                    self.log.push(Event::Removed { address: removed.to_string() });
                }
                if stops {
                    self.log.push(Event::Removed { address: address.to_string() });
                }
                self.log_notifications(&event).await;
            }
            CommitResult::LeaseLost => self.log.push(refused("lease lost".to_owned())),
            CommitResult::Rejected(reason) => self.log.push(refused(format!("rejected: {reason}"))),
        }
        Ok(answer)
    }

    async fn do_delete(&self, address: &ActorAddress, event: &MessageId) -> StoreResult<DeleteResult> {
        let before = self.pending_ids(address).await;
        let answer = self.inner.delete(address, event).await?;
        if matches!(answer, DeleteResult::Deleted { .. }) {
            self.log_ended(address, before).await;
            self.log.push(Event::Removed { address: address.to_string() });
            self.log_notifications(event).await;
        }
        Ok(answer)
    }

    async fn do_purge(&self, address: &ActorAddress) -> StoreResult<Option<u64>> {
        let before = self.pending_ids(address).await;
        let answer = self.inner.purge(address).await?;
        self.log_ended(address, before).await;
        Ok(answer)
    }

    async fn do_enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult> {
        let answer = self.inner.enqueue(message).await?;
        if let EnqueueResult::Enqueued(message) = &answer {
            self.log_accepted(message);
        }
        Ok(answer)
    }

    async fn do_cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        let answer = self.inner.cancel(id).await?;
        if matches!(answer, CancelResult::Cancelled(_)) {
            self.log.push(Event::Ended { id: id.to_string() });
        }
        Ok(answer)
    }

    async fn do_release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        self.log.push(Event::Released { node: self.node.clone(), address: address.to_string(), epoch });
        self.inner.release(address, epoch).await
    }
}

#[async_trait]
impl Backend for FaultStore {
    async fn now(&self) -> StoreResult<Timestamp> {
        self.gate(Method::Now, || self.inner.now()).await
    }

    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult> {
        self.gate(Method::Register, || self.inner.register(def)).await
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        self.gate(Method::ListTypes, || self.inner.list_types()).await
    }

    async fn remove_type(&self, name: &TypeName) -> StoreResult<RemoveTypeResult> {
        self.gate(Method::RemoveType, || self.inner.remove_type(name)).await
    }

    async fn create(&self, address: &ActorAddress, state: Value) -> StoreResult<CreateResult> {
        self.gate(Method::Create, || self.inner.create(address, state)).await
    }

    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        self.gate(Method::Get, || self.inner.get(address)).await
    }

    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        self.gate(Method::List, || self.inner.list(prefix, cursor, limit)).await
    }

    async fn list_records(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorRecord>> {
        self.gate(Method::ListRecords, || self.inner.list_records(prefix, cursor, limit)).await
    }

    async fn children(
        &self,
        parent: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<ActorAddress>> {
        self.gate(Method::Children, || self.inner.children(parent, cursor, limit)).await
    }

    async fn delete(&self, address: &ActorAddress, event: &MessageId) -> StoreResult<DeleteResult> {
        self.gate(Method::Delete, || self.do_delete(address, event)).await
    }

    async fn watch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<SetWatchResult> {
        self.gate(Method::Watch, || self.inner.watch(target, watcher)).await
    }

    async fn unwatch(&self, target: &WatchTarget, watcher: &ActorAddress) -> StoreResult<bool> {
        self.gate(Method::Unwatch, || self.inner.unwatch(target, watcher)).await
    }

    async fn watches(
        &self,
        watcher: Option<&ActorAddress>,
        target: Option<&WatchTarget>,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Watch>> {
        self.gate(Method::Watches, || self.inner.watches(watcher, target, cursor, limit)).await
    }

    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult> {
        self.gate(Method::Enqueue, || self.do_enqueue(message)).await
    }

    async fn claim_due(
        &self,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        self.gate(Method::ClaimDue, || self.do_claim(types, node, ttl, limit)).await
    }

    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult> {
        self.gate(Method::Renew, || self.inner.renew(address, epoch, ttl)).await
    }

    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        self.gate(Method::Release, || self.do_release(address, epoch)).await
    }

    async fn next_due_at(&self, types: &BTreeSet<TypeName>) -> StoreResult<Option<Timestamp>> {
        self.gate(Method::NextDueAt, || self.inner.next_due_at(types)).await
    }

    async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        self.gate(Method::Pending, || self.inner.pending(address, cursor, limit)).await
    }

    async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        self.gate(Method::ListPending, || self.inner.list_pending(prefix, cursor, limit)).await
    }

    async fn deliver_now(&self, id: &MessageId) -> StoreResult<DeliverNowResult> {
        self.gate(Method::DeliverNow, || self.inner.deliver_now(id)).await
    }

    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        self.gate(Method::Cancel, || self.do_cancel(id)).await
    }

    async fn set_paused(&self, address: &ActorAddress, paused: bool) -> StoreResult<bool> {
        self.gate(Method::SetPaused, || self.inner.set_paused(address, paused)).await
    }

    async fn purge(&self, address: &ActorAddress) -> StoreResult<Option<u64>> {
        self.gate(Method::Purge, || self.do_purge(address)).await
    }

    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        self.gate(Method::Status, || self.inner.status(id)).await
    }

    async fn outcomes(
        &self,
        status: Option<OutcomeStatus>,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>> {
        self.gate(Method::Outcomes, || self.inner.outcomes(status, prefix, since, cursor, limit)).await
    }

    async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<MessageStatus>> {
        self.gate(Method::CausedBy, || self.inner.caused_by(id, cursor, limit)).await
    }

    async fn stats(&self, prefix: &str) -> StoreResult<StoreStats> {
        self.gate(Method::Stats, || self.inner.stats(prefix)).await
    }

    async fn sweep_expired(&self) -> StoreResult<u64> {
        self.gate(Method::SweepExpired, || self.inner.sweep_expired()).await
    }

    async fn heartbeat_node(&self, id: &NodeId, hosted: &BTreeSet<TypeName>, details: &NodeDetails) -> StoreResult<()> {
        self.gate(Method::HeartbeatNode, || self.inner.heartbeat_node(id, hosted, details)).await
    }

    async fn list_nodes(&self) -> StoreResult<Vec<NodeRecord>> {
        self.gate(Method::ListNodes, || self.inner.list_nodes()).await
    }

    async fn remove_node(&self, id: &NodeId) -> StoreResult<()> {
        self.gate(Method::RemoveNode, || self.inner.remove_node(id)).await
    }

    async fn running_turns(
        &self,
        node: Option<&NodeId>,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<RunningTurn>> {
        self.gate(Method::RunningTurns, || self.inner.running_turns(node, prefix, cursor, limit)).await
    }

    async fn running_turn(&self, address: &ActorAddress) -> StoreResult<Option<RunningTurn>> {
        self.gate(Method::RunningTurn, || self.inner.running_turn(address)).await
    }

    async fn request_cancel(&self, address: &ActorAddress) -> StoreResult<bool> {
        self.gate(Method::RequestCancel, || self.inner.request_cancel(address)).await
    }

    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult> {
        self.gate(Method::Commit, || self.do_commit(commit)).await
    }

    fn changes(&self) -> std::sync::Arc<crate::change::ChangeHub> {
        self.inner.changes()
    }

    fn wake_ups(&self, types: &BTreeSet<TypeName>) -> Option<BoxStream<'static, ()>> {
        self.inner.wake_ups(types)
    }

    async fn close(&self) {
        self.inner.close().await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::testing::{InMemoryStore, TokioClock, VirtualClock, default_start};
    use proptest::prelude::*;
    use tokio::time::Instant;

    use super::*;
    use crate::tests::fuzz::compare::{Pair, ops};

    const LOST: &str = "store unavailable: injected: applied, acknowledgement lost";
    const NOT_APPLIED: &str = "store unavailable: injected: not applied";

    fn address() -> ActorAddress {
        ActorAddress::parse("alpha/0").unwrap()
    }

    fn type_def() -> TypeDef {
        TypeDef {
            name: TypeName::parse("alpha").unwrap(),
            contract_hash: "hash".to_owned(),
            contract: serde_json::json!({}),
        }
    }

    fn outgoing(id: &str) -> OutgoingMessage {
        OutgoingMessage { id: MessageId::new(id).unwrap(), ..OutgoingMessage::new(address(), "Run", Value::Null) }
    }

    fn types() -> BTreeSet<TypeName> {
        BTreeSet::from([TypeName::parse("alpha").unwrap()])
    }

    fn node() -> NodeId {
        NodeId::new("n1").unwrap()
    }

    /// A store with one instance and one due message, and a fault layer `n1` on it with `rules`, armed.
    async fn layer(rules: Vec<Rule>) -> (Arc<InMemoryStore>, Arc<FaultStore>, Log) {
        let memory = Arc::new(InMemoryStore::new(Arc::new(TokioClock::new(default_start())), Duration::from_secs(600)));
        memory.register(type_def()).await.unwrap();
        memory.create(&address(), serde_json::json!({})).await.unwrap();
        memory.enqueue(outgoing("m1")).await.unwrap();
        let log = Log::default();
        let store = FaultStore::new(memory.clone(), "n1", rules, log.clone());
        store.arm();
        (memory, store, log)
    }

    fn rule(method: Method, nth: u32, fault: Fault) -> Rule {
        Rule { method, nth, fault }
    }

    async fn claim(store: &dyn Backend) -> Claim {
        store.claim_due(&types(), &node(), Duration::from_secs(30), 1).await.unwrap().remove(0)
    }

    fn done(claim: &Claim) -> CommitRequest {
        CommitRequest::new(claim.actor.clone(), claim.epoch, claim.message.id.clone(), TurnOutcome::Done)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn with_no_rules_every_method_gives_the_inner_answer(list in ops()) {
            let result = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
                let clock = Arc::new(VirtualClock::default());
                let retention = Duration::from_secs(300);
                let first = Arc::new(InMemoryStore::new(clock.clone(), retention));
                let second = Arc::new(InMemoryStore::new(clock.clone(), retention));
                let layered = FaultStore::new(first, "n1", Vec::new(), Log::default());
                layered.arm();
                Pair::of(clock, layered, second).await.run(&list).await
            });
            if let Err(difference) = result {
                return Err(TestCaseError::fail(difference.to_string()));
            }
        }
    }

    /// The node rows, which the operation lists leave out.
    #[tokio::test(start_paused = true)]
    async fn with_no_rules_the_node_rows_give_the_inner_answer() {
        let (memory, store, _) = layer(Vec::new()).await;
        store.heartbeat_node(&node(), &types(), &Default::default()).await.unwrap();
        assert_eq!(store.list_nodes().await.unwrap(), memory.list_nodes().await.unwrap());
        assert_eq!(store.list_nodes().await.unwrap().len(), 1);
        store.remove_node(&node()).await.unwrap();
        assert!(memory.list_nodes().await.unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn before_fails_the_nth_call_and_applies_nothing() {
        let (memory, store, _) = layer(vec![rule(Method::Enqueue, 1, Fault::Before)]).await;
        assert!(matches!(store.enqueue(outgoing("a")).await.unwrap(), EnqueueResult::Enqueued(_)));
        assert_eq!(store.enqueue(outgoing("b")).await.unwrap_err().to_string(), NOT_APPLIED);
        assert_eq!(memory.status(&MessageId::new("b").unwrap()).await.unwrap(), None);
        assert!(matches!(store.enqueue(outgoing("c")).await.unwrap(), EnqueueResult::Enqueued(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn after_leaves_the_commit_applied_and_returns_unavailable() {
        let (memory, store, _) = layer(vec![rule(Method::Commit, 0, Fault::After)]).await;
        let claim = claim(&*store).await;
        let error = store.commit(done(&claim)).await.unwrap_err();
        assert!(error.is_unavailable());
        assert_eq!(error.to_string(), LOST);
        let status = memory.status(&claim.message.id).await.unwrap();
        assert!(matches!(status, Some(MessageStatus::Finished(o)) if o.status == OutcomeStatus::Done));
    }

    #[tokio::test(start_paused = true)]
    async fn delay_waits_on_virtual_time_and_then_runs() {
        let (memory, store, _) = layer(vec![rule(Method::Enqueue, 0, Fault::Delay(Duration::from_secs(7)))]).await;
        let started = Instant::now();
        store.enqueue(outgoing("a")).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(7));
        assert!(memory.status(&MessageId::new("a").unwrap()).await.unwrap().is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn burst_fails_this_call_and_the_next_ones_of_the_method_until_its_count_is_used_up() {
        let (memory, store, log) = layer(vec![rule(Method::Enqueue, 1, Fault::Burst(3))]).await;
        assert!(matches!(store.enqueue(outgoing("a")).await.unwrap(), EnqueueResult::Enqueued(_)));
        store.now().await.expect("another method is not affected");
        for id in ["b", "c", "d"] {
            assert_eq!(store.enqueue(outgoing(id)).await.unwrap_err().to_string(), NOT_APPLIED, "{id}");
            assert_eq!(memory.status(&MessageId::new(id).unwrap()).await.unwrap(), None, "{id}");
        }
        assert!(matches!(store.enqueue(outgoing("e")).await.unwrap(), EnqueueResult::Enqueued(_)));
        let faults: Vec<(u32, Fault)> = log
            .events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Fault { nth, fault, .. } => Some((nth, fault)),
                _ => None,
            })
            .collect();
        assert_eq!(faults, [(1, Fault::Burst(3)), (2, Fault::Before), (3, Fault::Before)]);
    }

    #[tokio::test(start_paused = true)]
    async fn dead_fails_this_call_and_every_later_one_also_after_a_heal() {
        let (memory, store, _) = layer(vec![rule(Method::Now, 1, Fault::Dead)]).await;
        store.now().await.unwrap();
        assert_eq!(store.now().await.unwrap_err().to_string(), NOT_APPLIED);
        assert_eq!(store.enqueue(outgoing("a")).await.unwrap_err().to_string(), NOT_APPLIED);
        store.heal();
        assert!(store.now().await.is_err());
        memory.now().await.expect("another node keeps working");
    }

    #[tokio::test(start_paused = true)]
    async fn calls_are_counted_per_method_and_only_when_armed() {
        let memory = Arc::new(InMemoryStore::new(Arc::new(TokioClock::new(default_start())), Duration::from_secs(600)));
        let store = FaultStore::new(memory, "n1", vec![rule(Method::Now, 0, Fault::Before)], Log::default());
        store.now().await.expect("not armed yet");
        store.list_types().await.unwrap();
        store.arm();
        store.list_types().await.unwrap();
        assert!(store.now().await.is_err(), "the first armed call of now");
        store.now().await.expect("only the first");
        store.heal();
    }

    #[tokio::test(start_paused = true)]
    async fn a_lost_acknowledgement_is_in_the_log() {
        let (_, store, log) = layer(vec![rule(Method::Commit, 0, Fault::After)]).await;
        let claim = claim(&*store).await;
        store.commit(done(&claim)).await.unwrap_err();
        let events = log.events();
        let fault = Event::Fault { node: "n1".to_owned(), method: Method::Commit, nth: 0, fault: Fault::After };
        let committed = Event::Committed {
            node: "n1".to_owned(),
            address: "alpha/0".to_owned(),
            epoch: claim.epoch,
            id: "m1".to_owned(),
            ending: Ending::Done,
        };
        let ended = Event::Ended { id: "m1".to_owned() };
        assert_eq!(events[events.len() - 3..], [fault, committed, ended], "{events:#?}");
    }

    #[tokio::test(start_paused = true)]
    async fn the_log_has_what_the_store_answered() {
        let (_, store, log) = layer(Vec::new()).await;
        let first = claim(&*store).await;
        tokio::time::sleep(Duration::from_secs(31)).await;
        store.enqueue(outgoing("m2")).await.unwrap();
        let second = claim(&*store).await;
        assert_eq!(store.commit(done(&first)).await.unwrap(), CommitResult::LeaseLost);
        store.commit(done(&second)).await.unwrap();
        let mut seen = BTreeMap::new();
        for event in log.events() {
            let name = format!("{event:?}");
            *seen.entry(name.split([' ', '(']).next().unwrap().to_owned()).or_insert(0) += 1;
        }
        let expected =
            BTreeMap::from([("Accepted", 1), ("Claimed", 2), ("CommitRefused", 1), ("Committed", 1), ("Ended", 1)]);
        assert_eq!(seen, expected.into_iter().map(|(k, v)| (k.to_owned(), v)).collect());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopping_turn_ends_what_it_purges_and_removes_the_instance() {
        let (memory, store, log) = layer(Vec::new()).await;
        memory.enqueue(outgoing("m2")).await.unwrap();
        let claim = claim(&*store).await;
        let mut commit = done(&claim);
        commit.state = StateChange::Stop;
        commit.sends = vec![outgoing("m3")];
        store.commit(commit).await.unwrap();
        let ended: BTreeSet<String> = log
            .events()
            .iter()
            .filter_map(|e| if let Event::Ended { id } = e { Some(id.clone()) } else { None })
            .collect();
        assert_eq!(ended, BTreeSet::from(["m1".to_owned(), "m2".to_owned(), "m3".to_owned()]));
        assert!(log.events().contains(&Event::Removed { address: "alpha/0".to_owned() }));
    }
}
