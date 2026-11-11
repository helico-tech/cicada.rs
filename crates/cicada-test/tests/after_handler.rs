//! Once a handler has returned or failed, the runtime never releases the lease for a re-run (G6). Every unexpected
//! failure after the handler ends in a failed outcome, or in abandoning the turn without release; a failure before the
//! handler fails the message instead of crash-looping it. Faults are injected at each point after the handler: the
//! typed-effects checks (store reads), encoding the state, the commit, the failure commit, and cancellation.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use cicada::address::{ActorAddress, TypeName};
use cicada::plugin::{ClaimContext, HookResult, Plugin};
use cicada::prelude::*;
use cicada::runtime::{CreateActorResult, NodeConfig, OpError};
use cicada::store::{
    Claim, CommitRequest, CommitResult, Message, MessageStatus, NodeId, OutcomeStatus, OutgoingMessage, Store,
    StoreError, StoreResult, TurnOutcome, TypeDef,
};
use cicada::{ActorSystem, ActorType};
use cicada_test::InMemoryStore;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Notify;

/// `tally/{id}`; the id `cursed` is refused when a ref is decoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "RawTally")]
struct Tally {
    id: String,
}

#[derive(Deserialize)]
struct RawTally {
    id: String,
}

impl TryFrom<RawTally> for Tally {
    type Error = String;

    fn try_from(raw: RawTally) -> Result<Self, String> {
        if raw.id == "cursed" {
            return Err("the id 'cursed' is not allowed".into());
        }
        Ok(Tally { id: raw.id })
    }
}

impl ActorRef for Tally {
    const TYPE: &'static str = "tally";
    type Message = TallyMsg;
}

fn tally(id: &str) -> Tally {
    Tally { id: id.into() }
}

fn t() -> Tally {
    tally("t")
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum TallyMsg {
    /// Counts a run and changes state.
    Bump,
    /// Counts a run, creates `child` and sends it a `Bump`.
    Hatch { child: String },
    /// Counts a run and sends `Bump` to `target`, which does not exist: the commit is rejected.
    Poke { target: String },
    /// Counts a run and stores a note containing a NUL character.
    Nul,
    /// Blocks its thread for `millis` (ignoring cancellation), then changes state.
    Stall { millis: u64 },
    /// Sends `Bump` to itself with `headers` (`<NUL>` stands for a NUL character); a refused send is noted in the state.
    Tag { headers: BTreeMap<String, String> },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct TallyState {
    #[serde(default)]
    runs: u32,
    #[serde(default)]
    note: String,
}

/// The tally type; its maximum turn is 5 minutes unless given.
struct TallyActor<const MAX_TURN_MILLIS: u64 = 300_000>(Arc<AtomicI32>);

impl<const MAX_TURN_MILLIS: u64> Actor for TallyActor<MAX_TURN_MILLIS> {
    type Ref = Tally;
    type State = TallyState;
    type Message = TallyMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;
    const MAX_TURN: Option<Duration> = Some(Duration::from_millis(MAX_TURN_MILLIS));

    async fn handle(&self, turn: &mut Turn<Self>, message: TallyMsg) -> HandlerResult {
        match message {
            TallyMsg::Bump => {
                self.0.fetch_add(1, Ordering::SeqCst);
                turn.state_mut().runs += 1;
            }
            TallyMsg::Hatch { child } => {
                self.0.fetch_add(1, Ordering::SeqCst);
                turn.create(&tally(&child), &TallyState::default())?;
                turn.send(&tally(&child), TallyMsg::Bump)?;
            }
            TallyMsg::Poke { target } => {
                self.0.fetch_add(1, Ordering::SeqCst);
                turn.send(&tally(&target), TallyMsg::Bump)?;
            }
            TallyMsg::Nul => {
                self.0.fetch_add(1, Ordering::SeqCst);
                turn.state_mut().note = "a\u{0}b".into();
            }
            TallyMsg::Stall { millis } => {
                self.0.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(millis));
                turn.state_mut().note = "blocked".into();
            }
            TallyMsg::Tag { headers } => {
                let mut options = SendOptions::new();
                for (k, v) in headers {
                    options = options.header(k, v.replace("<NUL>", "\u{0}"));
                }
                let note = match turn.send_self_with(TallyMsg::Bump, options) {
                    Ok(_) => "sent".to_owned(),
                    Err(e) => format!("refused: {e}"),
                };
                turn.state_mut().note = note;
            }
        }
        Ok(())
    }
}

struct Fixture {
    runs: Arc<AtomicI32>,
}

impl Fixture {
    fn new() -> Self {
        Self { runs: Arc::new(AtomicI32::new(0)) }
    }

    fn runs(&self) -> i32 {
        self.runs.load(Ordering::SeqCst)
    }

    fn tally_type(&self) -> ActorType<TallyActor> {
        ActorType::new(TallyActor(self.runs.clone()))
    }

    fn node(&self, store: Arc<dyn Store>, name: &str) -> ActorSystem {
        node(store, name).host(self.tally_type()).build()
    }

    fn node_with(&self, store: Arc<dyn Store>, name: &str, plugin: impl Plugin) -> ActorSystem {
        node(store, name).host(self.tally_type()).install(plugin).build()
    }
}

async fn accepted_by(system: &ActorSystem, message: TallyMsg) -> Message {
    accepted(system.send(&t(), message).await.unwrap())
}

async fn tally_state(store: &dyn Store, r: &Tally) -> TallyState {
    state_of(store, &addr(r)).await
}

/// Reading the TypeRegistry fails while `failures` is positive (counting down).
#[derive(Default)]
struct FlakyResolve {
    failures: AtomicI32,
}

#[async_trait]
impl Faults for FlakyResolve {
    async fn list_types(&self, inner: &dyn Store) -> StoreResult<Vec<TypeDef>> {
        if self.failures.fetch_sub(1, Ordering::SeqCst) > 0 {
            return Err(StoreError::unavailable("registry unreachable"));
        }
        self.failures.store(0, Ordering::SeqCst);
        inner.list_types().await
    }
}

#[tokio::test(start_paused = true)]
async fn a_store_read_that_keeps_failing_after_the_handler_returned_fails_the_message_instead_of_re_running_it() {
    let f = Fixture::new();
    let memory = virtual_store();
    let flaky = Faulty::new(memory.clone(), FlakyResolve::default());
    let system = f.node(flaky.clone(), "n1");
    system.create(&t(), &TallyState::default()).await.unwrap();
    let m = accepted_by(&system, TallyMsg::Hatch { child: "child".into() }).await;
    // From now on the check of the handler's typed create cannot read the registry.
    flaky.faults.failures.store(i32::MAX, Ordering::SeqCst);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);

    let outcome = outcome_of(&*memory, &m.id).await.expect("the turn ends with an outcome, it is not released");
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    let error = outcome.error.unwrap();
    assert!(error.contains("after its handler ended"), "{error}");
    assert!(error.contains("registry unreachable"), "{error}");
    assert_eq!(f.runs(), 1);
    flaky.faults.failures.store(0, Ordering::SeqCst);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 0, "nothing is re-run");
    assert!(memory.get(&addr(&tally("child"))).await.unwrap().is_none(), "no effects");
}

