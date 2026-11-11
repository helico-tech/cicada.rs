//! Runs actor types against an in-memory store with virtual time.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use crate::address::{ActorAddress, ActorRef};
use crate::runtime::{ActorSystemBuilder, CreateActorResult, SendResult, StartError};
use crate::store::{Message, MessageStatus, OutcomeRecord, OutcomeStatus, PageCursor, Store};
use crate::time::{self, Timestamp};
use crate::{ActorSystem, SendOptions};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::testing::clock::{TokioClock, default_start};
use crate::testing::memory::InMemoryStore;

const DEFAULT_MAX_TURNS: usize = 10_000;

type Configure = Box<dyn FnOnce(ActorSystemBuilder) -> ActorSystemBuilder>;

/// Builds a [`TestActors`].
pub struct TestActorsBuilder {
    start: Timestamp,
    retention: Duration,
    configure: Vec<Configure>,
}

impl TestActorsBuilder {
    /// Hosts and configures through the node builder: `.with(|node| node.host(SyncActor).config(..))`.
    pub fn with(mut self, configure: impl FnOnce(ActorSystemBuilder) -> ActorSystemBuilder + 'static) -> Self {
        self.configure.push(Box::new(configure));
        self
    }

    /// The virtual time at which the test begins; 2026-01-01T00:00:00Z by default.
    pub fn start_at(mut self, start: Timestamp) -> Self {
        self.start = start;
        self
    }

    /// The store's outcome retention.
    pub fn retention(mut self, retention: Duration) -> Self {
        self.retention = retention;
        self
    }

    /// Creates the store and the node, and initializes the node (validating its types and registering their
    /// contracts). The node is not started: turns run only when the test drives them.
    pub async fn build(self) -> Result<TestActors, StartError> {
        let clock = Arc::new(TokioClock::new(self.start));
        let store = Arc::new(InMemoryStore::new(clock.clone(), self.retention));
        let mut builder = ActorSystem::builder(store.clone());
        for configure in self.configure {
            builder = configure(builder);
        }
        let system = builder.build();
        system.initialize().await?;
        Ok(TestActors { system, store, clock })
    }
}

