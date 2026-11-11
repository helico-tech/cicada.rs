//! Shared fixtures of the runtime tests: counter and echo types, a store wrapper that
//! injects faults, virtual-time nodes and small helpers.

#![allow(dead_code, missing_docs)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::address::{ActorAddress, TypeName};
use cicada::prelude::*;
use cicada::runtime::{ActorSystemBuilder, NodeConfig, SendResult};
use cicada::store::{
    CancelResult, Claim, CommitRequest, CommitResult, CreateResult, DeleteResult, DeliverNowResult, EnqueueResult,
    Message, MessageId, MessageStatus, NodeId, NodeRecord, OutcomeRecord, OutcomeStatus, OutgoingMessage, Page,
    PageCursor, RegisterResult, RemoveTypeResult, RenewResult, RunningTurn, Store, StoreResult, StoreStats,
    TurnOutcome, TypeDef,
};
use cicada::time::{self, Timestamp};
use cicada::turn::{HandlerFailure, TurnError};
use cicada::{ActorSystem, Turn};
use cicada_test::{InMemoryStore, TokioClock, default_start};
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---- counter ----

/// type `counter`, key `{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Counter {
    pub id: String,
}

impl Counter {
    pub fn new(id: &str) -> Self {
        Self { id: id.to_owned() }
    }

    pub fn address(&self) -> ActorAddress {
        ActorRef::address(self).unwrap()
    }
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CounterMsg {
    /// Adds `n` to the total and logs it.
    Add { n: i32 },
    /// Assigns state, buffers a send, then fails.
    Boom { reason: String },
    /// Logs, then sleeps for `millis`.
    Sleep { millis: u64 },
    /// Raw-address send of `Add(n)` to `to`.
    Forward { to: String, n: i32 },
    /// Raw send of an unknown type name to `to`, catching the rejection and logging it.
    SendUnknown { to: String },
    /// Create-then-send in one turn.
    Spawn { child: String, n: i32 },
    /// `stop()`, after sending a message to itself.
    Halt,
    /// Sends `Add(1..=count)` to itself, all with the same delivery time.
    Burst { count: i32 },
    /// Sends `Add(n)` to itself after `delayMillis`.
    Later {
        n: i32,
        #[serde(rename = "delayMillis")]
        delay_millis: u64,
    },
    /// Replies `Add(n)` to `message.from`.
    Ping { n: i32 },
    /// Typed send to an instance that does not exist, plus a state change.
    Orphan { target: String },
    /// Lets the handler's own timeout error escape.
    InnerTimeout,
    /// Logs the envelope fields the handler sees.
    Inspect,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CounterState {
    #[serde(default)]
    pub total: i32,
    #[serde(default)]
    pub log: Vec<String>,
}

impl CounterState {
    pub fn total(total: i32) -> Self {
        Self { total, log: Vec::new() }
    }

    pub fn log(&mut self, entry: impl Into<String>) {
        self.log.push(entry.into());
    }
}

/// `None` as `null`.
pub fn opt<T: std::fmt::Display>(value: &Option<T>) -> String {
    value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "null".to_owned())
}

/// The shared handler of every counter type.
pub async fn handle_counter<A>(turn: &mut Turn<A>, message: CounterMsg) -> HandlerResult
where
    A: Actor<Ref = Counter, State = CounterState, Message = CounterMsg>,
{
    match message {
        CounterMsg::Add { n } => {
            let state = turn.state_mut();
            state.total += n;
            state.log(format!("add:{n}"));
        }
        CounterMsg::Boom { reason } => {
            turn.state_mut().log("boom");
            turn.send_self(CounterMsg::Add { n: 1000 })?;
            return Err(HandlerError::msg(reason));
        }
        CounterMsg::Sleep { millis } => {
            turn.state_mut().log(format!("sleep:{millis}"));
            tokio::time::sleep(Duration::from_millis(millis)).await;
            turn.state_mut().log("woke");
        }
        CounterMsg::Forward { to, n } => {
            let to = ActorAddress::parse(&to)?;
            turn.send_to(&to, &CounterMsg::Add { n }, SendOptions::new()).await?;
        }
        CounterMsg::SendUnknown { to } => {
            let to = ActorAddress::parse(&to)?;
            match turn.send_raw(&to, "Nope", serde_json::json!({}), SendOptions::new()).await {
                Ok(_) => turn.state_mut().log("sent"),
                Err(TurnError::SendRejected { message_type, .. }) => {
                    turn.state_mut().log(format!("rejected:{message_type}"))
                }
                Err(e) => return Err(e.into()),
            }
        }
        CounterMsg::Spawn { child, n } => {
            let child = Counter::new(&child);
            turn.create(&child, &CounterState::total(100))?;
            turn.send(&child, CounterMsg::Add { n })?;
        }
        CounterMsg::Halt => {
            turn.send_self(CounterMsg::Add { n: 1 })?;
            turn.stop();
        }
        CounterMsg::Burst { count } => {
            for n in 1..=count {
                turn.send_self(CounterMsg::Add { n })?;
            }
        }
        CounterMsg::Later { n, delay_millis } => {
            turn.send_self_with(CounterMsg::Add { n }, SendOptions::new().delay(Duration::from_millis(delay_millis)))?;
        }
        CounterMsg::Ping { n } => {
            let from = turn.message().from.clone().ok_or_else(|| HandlerError::msg("no sender"))?;
            turn.send_to(&from, &CounterMsg::Add { n }, SendOptions::new()).await?;
        }
        CounterMsg::Orphan { target } => {
            turn.state_mut().log("orphan");
            turn.send(&Counter::new(&target), CounterMsg::Add { n: 1 })?;
        }
        CounterMsg::InnerTimeout => {
            tokio::time::timeout(Duration::from_secs(1), tokio::time::sleep(Duration::from_secs(10))).await?;
        }
        CounterMsg::Inspect => {
            let m = turn.message().clone();
            let entry = format!(
                "attempt={} from={} type={} now={} deliverAt={} corr={} headers={:?}",
                m.attempt,
                opt(&m.from),
                m.message_type,
                time::format_instant(turn.now()),
                time::format_instant(m.deliver_at),
                opt(&m.correlation_id),
                m.headers
            );
            turn.state_mut().log(entry);
        }
    }
    Ok(())
}

/// Defines a counter type with the shared handler, the given unhandled policy and a maximum turn of one minute (or
/// the one given).
#[macro_export]
macro_rules! counter_type {
    ($name:ident, $policy:expr) => {
        $crate::counter_type!($name, $policy, ::std::time::Duration::from_secs(60));
    };
    ($name:ident, $policy:expr, $max_turn:expr) => {
        /// A counter type with the shared handler.
        pub struct $name;

        impl ::cicada::Actor for $name {
            type Ref = $crate::common::Counter;
            type State = $crate::common::CounterState;
            type Message = $crate::common::CounterMsg;
            const UNHANDLED: ::cicada::Unhandled = $policy;
            const MAX_TURN: Option<::std::time::Duration> = Some($max_turn);

            async fn handle(
                &self,
                turn: &mut ::cicada::Turn<Self>,
                message: $crate::common::CounterMsg,
            ) -> ::cicada::HandlerResult {
                $crate::common::handle_counter(turn, message).await
            }
        }
    };
}

counter_type!(CounterActor, Unhandled::Fail);

// ---- echo ----

/// type `echo`, key `{name}`: a type that keeps no data (`NoState`) and forwards `Add(1)` to a counter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Echo {
    pub name: String,
}

