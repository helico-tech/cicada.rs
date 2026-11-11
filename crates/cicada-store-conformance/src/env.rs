//! Helpers shared by the cases: the per-case [`Env`], constants and assertions.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use cicada::address::{ActorAddress, TypeName};
use cicada::store::{
    Claim, CommitRequest, CommitResult, CreateResult, Delivery, EnqueueResult, Message, MessageId, MessageStatus,
    NodeId, OutcomeRecord, OutgoingMessage, Page, PageCursor, Store, TurnOutcome, TypeDef,
};
use cicada::time::{self, Timestamp};
use serde_json::{Value, json};

use crate::fixture::{FixtureFactory, StoreFixture};

/// The main type used by the cases.
pub static TYPE: LazyLock<TypeName> = LazyLock::new(|| type_name("conformance"));

/// A second type, for type filtering.
pub static OTHER: LazyLock<TypeName> = LazyLock::new(|| type_name("other"));

/// A node identity used by the cases.
pub static NODE_A: LazyLock<NodeId> = LazyLock::new(|| node("node-a"));

/// A second node identity used by the cases.
pub static NODE_B: LazyLock<NodeId> = LazyLock::new(|| node("node-b"));

/// How long a case may run unless it says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs `block` against a fresh fixture with the long retention and the default timeout, closing the fixture
/// afterwards.
pub async fn store_test<B: FixtureFactory>(backend: &B, block: impl AsyncFnOnce(&mut Env<B::Fixture>)) {
    store_test_with(backend, backend.long_retention(), DEFAULT_TIMEOUT, block).await
}

/// Runs `block` against a fresh fixture that keeps outcomes for `retention`, failing after `timeout`, and closes the
/// fixture afterwards.
pub async fn store_test_with<B: FixtureFactory>(
    backend: &B,
    retention: Duration,
    timeout: Duration,
    block: impl AsyncFnOnce(&mut Env<B::Fixture>),
) {
    let fixture = backend.create(retention).await;
    let mut env = Env::new(fixture, backend.tick());
    if tokio::time::timeout(timeout, block(&mut env)).await.is_err() {
        panic!("the case did not finish within {timeout:?}");
    }
    env.fixture.close().await;
}

/// Skips the rest of a case unless `condition` holds: returns `false` after printing why.
pub fn assume(condition: bool, reason: &str) -> bool {
    if !condition {
        eprintln!("skipped: {reason}");
    }
    condition
}

/// Helpers available inside a case.
pub struct Env<F: StoreFixture> {
    /// The fixture under test.
    pub fixture: F,
    /// The suite's unit of time.
    pub tick: Duration,
    counter: AtomicU64,
}

impl<F: StoreFixture> Env<F> {
    /// An environment over `fixture`.
    pub fn new(fixture: F, tick: Duration) -> Self {
        Self { fixture, tick, counter: AtomicU64::new(0) }
    }

    /// The store under test (a new handle after a reopen).
    pub fn store(&self) -> Arc<dyn Store> {
        self.fixture.store()
    }

    /// The default lease TTL used by the cases.
    pub fn ttl(&self) -> Duration {
        self.tick * 4
    }

    /// `n` ticks.
    pub fn ticks(&self, n: u32) -> Duration {
        self.tick * n
    }

    /// False (after printing why) unless time is virtual (exact boundaries); the case then returns.
    pub fn require_virtual_time(&self) -> bool {
        assume(self.fixture.is_virtual_time(), "needs a virtual store clock")
    }

    /// Advances store time by `n` ticks.
    pub async fn advance_ticks(&self, n: u32) {
        self.fixture.advance(self.ticks(n)).await
    }

    /// Advances store time by `duration`.
    pub async fn advance(&self, duration: Duration) {
        self.fixture.advance(duration).await
    }