/// Runs actor types against an [`InMemoryStore`] with virtual time.
///
/// Nothing runs in the background: [`Self::run_until_idle`] and [`Self::advance_time_by`] drive turns in the test's
/// task, so tests are deterministic. Time is tokio's paused clock: run the test with
/// `#[tokio::test(start_paused = true)]`. The store clock, handler sleeps, heartbeats and turn timeouts all use it, and
/// none of them waits in real time.
///
/// ```
/// # use cicada::prelude::*;
/// # use serde::{Deserialize, Serialize};
/// # #[derive(Debug, Clone, Serialize, Deserialize)]
/// # struct Counter { id: String }
/// # impl ActorRef for Counter { const TYPE: &'static str = "counter"; type Message = CounterMsg; }
/// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// # #[serde(tag = "type")]
/// # enum CounterMsg { Add { amount: i64 } }
/// # #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
/// # struct CounterState { total: i64 }
/// # struct CounterActor;
/// # impl Actor for CounterActor {
/// #     type Ref = Counter; type State = CounterState; type Message = CounterMsg;
/// #     const UNHANDLED: Unhandled = Unhandled::Fail;
/// #     async fn handle(&self, turn: &mut Turn<Self>, m: CounterMsg) -> HandlerResult {
/// #         let CounterMsg::Add { amount } = m; turn.state_mut().total += amount; Ok(())
/// #     }
/// # }
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() {
/// use cicada::testing::TestActors;
///
/// let test = TestActors::builder().with(|node| node.host(CounterActor)).build().await.unwrap();
/// let counter = Counter { id: "c1".into() };
/// test.create(&counter, &CounterState::default()).await;
/// test.send(&counter, CounterMsg::Add { amount: 2 }).await;
/// test.run_until_idle().await;
/// assert_eq!(test.state::<_, CounterState>(&counter).await.total, 2);
/// # }
/// ```
pub struct TestActors {
    system: ActorSystem,
    store: Arc<InMemoryStore>,
    clock: Arc<TokioClock>,
}

impl TestActors {
    /// A builder; see [`TestActorsBuilder`].
    pub fn builder() -> TestActorsBuilder {
        TestActorsBuilder { start: default_start(), retention: crate::time::DEFAULT_RETENTION, configure: Vec::new() }
    }

    /// The node under test. It is initialized but not started.
    pub fn system(&self) -> &ActorSystem {
        &self.system
    }

    /// The in-memory store.
    pub fn store(&self) -> &Arc<InMemoryStore> {
        &self.store
    }

    /// The store clock, for dependencies that need one.
    pub fn clock(&self) -> &Arc<TokioClock> {
        &self.clock
    }

    /// Current virtual store time.
    pub fn now(&self) -> Timestamp {
        time::truncate(crate::store::Clock::now(&*self.clock))
    }

    /// Creates `r` with `initial_state`; panics when the create is refused.
    pub async fn create<R: ActorRef, S: Serialize + ?Sized>(&self, r: &R, initial_state: &S) -> CreateActorResult {
        let result = self.system.create(r, initial_state).await.expect("create failed");
        assert!(
            matches!(result, CreateActorResult::Created | CreateActorResult::Exists(_)),
            "Create of {:?} refused: {result:?}",
            r.address()
        );
        result
    }

    /// Sends `msg` to `to` from outside an actor; returns the accepted message and panics when the send is refused.
    pub async fn send<R: ActorRef>(&self, to: &R, msg: impl Into<R::Message>) -> Message {
        self.send_with(to, msg, SendOptions::default()).await
    }

    /// Sends `msg` to `to` with `options`; returns the accepted message and panics when the send is refused.
    pub async fn send_with<R: ActorRef>(&self, to: &R, msg: impl Into<R::Message>, options: SendOptions) -> Message {
        match self.system.send_with(to, msg, options).await.expect("send failed") {
            SendResult::Accepted(message) => message,
            other => panic!("Send to {:?} refused: {other:?}", to.address()),
        }
    }

    /// Sends a raw message; returns the raw result (including refusals) like the HTTP API.
    pub async fn send_raw(
        &self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: SendOptions,
    ) -> SendResult {
        self.system.send_message(to, message_type, payload, options).await.expect("send failed")
    }

    /// Handles every message that is due now, including those that become due because of these turns, without moving
    /// time (beyond what handlers themselves sleep). Panics when more than 10 000 turns were needed (an actor sending
    /// to itself without delay). Returns the number of turns run.
    pub async fn run_until_idle(&self) -> usize {
        self.run_until_idle_max(DEFAULT_MAX_TURNS).await
    }

    /// [`Self::run_until_idle`] with a custom turn limit.
    pub async fn run_until_idle_max(&self, max_turns: usize) -> usize {
        let mut turns = 0;
        loop {
            let ran = self.system.run_due_turns(1).await.expect("running turns failed");
            if ran == 0 {
                return turns;
            }
            turns += ran;
            assert!(
                turns <= max_turns,
                "Not idle after {max_turns} turns; an actor may be sending to itself without delay"
            );
        }
    }

    /// Moves virtual time forward by `duration`. Every message that becomes due on the way is handled at its due time,
    /// so an hourly self-send runs twice in `advance_time_by(2 hours)`. Returns the number of turns run.
    pub async fn advance_time_by(&self, duration: Duration) -> usize {
        let target = time::add(self.now(), duration).expect("virtual time stays in range");
        let mut turns = self.run_until_idle().await;
        let types: BTreeSet<_> = self.system.hosted_types();
        loop {
            let next = self.store.next_due_at(&types).await.expect("store failed");
            match next {
                Some(next) if next <= target => {
                    let now = self.now();
                    if next > now {
                        tokio::time::sleep(time::between(now, next)).await;
                    }
                    turns += self.run_until_idle_max(DEFAULT_MAX_TURNS - turns).await;
                }
                _ => break,
            }
        }
        let now = self.now();
        if target > now {
            tokio::time::sleep(time::between(now, target)).await;
        }
        turns + self.run_until_idle_max(DEFAULT_MAX_TURNS - turns).await
    }

    /// All pending messages of `r` in delivery order; empty when the instance is absent.
    pub async fn pending<R: ActorRef>(&self, r: &R) -> Vec<Message> {
        self.pending_at(&r.address().expect("a valid ref")).await
    }

    /// All pending messages of `address` in delivery order; empty when the instance is absent.
    pub async fn pending_at(&self, address: &ActorAddress) -> Vec<Message> {
        let mut out = Vec::new();
        let mut cursor: Option<PageCursor> = None;
        loop {
            let page = self.store.pending(address, cursor.as_ref(), 100).await.expect("store failed");
            out.extend(page.items);
            match page.next {
                Some(next) => cursor = Some(next),
                None => return out,
            }
        }
    }

    /// All instance addresses starting with `prefix` (a prefix of the text form, such as `sync/`), in address order.
    pub async fn list(&self, prefix: &str) -> Vec<ActorAddress> {
        let mut out = Vec::new();
        let mut cursor: Option<PageCursor> = None;
        loop {
            let page = self.store.list(prefix, cursor.as_ref(), 100).await.expect("store failed");
            out.extend(page.items);
            match page.next {
                Some(next) => cursor = Some(next),
                None => return out,
            }
        }
    }

    /// True when `r` exists.
    pub async fn exists<R: ActorRef>(&self, r: &R) -> bool {
        self.store.get(&r.address().expect("a valid ref")).await.expect("store failed").is_some()
    }

    /// The raw state of `r`, or `None` when it is absent.
    pub async fn state_json<R: ActorRef>(&self, r: &R) -> Option<Value> {
        self.store.get(&r.address().expect("a valid ref")).await.expect("store failed").map(|rec| rec.state)
    }

    /// The state of `r` decoded as `S`; panics when the instance is absent.
    pub async fn state<R: ActorRef, S: DeserializeOwned>(&self, r: &R) -> S {
        let address = r.address().expect("a valid ref");
        let record =
            self.store.get(&address).await.expect("store failed").unwrap_or_else(|| panic!("{address} does not exist"));
        serde_json::from_value(record.state).expect("state decodes")
    }

    /// The state version of `r`; panics when the instance is absent.
    pub async fn version<R: ActorRef>(&self, r: &R) -> i64 {
        let address = r.address().expect("a valid ref");
        self.store
            .get(&address)
            .await
            .expect("store failed")
            .unwrap_or_else(|| panic!("{address} does not exist"))
            .version
    }

    /// The outcome of `message`, or `None` while it is pending.
    pub async fn outcome(&self, message: &Message) -> Option<OutcomeRecord> {
        match self.store.status(&message.id).await.expect("store failed") {
            Some(MessageStatus::Finished(outcome)) => Some(outcome),
            _ => None,
        }
    }

    /// All retained failed outcomes, oldest first.
    pub async fn failures(&self) -> Vec<OutcomeRecord> {
        let mut out = Vec::new();
        let mut cursor: Option<PageCursor> = None;
        loop {
            let page = self
                .store
                .outcomes(
                    Some(OutcomeStatus::Failed),
                    "",
                    chrono::DateTime::<chrono::Utc>::MIN_UTC,
                    cursor.as_ref(),
                    100,
                )
                .await
                .expect("store failed");
            out.extend(page.items);
            match page.next {
                Some(next) => cursor = Some(next),
                None => return out,
            }
        }
    }
}