impl Echo {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_owned() }
    }
}

impl ActorRef for Echo {
    const TYPE: &'static str = "echo";
    type Message = EchoMsg;
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum EchoMsg {
    Hello { counter: String },
}

pub struct EchoActor;

impl Actor for EchoActor {
    type Ref = Echo;
    type State = NoState;
    type Message = EchoMsg;
    const UNHANDLED: Unhandled = Unhandled::Drop;

    async fn handle(&self, turn: &mut Turn<Self>, message: EchoMsg) -> HandlerResult {
        let EchoMsg::Hello { counter } = message;
        turn.send(&Counter::new(&counter), CounterMsg::Add { n: 1 })?;
        Ok(())
    }
}

// ---- stores and nodes ----

/// An in-memory store on tokio's (paused) clock, starting at 2026-01-01T00:00:00Z.
pub fn virtual_store() -> Arc<InMemoryStore> {
    virtual_store_at(default_start())
}

/// An in-memory store on tokio's clock, starting at `start`.
pub fn virtual_store_at(start: Timestamp) -> Arc<InMemoryStore> {
    Arc::new(InMemoryStore::new(Arc::new(TokioClock::new(start)), time::DEFAULT_RETENTION))
}

/// The configuration of a node named `name` with a lease TTL of 30 s.
pub fn node_config(name: &str) -> NodeConfig {
    NodeConfig { node_id: NodeId::new(name).unwrap(), lease_ttl: Duration::from_secs(30), ..NodeConfig::default() }
}

/// A node on `store` named `name` with a lease TTL of 30 s.
pub fn node(store: Arc<dyn Store>, name: &str) -> ActorSystemBuilder {
    ActorSystem::builder(store).config(node_config(name))
}

/// The retained outcome of message `id`; `None` while it is pending or running, or when it is unknown.
pub async fn outcome_of(store: &dyn Store, id: &MessageId) -> Option<OutcomeRecord> {
    match store.status(id).await.unwrap() {
        Some(MessageStatus::Finished(outcome)) => Some(outcome),
        _ => None,
    }
}

/// The type name of the ref type `R`.
pub fn type_name<R: ActorRef>() -> TypeName {
    TypeName::parse(R::TYPE).unwrap()
}

/// The address of a ref.
pub fn addr<R: ActorRef>(r: &R) -> ActorAddress {
    r.address().unwrap()
}

/// The accepted message of a send result.
pub fn accepted(result: SendResult) -> Message {
    match result {
        SendResult::Accepted(m) => m,
        other => panic!("send refused: {other:?}"),
    }
}

/// Enqueues directly into the store, as a node that does not host the type would.
pub async fn enqueue(store: &dyn Store, message: OutgoingMessage) -> Message {
    match store.enqueue(message).await.unwrap() {
        EnqueueResult::Enqueued(m) => m,
        other => panic!("enqueue refused: {other:?}"),
    }
}

/// Polls `condition` every 10 ms until it holds; panics after `timeout`.
pub async fn eventually<F, Fut>(timeout: Duration, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    tokio::time::timeout(timeout, async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition not met in time");
}

/// Lets background work run to idle: a turn whose `run_due_turns` future was dropped releases its lease in the
/// background.
pub async fn settle() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

/// The state of `address` decoded as `S`.
pub async fn state_of<S: serde::de::DeserializeOwned>(store: &dyn Store, address: &ActorAddress) -> S {
    let record = store.get(address).await.unwrap().unwrap_or_else(|| panic!("{address} does not exist"));
    serde_json::from_value(record.state).unwrap()
}

// ---- hooks ----

/// A shared event log.
#[derive(Debug, Clone, Default)]
pub struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub fn push(&self, event: impl Into<String>) {
        self.0.lock().unwrap().push(event.into());
    }

    pub fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    pub fn last(&self) -> String {
        self.0.lock().unwrap().last().cloned().unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// A turn outcome as text: `Done`, `Failed(error=…)`, `Rescheduled(delivery=…, error=…)`.
pub fn outcome_text(outcome: &TurnOutcome) -> String {
    match outcome {
        TurnOutcome::Done => "Done".to_owned(),
        TurnOutcome::Failed(e) => format!("Failed(error={e})"),
        TurnOutcome::Cancelled => "Cancelled".to_owned(),
        TurnOutcome::Rescheduled { delivery, error } => {
            let delivery = match delivery {
                cicada::store::Delivery::Now => "Now".to_owned(),
                cicada::store::Delivery::At(t) => format!("At(instant={})", time::format_instant(*t)),
                cicada::store::Delivery::After(d) => format!("After(delay={d:?})"),
            };
            format!("Rescheduled(delivery={delivery}, error={error})")
        }
    }
}

/// The kind of a handler failure: `TurnTimeoutException`, `Error`, `Panic`.
pub fn failure_kind(failure: &HandlerFailure) -> &'static str {
    match failure {
        HandlerFailure::Timeout { .. } => "TurnTimeoutException",
        HandlerFailure::Error(_) => "Error",
        HandlerFailure::Panic(_) => "Panic",
    }
}

// ---- fault injection ----

/// The operations a test can intercept; each default delegates to the real store.
#[async_trait]
pub trait Faults: Send + Sync + 'static {
    async fn now(&self, inner: &dyn Store) -> StoreResult<Timestamp> {
        inner.now().await
    }

    async fn list_types(&self, inner: &dyn Store) -> StoreResult<Vec<TypeDef>> {
        inner.list_types().await
    }

    async fn claim_due(
        &self,
        inner: &dyn Store,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        inner.claim_due(types, node, ttl, limit).await
    }

    async fn renew(
        &self,
        inner: &dyn Store,
        address: &ActorAddress,
        epoch: i64,
        ttl: Duration,
    ) -> StoreResult<RenewResult> {
        inner.renew(address, epoch, ttl).await
    }

    async fn commit(&self, inner: &dyn Store, commit: CommitRequest) -> StoreResult<CommitResult> {
        inner.commit(commit).await
    }

    async fn sweep_expired(&self, inner: &dyn Store) -> StoreResult<u64> {
        inner.sweep_expired().await
    }

    fn wake_ups(&self, inner: &dyn Store, types: &BTreeSet<TypeName>) -> Option<BoxStream<'static, ()>> {
        inner.wake_ups(types)
    }
}