    /// Advances store time past the expiry of a lease taken with [`Self::ttl`].
    pub async fn expire_lease(&self) {
        self.fixture.advance(self.ttl() + self.tick).await
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Creates a new instance of `ty` with `state` and returns its address.
    pub async fn new_actor(&self, ty: &TypeName, state: Value) -> ActorAddress {
        let id = format!("a{}", self.next());
        self.new_actor_named(ty, &id, state).await
    }

    /// Registers `ty`, so that instances of it can be created; idempotent. Does nothing to a registration a case made.
    pub async fn register_type(&self, ty: &TypeName) {
        self.store().register(type_def(ty, "fixture")).await.unwrap();
    }

    /// Creates the instance of `ty` with the key `id` and `state`, and returns its address. Registers `ty` first.
    pub async fn new_actor_named(&self, ty: &TypeName, id: &str, state: Value) -> ActorAddress {
        self.register_type(ty).await;
        let address = ActorAddress::of(ty, [id]).expect("a valid address");
        assert_eq!(self.store().create(&address, state).await.unwrap(), CreateResult::Created);
        address
    }

    /// Enqueues a message of type `Msg` for `to`, due now, and asserts that it was accepted.
    pub async fn send(&self, to: &ActorAddress) -> Message {
        self.send_at(to, Delivery::Now).await
    }

    /// Enqueues a message for `to` with `delivery` and asserts that it was accepted.
    pub async fn send_at(&self, to: &ActorAddress, delivery: Delivery) -> Message {
        let payload = json!(self.next());
        enqueue_ok(&*self.store(), OutgoingMessage { delivery, ..OutgoingMessage::new(to.clone(), "Msg", payload) })
            .await
    }

    /// Enqueues a message with `payload` for `to` and asserts that it was accepted.
    pub async fn send_payload(&self, to: &ActorAddress, payload: Value) -> Message {
        enqueue_ok(&*self.store(), OutgoingMessage::new(to.clone(), "Msg", payload)).await
    }

    /// Enqueues a message with `id` for `to` and asserts that it was accepted.
    pub async fn send_id(&self, to: &ActorAddress, id: MessageId) -> Message {
        let payload = json!(self.next());
        enqueue_ok(&*self.store(), OutgoingMessage { id, ..OutgoingMessage::new(to.clone(), "Msg", payload) }).await
    }

    /// Claims due turns of [`TYPE`] for [`NODE_A`] with the default TTL, at most 10.
    pub async fn claim(&self) -> Vec<Claim> {
        self.claim_with(&[&TYPE], &NODE_A, self.ttl(), 10).await
    }

    /// Claims due turns of [`TYPE`] for `node`.
    pub async fn claim_node(&self, node: &NodeId) -> Vec<Claim> {
        self.claim_with(&[&TYPE], node, self.ttl(), 10).await
    }

    /// Claims due turns of `types` for [`NODE_A`].
    pub async fn claim_types(&self, types: &[&TypeName]) -> Vec<Claim> {
        self.claim_with(types, &NODE_A, self.ttl(), 10).await
    }

    /// Claims at most `limit` due turns of [`TYPE`] for [`NODE_A`].
    pub async fn claim_limit(&self, limit: usize) -> Vec<Claim> {
        self.claim_with(&[&TYPE], &NODE_A, self.ttl(), limit).await
    }

    /// Claims due turns.
    pub async fn claim_with(&self, types: &[&TypeName], node: &NodeId, ttl: Duration, limit: usize) -> Vec<Claim> {
        claim_on(&*self.store(), types, node, ttl, limit).await
    }

    /// Claims and asserts that exactly one turn is returned.
    pub async fn claim_one(&self) -> Claim {
        single(self.claim().await)
    }

    /// [`Self::claim_one`] for `node`.
    pub async fn claim_one_node(&self, node: &NodeId) -> Claim {
        single(self.claim_node(node).await)
    }

    /// [`Self::claim_one`] of `types`.
    pub async fn claim_one_types(&self, types: &[&TypeName]) -> Claim {
        single(self.claim_types(types).await)
    }

    /// [`Self::claim_one`] with lease `ttl`.
    pub async fn claim_one_ttl(&self, ttl: Duration) -> Claim {
        single(self.claim_with(&[&TYPE], &NODE_A, ttl, 10).await)
    }

    /// Commits; a store error fails the case.
    pub async fn commit(&self, request: CommitRequest) -> CommitResult {
        self.store().commit(request).await.unwrap()
    }

    /// Commits and asserts that it was committed.
    pub async fn commit_ok(&self, request: CommitRequest) -> Committed {
        committed(self.commit(request).await)
    }

    /// The first page (default size) of the pending messages of `address`.
    pub async fn pending(&self, address: &ActorAddress) -> Vec<Message> {
        self.store().pending(address, None, Page::<Message>::DEFAULT_SIZE).await.unwrap().items
    }

    /// All pending messages of `address`, following every page of `page_size`.
    pub async fn all_pending(&self, address: &ActorAddress, page_size: usize) -> Vec<Message> {
        let store = self.store();
        let mut out = Vec::new();
        let mut cursor: Option<PageCursor> = None;
        loop {
            let page = store.pending(address, cursor.as_ref(), page_size).await.unwrap();
            out.extend(page.items);
            match page.next {
                Some(next) => cursor = Some(next),
                None => return out,
            }
        }
    }

    /// Runs `count` copies of `block` concurrently on the runtime's worker threads, each with its own peer handle,
    /// and returns their results in order. A panic in one copy fails the case.
    pub async fn concurrently<T, Fut>(&self, count: usize, block: impl Fn(usize, Arc<dyn Store>) -> Fut) -> Vec<T>
    where
        Fut: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let mut peers = Vec::with_capacity(count);
        for _ in 0..count {
            peers.push(self.fixture.peer().await);
        }
        let handles: Vec<_> = peers.into_iter().enumerate().map(|(i, peer)| tokio::spawn(block(i, peer))).collect();
        let mut out = Vec::with_capacity(count);
        for handle in handles {
            match handle.await {
                Ok(value) => out.push(value),
                Err(e) => match e.try_into_panic() {
                    Ok(panic) => std::panic::resume_unwind(panic),
                    Err(e) => panic!("concurrent task failed: {e}"),
                },
            }
        }
        out
    }
}