#[tokio::test(start_paused = true)]
async fn a_store_read_that_fails_briefly_after_the_handler_returned_is_retried() {
    let f = Fixture::new();
    let memory = virtual_store();
    let flaky = Faulty::new(memory.clone(), FlakyResolve::default());
    let system = f.node(flaky.clone(), "n1");
    system.create(&t(), &TallyState::default()).await.unwrap();
    let m = accepted_by(&system, TallyMsg::Hatch { child: "child".into() }).await;
    flaky.faults.failures.store(2, Ordering::SeqCst);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(outcome_of(&*memory, &m.id).await.map(|o| o.status), Some(OutcomeStatus::Done));
    assert_eq!(f.runs(), 1);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1, "the child's Bump");
    assert_eq!(tally_state(&*memory, &tally("child")).await.runs, 1);
}

/// Sleeps in `on_claim`.
struct Stalling;

#[async_trait]
impl Plugin for Stalling {
    fn name(&self) -> &str {
        "Stalling"
    }

    async fn on_claim(&self, _ctx: &ClaimContext<'_>) -> HookResult {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Ok(())
    }
}

/// Reading the TypeRegistry takes 10 s while `stall` is set (a slow database).
#[derive(Default)]
struct StallingRegistry {
    stall: AtomicBool,
}

