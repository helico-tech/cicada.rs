//! The turn's task structure: the worker (handler and hooks), the lease keeper (heartbeat and watchdog) and the
//! overtime warning are separate tasks, so one failing child never cancels another; each child's failure is handled
//! explicitly. A keeper that fails unexpectedly means the lease is no longer known to be held: the worker is cancelled
//! and nothing is committed or released (Kotlin `TurnStructureTest`).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use cicada::address::ActorAddress;
use cicada::prelude::*;
use cicada::store::{CommitRequest, CommitResult, MessageStatus, OutcomeStatus, RenewResult, Store, StoreResult};
use cicada::{ActorSystem, ActorType};
use common::*;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sleeper {
    id: String,
}

impl ActorRef for Sleeper {
    const TEMPLATE: &'static str = "actor://sleeper/{id}";
    type Message = SleeperMsg;
}

fn s() -> Sleeper {
    Sleeper { id: "s".into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SleeperMsg {
    /// Sleeps for `seconds` (cancellable).
    Slumber { seconds: u64 },
}

#[derive(Default)]
struct Counts {
    runs: AtomicUsize,
    cancelled: AtomicUsize,
}

/// Counts a cancellation when dropped before it is disarmed.
struct CancelProbe<'a> {
    counts: &'a Counts,
    armed: bool,
}

impl Drop for CancelProbe<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.counts.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
}

struct SleeperActor(Arc<Counts>);

impl Actor for SleeperActor {
    type Ref = Sleeper;
    type State = Stateless;
    type Message = SleeperMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _turn: &mut Turn<Self>, message: SleeperMsg) -> HandlerResult {
        let SleeperMsg::Slumber { seconds } = message;
        self.0.runs.fetch_add(1, Ordering::SeqCst);
        let mut probe = CancelProbe { counts: &self.0, armed: true };
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        probe.armed = false;
        Ok(())
    }
}

fn sleeper_node(store: Arc<dyn Store>, name: &str, counts: &Arc<Counts>) -> ActorSystem {
    node(store, name).host(ActorType::new(SleeperActor(counts.clone())).max_turn(Duration::from_secs(300))).build()
}

/// Renewal panics while `broken` is set: an unexpected failure of the store's renewal, not a failed renewal (Kotlin
/// throws an `Error`).
struct BrokenRenew {
    broken: AtomicBool,
    slow_commit: bool,
}

#[async_trait]
impl Faults for BrokenRenew {
    async fn renew(
        &self,
        inner: &dyn Store,
        address: &ActorAddress,
        epoch: i64,
        ttl: Duration,
    ) -> StoreResult<RenewResult> {
        if self.broken.load(Ordering::SeqCst) {
            panic!("renew bug");
        }
        inner.renew(address, epoch, ttl).await
    }

    async fn commit(&self, inner: &dyn Store, commit: CommitRequest) -> StoreResult<CommitResult> {
        if self.slow_commit {
            // The first heartbeat (10 s) fails while the commit is in flight.
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
        inner.commit(commit).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_lease_keeper_that_fails_unexpectedly_cancels_the_handler_and_abandons_the_turn_without_release() {
    let counts = Arc::new(Counts::default());
    let memory = virtual_store();
    let faulty = Faulty::new(memory.clone(), BrokenRenew { broken: AtomicBool::new(true), slow_commit: false });
    let n1 = sleeper_node(faulty, "n1", &counts);
    let n2 = sleeper_node(memory.clone(), "n2", &counts);
    n1.create_stateless(&s()).await.unwrap();
    let m = accepted(n1.send(&s(), SleeperMsg::Slumber { seconds: 20 }).await.unwrap());

    // The first heartbeat (10 s) panics: the turn ends without the panic escaping to the caller.
    let started = Instant::now();
    assert_eq!(n1.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(counts.runs.load(Ordering::SeqCst), 1);
    assert_eq!(counts.cancelled.load(Ordering::SeqCst), 1, "the handler was cancelled when the keeper failed");
    assert_eq!(started.elapsed(), Duration::from_secs(10), "at the failing heartbeat, not later");
    assert!(
        matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })),
        "not released and nothing committed"
    );
    assert_eq!(n2.run_due_turns(1).await.unwrap(), 0, "the lease still holds");

    // After the lease expires the message is handed over, as after a node crash: the handler never ended.
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(n2.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(counts.runs.load(Ordering::SeqCst), 2);
    assert_eq!(memory.outcome(&m.id).await.unwrap().map(|o| o.status), Some(OutcomeStatus::Done));
}

#[tokio::test(start_paused = true)]
async fn with_a_working_keeper_the_same_turn_runs_to_its_outcome() {
    let counts = Arc::new(Counts::default());
    let memory = virtual_store();
    let faulty = Faulty::new(memory.clone(), BrokenRenew { broken: AtomicBool::new(false), slow_commit: false });
    let n1 = sleeper_node(faulty, "n1", &counts);
    n1.create_stateless(&s()).await.unwrap();
    // Longer than the TTL: needs renewals.
    let m = accepted(n1.send(&s(), SleeperMsg::Slumber { seconds: 45 }).await.unwrap());
    assert_eq!(n1.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(counts.cancelled.load(Ordering::SeqCst), 0);
    assert_eq!(memory.outcome(&m.id).await.unwrap().map(|o| o.status), Some(OutcomeStatus::Done));
}

#[tokio::test(start_paused = true)]
async fn a_keeper_that_fails_during_the_commit_does_not_cancel_it_the_stores_fencing_decides() {
    let counts = Arc::new(Counts::default());
    let memory = virtual_store();
    let faulty = Faulty::new(memory.clone(), BrokenRenew { broken: AtomicBool::new(true), slow_commit: true });
    let n1 = sleeper_node(faulty, "n1", &counts);
    n1.create_stateless(&s()).await.unwrap();
    let m = accepted(n1.send(&s(), SleeperMsg::Slumber { seconds: 1 }).await.unwrap());
    assert_eq!(n1.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(counts.cancelled.load(Ordering::SeqCst), 0);
    assert_eq!(
        memory.outcome(&m.id).await.unwrap().map(|o| o.status),
        Some(OutcomeStatus::Done),
        "within the lease: the store accepted it"
    );
}