/// A store that delegates everything to `inner`, routing the operations of [`Faults`] through `faults`.
pub struct Faulty<F> {
    pub inner: Arc<dyn Store>,
    pub faults: F,
}

impl<F: Faults> Faulty<F> {
    pub fn new(inner: Arc<dyn Store>, faults: F) -> Arc<Self> {
        Arc::new(Self { inner, faults })
    }
}

#[async_trait]
impl<F: Faults> Store for Faulty<F> {
    async fn now(&self) -> StoreResult<Timestamp> {
        self.faults.now(&*self.inner).await
    }

    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult> {
        self.inner.register(def).await
    }

    async fn remove_type(&self, name: &TypeName) -> StoreResult<RemoveTypeResult> {
        self.inner.remove_type(name).await
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        self.faults.list_types(&*self.inner).await
    }

    async fn create(&self, address: &ActorAddress, state: Value) -> StoreResult<CreateResult> {
        self.inner.create(address, state).await
    }

    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<cicada::store::ActorRecord>> {
        self.inner.get(address).await
    }

    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        self.inner.list(prefix, cursor, limit).await
    }

    async fn delete(&self, address: &ActorAddress) -> StoreResult<DeleteResult> {
        self.inner.delete(address).await
    }

    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult> {
        self.inner.enqueue(message).await
    }

    async fn claim_due(
        &self,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        self.faults.claim_due(&*self.inner, types, node, ttl, limit).await
    }

    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult> {
        self.faults.renew(&*self.inner, address, epoch, ttl).await
    }

    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        self.inner.release(address, epoch).await
    }

    async fn next_due_at(&self, types: &BTreeSet<TypeName>) -> StoreResult<Option<Timestamp>> {
        self.inner.next_due_at(types).await
    }

    async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        self.inner.pending(address, cursor, limit).await
    }

    async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        self.inner.list_pending(prefix, cursor, limit).await
    }

    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        self.inner.cancel(id).await
    }

    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        self.inner.status(id).await
    }

    async fn outcomes(
        &self,
        status: Option<OutcomeStatus>,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>> {
        self.inner.outcomes(status, prefix, since, cursor, limit).await
    }

    async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<MessageStatus>> {
        self.inner.caused_by(id, cursor, limit).await
    }

    async fn stats(&self, prefix: &str) -> StoreResult<StoreStats> {
        self.inner.stats(prefix).await
    }

    async fn sweep_expired(&self) -> StoreResult<u64> {
        self.faults.sweep_expired(&*self.inner).await
    }

    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult> {
        self.faults.commit(&*self.inner, commit).await
    }

    async fn deliver_now(&self, id: &MessageId) -> StoreResult<DeliverNowResult> {
        self.inner.deliver_now(id).await
    }

    async fn set_paused(&self, address: &ActorAddress, paused: bool) -> StoreResult<bool> {
        self.inner.set_paused(address, paused).await
    }

    async fn purge(&self, address: &ActorAddress) -> StoreResult<Option<u64>> {
        self.inner.purge(address).await
    }

    async fn heartbeat_node(&self, id: &NodeId, hosted: &BTreeSet<TypeName>) -> StoreResult<()> {
        self.inner.heartbeat_node(id, hosted).await
    }

    async fn list_nodes(&self) -> StoreResult<Vec<NodeRecord>> {
        self.inner.list_nodes().await
    }

    async fn remove_node(&self, id: &NodeId) -> StoreResult<()> {
        self.inner.remove_node(id).await
    }

    async fn running_turns(
        &self,
        node: Option<&NodeId>,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<RunningTurn>> {
        self.inner.running_turns(node, prefix, cursor, limit).await
    }

    async fn running_turn(&self, address: &ActorAddress) -> StoreResult<Option<RunningTurn>> {
        self.inner.running_turn(address).await
    }

    async fn request_cancel(&self, address: &ActorAddress) -> StoreResult<bool> {
        self.inner.request_cancel(address).await
    }

    fn wake_ups(&self, types: &BTreeSet<TypeName>) -> Option<BoxStream<'static, ()>> {
        self.faults.wake_ups(&*self.inner, types)
    }

    async fn close(&self) {
        self.inner.close().await
    }
}