#[async_trait]
impl Faults for StallingRegistry {
    async fn list_types(&self, inner: &dyn Store) -> StoreResult<Vec<TypeDef>> {
        if self.stall.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        inner.list_types().await
    }
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_turn_after_its_handler_returned_does_not_release_the_lease() {
    let f = Fixture::new();
    let memory = virtual_store();
    let stalling = Faulty::new(memory.clone(), StallingRegistry::default());
    let stale = f.node(stalling.clone(), "stale");
    let other = f.node(memory.clone(), "other");
    stale.create(&t(), &TallyState::default()).await.unwrap();
    let m = accepted_by(&stale, TallyMsg::Hatch { child: "child".into() }).await;
    // From now on the check of the handler's typed create waits for the registry.
    stalling.faults.stall.store(true, Ordering::SeqCst);
    let job = tokio::spawn({
        let stale = stale.clone();
        async move { stale.run_due_turns(1).await }
    });
    tokio::time::sleep(Duration::from_secs(5)).await; // the handler returned; the check of its effects is still running
    job.abort();
    let _ = job.await;
    settle().await;

    assert_eq!(f.runs(), 1);
    assert!(
        matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })),
        "not released: only lease expiry hands it over"
    );
    assert_eq!(other.run_due_turns(1).await.unwrap(), 0);
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_turn_before_its_handler_returned_releases_the_lease() {
    let f = Fixture::new();
    let memory = virtual_store();
    let system = f.node_with(memory.clone(), "n1", Stalling);
    system.create(&t(), &TallyState::default()).await.unwrap();
    let m = accepted_by(&system, TallyMsg::Bump).await;
    let job = tokio::spawn({
        let system = system.clone();
        async move { system.run_due_turns(1).await }
    });
    tokio::time::sleep(Duration::from_secs(5)).await;
    job.abort();
    let _ = job.await;
    settle().await;
    assert_eq!(f.runs(), 0);
    assert!(
        matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(_))),
        "released at once: no handler ran"
    );
}

#[tokio::test(start_paused = true)]
async fn an_error_from_decoding_the_ref_fails_the_message_instead_of_crash_looping_it() {
    let f = Fixture::new();
    let memory = virtual_store();
    let system = f.node(memory.clone(), "n1");
    let cursed = ActorAddress::parse("tally/cursed").unwrap();
    match system.create_actor(&cursed, json!({})).await.unwrap() {
        CreateActorResult::InvalidAddress(e) => assert!(e.contains("not allowed"), "{e}"),
        other => panic!("refused by a hosting node, was {other:?}"),
    }

    // Created where nothing checks the ref (a node that does not host the type).
    memory.create(&cursed, Some(json!({}))).await.unwrap();
    let m = enqueue(&*memory, OutgoingMessage::new(cursed, "Bump", json!({}))).await;
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&*memory, &m.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert!(outcome.error.as_deref().unwrap().contains("not allowed"), "{:?}", outcome.error);
    assert_eq!(f.runs(), 0);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 0);
}

/// The commit of a failure keeps failing.
struct FailingFailureCommit;

#[async_trait]
impl Faults for FailingFailureCommit {
    async fn commit(&self, inner: &dyn Store, commit: CommitRequest) -> StoreResult<CommitResult> {
        if let TurnOutcome::Failed(_) = commit.outcome {
            return Err(StoreError::unavailable("disk full"));
        }
        inner.commit(commit).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_failure_commit_that_keeps_failing_abandons_the_turn_without_releasing_the_lease() {
    let f = Fixture::new();
    let memory = virtual_store();
    let stale = f.node(Faulty::new(memory.clone(), FailingFailureCommit), "stale");
    let other = f.node(memory.clone(), "other");
    stale.create(&t(), &TallyState::default()).await.unwrap();
    let m = accepted_by(&stale, TallyMsg::Poke { target: "nobody".into() }).await; // rejected at commit
    assert_eq!(stale.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(f.runs(), 1);
    assert!(matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })), "not released");
    assert_eq!(other.run_due_turns(1).await.unwrap(), 0);
}

#[tokio::test(start_paused = true)]
async fn new_state_with_a_nul_character_fails_the_turn_on_every_backend() {
    let f = Fixture::new();
    let memory = virtual_store();
    let system = f.node(memory.clone(), "n1");
    system.create(&t(), &TallyState::default()).await.unwrap();
    let m = accepted_by(&system, TallyMsg::Nul).await;
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&*memory, &m.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert!(outcome.error.as_deref().unwrap().contains("NUL"), "{:?}", outcome.error);
    assert_eq!(tally_state(&*memory, &t()).await, TallyState::default());
}

