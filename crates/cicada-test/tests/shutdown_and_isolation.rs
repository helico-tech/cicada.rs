//! Handler isolation from heartbeats, and shutdown that never hangs or leaks leases. Real time (Kotlin
//! `ShutdownAndIsolationTest`).
//!
//! Kotlin runs handlers on a dispatcher of their own and has handlers that ignore cancellation by suspending in
//! `NonCancellable`. A Rust future cannot ignore being dropped, so here the handler that ignores cancellation blocks its
//! thread; heartbeats stay alive on the other worker threads of a multi-thread runtime.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use cicada::address::ActorAddress;
use cicada::prelude::*;
use cicada::runtime::NodeState;
use cicada::store::{MessageStatus, NodeId, OutcomeStatus, RenewResult, Store, StoreResult};
use cicada::ActorSystem;
use cicada_test::InMemoryStore;
use common::*;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Blocker {
    id: String,
}

impl ActorRef for Blocker {
    const TEMPLATE: &'static str = "actor://blocker/{id}";
    type Message = BlockerMsg;
}

fn blocker(id: &str) -> Blocker {
    Blocker { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum BlockerMsg {
    /// Blocks its thread for `millis` (e.g. a blocking JDBC or HTTP call); it ignores cancellation meanwhile.
    Block { millis: u64 },
    /// Sleeps for `millis`, cancellable.
    Nap { millis: u64 },
}

struct BlockerActor;

impl Actor for BlockerActor {
    type Ref = Blocker;
    type State = Stateless;
    type Message = BlockerMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _turn: &mut Turn<Self>, message: BlockerMsg) -> HandlerResult {
        match message {
            BlockerMsg::Block { millis } => std::thread::sleep(Duration::from_millis(millis)),
            BlockerMsg::Nap { millis } => tokio::time::sleep(Duration::from_millis(millis)).await,
        }
        Ok(())
    }
}

#[derive(Default)]
struct CountRenewals(AtomicUsize);

#[async_trait]
impl Faults for CountRenewals {
    async fn renew(&self, inner: &dyn Store, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        inner.renew(address, epoch, ttl).await
    }
}

const TEN_SECONDS: Duration = Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blocking_handler_does_not_starve_the_heartbeat() {
    let memory = Arc::new(InMemoryStore::default());
    let store = Faulty::new(memory.clone(), CountRenewals::default());
    let node = ActorSystem::builder(store.clone())
        .node_id(NodeId::new("blocking").unwrap())
        .host(BlockerActor)
        .lease_ttl(Duration::from_millis(600))
        .heartbeat_interval(Duration::from_millis(150))
        .poll_interval(Duration::from_millis(20))
        .build();
    let other = ActorSystem::builder(memory.clone())
        .node_id(NodeId::new("other").unwrap())
        .host(BlockerActor)
        .lease_ttl(Duration::from_millis(600))
        .poll_interval(Duration::from_millis(20))
        .build();
    node.create_stateless(&blocker("b")).await.unwrap();
    node.start().await.unwrap();
    let m = accepted(node.send(&blocker("b"), BlockerMsg::Block { millis: 1_500 }).await.unwrap());
    eventually(TEN_SECONDS, || async { matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })) })
        .await;
    other.start().await.unwrap();
    eventually(TEN_SECONDS, || async { memory.outcome(&m.id).await.unwrap().is_some() }).await;
    let outcome = memory.outcome(&m.id).await.unwrap().unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!(outcome.envelope.attempt, 1, "the lease never expired, so no other node ran the turn");
    let renewals = store.faults.0.load(Ordering::SeqCst);
    assert!(renewals >= 3, "heartbeats kept renewing while the handler blocked ({renewals})");
    node.stop_with_grace(Duration::ZERO).await;
    other.stop_with_grace(Duration::ZERO).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_does_not_hang_on_a_handler_that_ignores_cancellation() {
    let memory = Arc::new(InMemoryStore::default());
    let node = ActorSystem::builder(memory.clone())
        .host(BlockerActor)
        .poll_interval(Duration::from_millis(20))
        .shutdown_cancel_timeout(Duration::from_millis(200))
        .build();
    node.create_stateless(&blocker("s")).await.unwrap();
    node.start().await.unwrap();
    // Kotlin: 20 s suspended in NonCancellable. A blocked thread holds up the runtime's shutdown at the end of the test,
    // so it blocks for 3 s only.
    let m = accepted(node.send(&blocker("s"), BlockerMsg::Block { millis: 3_000 }).await.unwrap());
    eventually(TEN_SECONDS, || async { matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })) })
        .await;
    let mark = Instant::now();
    node.stop_with_grace(Duration::ZERO).await;
    assert!(mark.elapsed() < Duration::from_secs(2), "stop returned after {:?}", mark.elapsed());
    assert_eq!(node.state(), NodeState::Stopped);
}

#[tokio::test]
async fn stopping_right_after_a_claim_never_leaves_a_lease_behind() {
    for round in 0..30 {
        let memory = Arc::new(InMemoryStore::default());
        let node = ActorSystem::builder(memory.clone()).host(BlockerActor).poll_interval(Duration::from_millis(5)).build();
        let r = blocker(&format!("r{round}"));
        node.create_stateless(&r).await.unwrap();
        let m = accepted(node.send(&r, BlockerMsg::Nap { millis: 10_000 }).await.unwrap());
        node.start().await.unwrap();
        eventually(TEN_SECONDS, || async {
            !matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(_))) || node.running_turns() > 0
        })
        .await;
        node.stop_with_grace(Duration::ZERO).await;
        let status = memory.status(&m.id).await.unwrap();
        assert!(
            matches!(status, Some(MessageStatus::Pending(_))),
            "round {round}: the lease was released on stop, was {status:?}"
        );
        memory.close().await;
    }
}