/// What a successful commit returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Committed {
    /// The instance's version after the commit.
    pub version: i64,
    /// The commit's sends as enqueued.
    pub sent: Vec<Message>,
    /// The commit's cancels that took effect.
    pub cancelled: Vec<MessageId>,
}

/// Asserts that `result` is committed.
pub fn committed(result: CommitResult) -> Committed {
    match result {
        CommitResult::Committed { version, sent, cancelled } => Committed { version, sent, cancelled },
        other => panic!("expected Committed, got {other:?}"),
    }
}

/// A commit of `claim` with outcome `done` and no effects.
pub fn done(claim: &Claim) -> CommitRequest {
    ending(claim, TurnOutcome::Done)
}

/// A commit of `claim` with `outcome` and no effects.
pub fn ending(claim: &Claim, outcome: TurnOutcome) -> CommitRequest {
    CommitRequest::new(claim.actor.clone(), claim.epoch, claim.message.id.clone(), outcome)
}

/// Enqueues `message` on `store` and asserts that it was accepted.
pub async fn enqueue_ok(store: &dyn Store, message: OutgoingMessage) -> Message {
    let to = message.to.clone();
    match store.enqueue(message).await.unwrap() {
        EnqueueResult::Enqueued(m) => m,
        other => panic!("enqueue to {to}: expected Enqueued, got {other:?}"),
    }
}

/// The retained, visible outcome of `id`: the record of a [`MessageStatus::Finished`] status; `None` while the message is
/// pending or running, and when it is unknown, hidden or past retention.
pub async fn outcome_record(store: &dyn Store, id: &MessageId) -> Option<OutcomeRecord> {
    match store.status(id).await.unwrap() {
        Some(MessageStatus::Finished(outcome)) => Some(outcome),
        _ => None,
    }
}

/// Claims due turns on `store`.
pub async fn claim_on(
    store: &dyn Store,
    types: &[&TypeName],
    node: &NodeId,
    ttl: Duration,
    limit: usize,
) -> Vec<Claim> {
    store.claim_due(&type_set(types), node, ttl, limit).await.unwrap()
}

/// Asserts that `claims` holds exactly one claim and returns it.
pub fn single(mut claims: Vec<Claim>) -> Claim {
    assert_eq!(claims.len(), 1, "expected exactly one claim, got {claims:?}");
    claims.remove(0)
}

/// The set of `types`.
pub fn type_set(types: &[&TypeName]) -> BTreeSet<TypeName> {
    types.iter().map(|t| (*t).clone()).collect()
}

/// A type definition for `name` with contract hash `hash`.
pub fn type_def(name: &TypeName, hash: &str) -> TypeDef {
    TypeDef {
        name: name.clone(),
        contract: json!({"type": name.as_str(), "hash": hash}),
        contract_hash: hash.to_owned(),
    }
}

/// Parses a type name.
pub fn type_name(text: &str) -> TypeName {
    TypeName::parse(text).expect("a valid type name")
}

/// Parses an address.
pub fn address(text: &str) -> ActorAddress {
    ActorAddress::parse(text).expect("a valid address")
}

/// A node id.
pub fn node(id: &str) -> NodeId {
    NodeId::new(id).expect("a valid node id")
}

/// `instant + duration`.
pub fn plus(instant: Timestamp, duration: Duration) -> Timestamp {
    time::add(instant, duration).expect("in range")
}

/// `instant - duration`.
pub fn minus(instant: Timestamp, duration: Duration) -> Timestamp {
    time::sub(instant, duration)
}

/// The ids of `messages`.
pub fn ids(messages: &[Message]) -> Vec<MessageId> {
    messages.iter().map(|m| m.id.clone()).collect()
}

/// Asserts that `list` is sorted by delivery order `(deliver_at, seq)`.
pub fn assert_delivery_order(list: &[Message]) {
    assert!(
        list.windows(2).all(|w| (w[0].deliver_at, w[0].seq) < (w[1].deliver_at, w[1].seq)),
        "not in (deliverAt, seq) order: {:?}",
        list.iter().map(|m| (m.deliver_at, m.seq)).collect::<Vec<_>>()
    );
}