#[tokio::test(start_paused = true)]
async fn a_handler_cannot_send_nul_characters_in_headers_refused_at_the_call() {
    let f = Fixture::new();
    let memory = virtual_store();
    let system = f.node(memory.clone(), "n1");
    system.create(&t(), &TallyState::default()).await.unwrap();
    let tag = |k: &str, v: &str| TallyMsg::Tag { headers: [(k.to_owned(), v.to_owned())].into() };
    accepted_by(&system, tag("x", "a<NUL>b")).await;
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let note = tally_state(&*memory, &t()).await.note;
    assert!(note.contains("NUL"), "{note}");
    accepted_by(&system, tag("x", "fine")).await;
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(tally_state(&*memory, &t()).await.note, "sent");
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1, "the one accepted Bump");
    assert_eq!(system.run_due_turns(1).await.unwrap(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_cicada_header_is_an_ordinary_header_accepted_and_delivered() {
    let f = Fixture::new();
    let memory = virtual_store();
    let system = f.node(memory.clone(), "n1");
    system.create(&t(), &TallyState::default()).await.unwrap();
    let headers = BTreeMap::from([("cicada-trace".to_owned(), "x".to_owned())]);
    // From outside ...
    let options = SendOptions::new().header("cicada-trace", "x");
    let tag = accepted(system.send_with(&t(), TallyMsg::Tag { headers: headers.clone() }, options).await.unwrap());
    assert_eq!(tag.headers, headers);
    // ... and from a handler.
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(tally_state(&*memory, &t()).await.note, "sent");
    let pending = memory.pending(&addr(&t()), None, 10).await.unwrap().items;
    assert_eq!(pending.iter().map(|m| &m.headers).collect::<Vec<_>>(), vec![&headers]);
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1, "the Bump is delivered");
    assert_eq!(outcome_of(&*memory, &pending[0].id).await.map(|o| o.status), Some(OutcomeStatus::Done));
    assert_eq!(f.runs(), 1);
}

#[tokio::test(start_paused = true)]
async fn sends_and_creates_from_outside_refuse_nul_characters_in_payload_and_state() {
    let f = Fixture::new();
    let system = f.node(virtual_store(), "n1");
    system.create(&t(), &TallyState::default()).await.unwrap();
    let payload = json!({"headers": {"x": "a\u{0}b"}});
    let e = system.send_message(&addr(&t()), "Tag", payload, SendOptions::new()).await.unwrap_err();
    assert!(matches!(e, OpError::Invalid(_)), "{e:?}");
    let e = system.create_actor(&addr(&tally("u")), json!({"note": "a\u{0}b"})).await.unwrap_err();
    assert!(matches!(e, OpError::Invalid(_)), "{e:?}");
}

#[tokio::test]
async fn a_handler_that_blocks_past_its_max_turn_still_ends_failed_timeout_never_done() {
    // Cancellation is cooperative; a blocking handler is waited for, but its outcome stands. Real time: blocking code
    // does not advance tokio's paused clock.
    let f = Fixture::new();
    let memory = Arc::new(InMemoryStore::default());
    let system = ActorSystem::builder(memory.clone()).host(TallyActor::<200>(f.runs.clone())).build();
    system.create(&t(), &TallyState::default()).await.unwrap();
    let m = accepted_by(&system, TallyMsg::Stall { millis: 1_000 }).await;
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    let outcome = outcome_of(&*memory, &m.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert_eq!(outcome.error.as_deref(), Some("timeout"));
    assert_eq!(tally_state(&*memory, &t()).await, TallyState::default());
}

/// A claim that hangs (a database socket that hangs) until the gate opens.
#[derive(Default)]
struct HangingClaim {
    claiming: Notify,
    gate: Notify,
}

#[async_trait]
impl Faults for HangingClaim {
    async fn claim_due(
        &self,
        inner: &dyn Store,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        self.claiming.notify_one();
        self.gate.notified().await;
        inner.claim_due(types, node, ttl, limit).await
    }
}

#[tokio::test]
async fn stop_does_not_hang_on_a_claim_that_never_returns_and_releases_what_that_claim_gets_later() {
    let f = Fixture::new();
    let memory = Arc::new(InMemoryStore::default());
    let hanging = Faulty::new(memory.clone(), HangingClaim::default());
    let system = ActorSystem::builder(hanging.clone())
        .host(f.tally_type())
        .config(NodeConfig { shutdown_cancel_timeout: Duration::from_millis(200), ..NodeConfig::default() })
        .build();
    system.create(&t(), &TallyState::default()).await.unwrap();
    let m = accepted_by(&system, TallyMsg::Bump).await;
    system.start().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), hanging.faults.claiming.notified()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), system.stop_with_grace(Duration::ZERO)).await.unwrap();
    hanging.faults.gate.notify_one(); // the hanging claim returns after all
    // Claimed (attempt 1) by the late claim, then released.
    eventually(Duration::from_secs(5), || async {
        matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(message)) if message.attempt == 1)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    match memory.status(&m.id).await.unwrap() {
        Some(MessageStatus::Pending(message)) => assert_eq!(message.attempt, 1, "released, never run"),
        other => panic!("not pending: {other:?}"),
    }
    assert_eq!(f.runs(), 0);
}
