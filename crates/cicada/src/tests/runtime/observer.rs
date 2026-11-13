//! Turn observers (spec `turn-observer`): when a node tells them that a turn starts and ends, what a dropped value
//! means, that they have no say in a turn, their order, and the headers they add to the sends of a turn. Also the
//! failures of a handler as an observer sees them: timeouts and retries.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::common::*;
use crate::address::ActorAddress;
use crate::observer::{ObservedTurn, Sends, TurnEnd, TurnObserver, TurnStart};
use crate::prelude::*;
use crate::runtime::NodeConfig;
use crate::store::{
    Backend, CommitRequest, CommitResult, Message, MessageStatus, OutcomeStatus, OutgoingMessage, RenewResult,
    StoreError, StoreResult, TurnOutcome,
};
use crate::testing::TestActors;
use crate::{ActorSystem, Failed, Stopped, time};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Probe {
    id: String,
}

impl ActorRef for Probe {
    const TYPE: &'static str = "probe";
    type Message = ProbeMsg;
}

fn p(id: &str) -> Probe {
    Probe { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ProbeMsg {
    /// Records itself, buffers a state change and `sends` self-sends (one when not given), then optionally sleeps
    /// and/or fails.
    Do {
        #[serde(default)]
        fail: bool,
        #[serde(default, rename = "sleepSeconds")]
        sleep_seconds: u64,
        #[serde(default)]
        sends: Option<u32>,
    },
    Log {
        text: String,
    },
    /// Makes the receiver send a failing `Do` to `target`, with a correlation id, a header and the kick retry.
    Kick {
        target: String,
    },
    /// Sends `Log` to itself with `own` headers of its own (`own-00`, `own-01`, …) and `headers`; a refused send fails
    /// the handler.
    Tag {
        #[serde(default)]
        own: u32,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    /// Sends `Log` to itself and to the probe `q`, each now and after ten seconds, with `own` headers of its own
    /// (`own-00`, …) and `headers`; the text of each says which (`now-self`, `later-self`, `now-other`, `later-other`).
    Mixed {
        #[serde(default)]
        own: u32,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    /// Sends `Log` to itself with a zero delay, at the claim, at an hour before it, at a second after the turn's
    /// current time (then sleeps five seconds, so that instant is past at the commit), and now.
    Timers,
    /// Sends `Log` to a probe that does not exist: the commit is rejected.
    Orphan,
    /// Watches `target`.
    Watch {
        target: String,
    },
    Failed(Failed),
    Stopped(Stopped),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct ProbeState {
    #[serde(default)]
    log: Vec<String>,
}

fn kick_retry() -> RetryPolicy {
    RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(5)).unwrap()).unwrap()
}

async fn handle_probe<A>(events: &Events, turn: &mut Turn<A>, message: ProbeMsg) -> HandlerResult
where
    A: Actor<Ref = Probe, State = ProbeState>,
{
    match message {
        ProbeMsg::Do { fail, sleep_seconds, sends } => {
            let m = turn.message();
            events.push(format!(
                "handler from={} corr={} x={}",
                opt(&m.from),
                opt(&m.correlation_id),
                opt(&m.headers.get("x"))
            ));
            turn.state_mut().log.push("do".into());
            for n in 0..sends.unwrap_or(1) {
                turn.send_self(ProbeMsg::Log { text: format!("from handler {n}") })?;
            }
            if sleep_seconds > 0 {
                tokio::time::sleep(Duration::from_secs(sleep_seconds)).await;
            }
            if fail {
                return Err(HandlerError::msg("boom"));
            }
        }
        ProbeMsg::Log { text } => turn.state_mut().log.push(text),
        ProbeMsg::Kick { target } => {
            let options = SendOptions::new().correlation_id("c-1").header("x", "1").retry(kick_retry());
            turn.send_with(&p(&target), ProbeMsg::Do { fail: true, sleep_seconds: 0, sends: None }, options)?;
        }
        ProbeMsg::Tag { own, headers } => {
            let mut options = SendOptions::new();
            for n in 0..own {
                options = options.header(format!("own-{n:02}"), "o");
            }
            for (name, value) in headers {
                options = options.header(name, value);
            }
            turn.send_self_with(ProbeMsg::Log { text: "tagged".into() }, options)?;
        }
        ProbeMsg::Mixed { own, headers } => {
            let mut options = SendOptions::new();
            for n in 0..own {
                options = options.header(format!("own-{n:02}"), "o");
            }
            for (name, value) in headers {
                options = options.header(name, value);
            }
            let later = options.clone().delay(Duration::from_secs(10));
            let log = |text: &str| ProbeMsg::Log { text: text.into() };
            turn.send_self_with(log("now-self"), options.clone())?;
            turn.send_self_with(log("later-self"), later.clone())?;
            turn.send_with(&p("q"), log("now-other"), options)?;
            turn.send_with(&p("q"), log("later-other"), later)?;
        }
        ProbeMsg::Timers => {
            let log = |text: &str| ProbeMsg::Log { text: text.into() };
            let now = turn.now();
            let soon = time::add(turn.current_time(), Duration::from_secs(1)).unwrap();
            turn.send_self_with(log("after-zero"), SendOptions::new().delay(Duration::ZERO))?;
            turn.send_self_with(log("at-now"), SendOptions::new().at(now))?;
            turn.send_self_with(log("at-past"), SendOptions::new().at(time::sub(now, Duration::from_secs(3_600))))?;
            turn.send_self_with(log("at-clamped"), SendOptions::new().at(soon))?;
            turn.send_self(log("now"))?;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        ProbeMsg::Orphan => {
            turn.send(&p("nobody"), ProbeMsg::Log { text: "lost".into() })?;
        }
        ProbeMsg::Watch { target } => turn.watch(&p(&target))?,
        ProbeMsg::Failed(failed) => turn.state_mut().log.push(format!("failed:{}", failed.message_id)),
        ProbeMsg::Stopped(stopped) => turn.state_mut().log.push(format!("stopped:{}", stopped.actor)),
    }
    Ok(())
}

/// The probe; its maximum turn is the node's default.
struct ProbeActor(Events);

impl Actor for ProbeActor {
    type Ref = Probe;
    type State = ProbeState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProbeMsg) -> HandlerResult {
        handle_probe(&self.0, turn, message).await
    }
}

fn do_it() -> ProbeMsg {
    ProbeMsg::Do { fail: false, sleep_seconds: 0, sends: None }
}

/// A probe type with a maximum turn of `max_turn` (the node's default) and `observers`, hosted by a test harness.
async fn probe_test(max_turn: Duration, observers: Vec<Arc<dyn TurnObserver>>) -> TestActors {
    TestActors::builder()
        .with(move |n| {
            let mut n = n
                .host(ProbeActor(Events::default()))
                .config(NodeConfig { default_max_turn: max_turn, ..NodeConfig::default() });
            for observer in observers {
                n = n.observe(observer);
            }
            n
        })
        .build()
        .await
        .unwrap()
}

/// [`probe_test`] with one [`Recorder`] that writes into `events`.
async fn recorded(events: &Events) -> TestActors {
    probe_test(MINUTE, vec![Arc::new(Recorder::new(events))]).await
}

const MINUTE: Duration = Duration::from_secs(60);

type Ends = Arc<Mutex<Vec<(TurnOutcome, Vec<Message>)>>>;

/// What an observer saw: the start and the end of each turn, with every field the test asks about.
#[derive(Clone, Default)]
struct Seen {
    starts: Arc<Mutex<Vec<(ActorAddress, Message, time::Timestamp)>>>,
    ends: Ends,
}

impl TurnObserver for Seen {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        assert_eq!(turn.type_name(), turn.actor().type_name());
        self.starts.lock().unwrap().push((turn.actor().clone(), turn.message().clone(), turn.claimed_at()));
        Box::new(SeenTurn(self.ends.clone()))
    }
}

struct SeenTurn(Ends);

impl ObservedTurn for SeenTurn {
    fn turn_ended(self: Box<Self>, end: &TurnEnd<'_>) {
        self.0.lock().unwrap().push((end.outcome().clone(), end.sent().to_vec()));
    }
}

// ---- when a node tells its observers ----

/// Scenario: A handler returns.
#[tokio::test(start_paused = true)]
async fn a_handler_that_returns_is_told_with_its_start_and_its_end_with_the_messages_it_sent() {
    let seen = Seen::default();
    let test = probe_test(MINUTE, vec![Arc::new(seen.clone())]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 0, sends: Some(2) }).await;
    let before = test.now();
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    let starts = seen.starts.lock().unwrap().clone();
    assert_eq!(starts.len(), 1);
    let (actor, message, claimed_at) = &starts[0];
    assert_eq!((actor, &message.id, message.attempt), (&addr(&p("p")), &m.id, 1));
    assert_eq!(*claimed_at, before, "the store time of the claim");
    let ends = seen.ends.lock().unwrap().clone();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].0, TurnOutcome::Done);
    assert_eq!(ends[0].1.len(), 2, "the two messages");
    assert_eq!(ids_of(&ends[0].1), ids_of(&test.pending(&p("p")).await), "as the store holds them");
}

fn ids_of(messages: &[Message]) -> Vec<MessageId> {
    messages.iter().map(|m| m.id.clone()).collect()
}

/// Scenario: A handler fails and is retried.
#[tokio::test(start_paused = true)]
async fn a_retry_puts_the_same_message_back_in_place_and_the_next_attempt_is_a_new_start() {
    let events = Events::default();
    let test = recorded(&events).await;
    test.create(&p("a"), &ProbeState::default()).await;
    test.create(&p("b"), &ProbeState::default()).await;
    test.send(&p("a"), ProbeMsg::Kick { target: "b".into() }).await; // a failing Do with the kick retry: 2 attempts, 5 s apart
    test.run_until_idle().await;
    let pending = test.pending(&p("b")).await;
    assert_eq!(pending.len(), 1);
    let again = pending[0].clone();
    assert_eq!(again.message_type, "Do");
    assert_eq!(again.from, Some(addr(&p("a"))));
    assert_eq!(again.correlation_id.as_deref(), Some("c-1"));
    assert_eq!(again.headers, [("x".to_owned(), "1".to_owned())].into());
    assert_eq!(again.retry, Some(kick_retry()));
    assert_eq!(again.attempt, 1);
    assert_eq!(again.failures, 1);
    assert_eq!(again.last_error.as_deref(), Some("boom"));
    assert_eq!(time::between(test.now(), again.deliver_at), Duration::from_secs(5));
    assert!(test.outcome(&again).await.is_none(), "no outcome while waiting for the retry");
    assert_eq!(
        test.state::<_, ProbeState>(&p("b")).await,
        ProbeState::default(),
        "the handler's effects are discarded"
    );
    assert_eq!(
        events.all(),
        vec![
            "start:Kick:1",
            "end:Done:sent=1",
            "start:Do:1",
            "end:Rescheduled(delivery=After(delay=5s), error=boom):sent=0"
        ]
    );

    events.clear();
    test.advance_time_by(Duration::from_secs(5)).await;
    let record = test.outcome(&again).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed, "the second failure is the last attempt: exactly one outcome");
    assert_eq!((record.envelope.id, record.envelope.attempt, record.envelope.failures), (again.id, 2, 1));
    assert_eq!(events.all(), vec!["start:Do:2", "end:Failed(error=boom):sent=0"], "a new start with attempt 2");
}

/// Scenario: A handler fails for good.
#[tokio::test(start_paused = true)]
async fn a_handler_that_fails_for_good_is_told_failed_with_the_error() {
    let events = Events::default();
    let test = recorded(&events).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Do { fail: true, sleep_seconds: 0, sends: None }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().error.as_deref(), Some("boom"));
    assert_eq!(events.all(), vec!["start:Do:1", "end:Failed(error=boom):sent=0"]);
}

/// Scenario: A message that cannot be decoded.
#[tokio::test(start_paused = true)]
async fn a_message_that_cannot_be_decoded_is_told_its_start_and_its_end_failed() {
    let events = Events::default();
    let test = recorded(&events).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let bad = enqueue(test.store(), OutgoingMessage::new(addr(&p("p")), "Do", json!({"fail": "not a boolean"}))).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&bad).await.unwrap().status, OutcomeStatus::Failed);
    let all = events.all();
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0], "start:Do:1");
    assert!(all[1].starts_with("end:Failed(error=Payload of 'Do' cannot be decoded"), "{}", all[1]);

    events.clear();
    enqueue(test.store(), OutgoingMessage::new(addr(&p("p")), "Throw", json!({}))).await;
    test.run_until_idle().await;
    assert_eq!(events.all(), vec!["start:Throw:1", "end:Failed(error=Unknown message type 'Throw'):sent=0"]);
}

/// Scenario: State that cannot be decoded.
#[tokio::test(start_paused = true)]
async fn state_that_cannot_be_decoded_is_told_its_start_and_its_end_failed() {
    let events = Events::default();
    let test = recorded(&events).await;
    test.store().create(&addr(&p("p")), json!({"log": "not a list"})).await.unwrap();
    let m = test.send(&p("p"), do_it()).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Failed);
    let all = events.all();
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0], "start:Do:1", "told before anything is decoded");
    assert!(all[1].starts_with("end:Failed(error=State cannot be decoded"), "{}", all[1]);
}

/// Scenario: A turn is cancelled.
#[tokio::test(start_paused = true)]
async fn a_turn_an_operator_cancels_is_told_cancelled() {
    let events = Events::default();
    let test = recorded(&events).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 50, sends: None }).await;
    let system = test.system().clone();
    let runner = tokio::spawn(async move { system.run_due_turns(1).await.unwrap() });
    settle().await;
    assert!(test.system().cancel_turn(&addr(&p("p"))).await.unwrap());
    assert_eq!(runner.await.unwrap(), 1);
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Cancelled);
    assert_eq!(events.all(), vec!["start:Do:1", "end:Cancelled:sent=0"]);
}

/// Scenario: A commit is rejected.
#[tokio::test(start_paused = true)]
async fn a_rejected_commit_is_told_once_as_the_failed_the_node_committed_in_its_place() {
    let seen = Seen::default();
    let test = probe_test(MINUTE, vec![Arc::new(seen.clone())]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Orphan).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&m).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    let ends = seen.ends.lock().unwrap().clone();
    assert_eq!(ends, vec![(TurnOutcome::Failed(outcome.error.unwrap()), vec![])], "once, and nothing sent");
}

/// Scenario: A turn timeout ends `failed(timeout)` by default, and the observer is told so.
#[tokio::test(start_paused = true)]
async fn a_turn_timeout_ends_failed_timeout_by_default() {
    let events = Events::default();
    let test = probe_test(Duration::from_secs(30), vec![Arc::new(Recorder::new(&events))]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 60, sends: None }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().error.as_deref(), Some("timeout"));
    assert_eq!(test.state::<_, ProbeState>(&p("p")).await, ProbeState::default()); // handler effects discarded
    assert!(test.pending(&p("p")).await.is_empty());
    assert_eq!(events.all(), vec!["start:Do:1", "end:Failed(error=timeout):sent=0"]);
}

#[tokio::test(start_paused = true)]
async fn a_timeout_is_retried_under_the_messages_policy_and_records_timeout_as_its_error() {
    let events = Events::default();
    let test = probe_test(Duration::from_secs(30), vec![Arc::new(Recorder::new(&events))]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let retry = RetryPolicy::new(2, Backoff::fixed(Duration::from_millis(1)).unwrap()).unwrap();
    let m = test
        .send_with(
            &p("p"),
            ProbeMsg::Do { fail: false, sleep_seconds: 60, sends: None },
            SendOptions::new().retry(retry),
        )
        .await;
    test.advance_time_by(Duration::from_secs(300)).await;
    let record = test.outcome(&m).await.unwrap();
    assert_eq!(record.error.as_deref(), Some("timeout"));
    assert_eq!(record.envelope.attempt, 2);
    assert_eq!(record.envelope.failures, 1);
    assert_eq!(record.envelope.last_error.as_deref(), Some("timeout"));
    assert_eq!(
        time::between(record.envelope.deliver_at, record.finished_at),
        Duration::from_secs(30),
        "retried 1 ms after the first timeout, timed out again"
    );
    assert!(events.all().iter().any(|e| e.starts_with("end:Rescheduled(")), "{:?}", events.all());
}

// ---- a turn without a commit that its node saw succeed ----

/// The store reports every renewal lost.
struct LoseLease;

#[async_trait]
impl Faults for LoseLease {
    async fn renew(&self, _: &dyn Backend, _: &ActorAddress, _: i64, _: Duration) -> StoreResult<RenewResult> {
        Ok(RenewResult::LeaseLost)
    }
}

fn probe_node(store: impl IntoStore, name: &str, events: &Events) -> ActorSystem {
    node(store, name).host(ProbeActor(Events::default())).observe(Recorder::new(events)).build()
}

/// Scenario: A lease is lost.
#[tokio::test(start_paused = true)]
async fn a_turn_that_loses_its_lease_is_dropped_and_the_next_claim_is_a_new_start() {
    let memory = virtual_store();
    let events = Events::default();
    let losing = probe_node(Faulty::new(memory.clone(), LoseLease), "losing", &events);
    let healthy = probe_node(memory.clone(), "healthy", &events);
    losing.create(&p("p"), &ProbeState::default()).await.unwrap();
    let m = accepted(losing.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 20, sends: None }).await.unwrap());
    assert_eq!(losing.run_due_turns(1).await.unwrap(), 1); // cancelled at the first heartbeat, nothing committed
    assert_eq!(events.all(), vec!["start:Do:1", "dropped:Do"]);
    sleep(Duration::from_secs(31)).await; // the lease runs out
    assert_eq!(healthy.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(events.all(), vec!["start:Do:1", "dropped:Do", "start:Do:2", "end:Done:sent=1"]);
    assert_eq!(outcome_of(&memory, &m.id).await.unwrap().status, OutcomeStatus::Done);
}

/// Scenario: A node stops in the middle of a turn.
#[tokio::test(start_paused = true)]
async fn a_turn_whose_node_stops_and_gives_the_lease_back_is_dropped() {
    let memory = virtual_store();
    let events = Events::default();
    let config = NodeConfig { shutdown_grace_period: Duration::from_secs(1), ..node_config("n1") };
    let system = node(memory.clone(), "n1")
        .host(ProbeActor(Events::default()))
        .observe(Recorder::new(&events))
        .config(config)
        .build();
    system.create(&p("p"), &ProbeState::default()).await.unwrap();
    let m = accepted(system.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 50, sends: None }).await.unwrap());
    system.start().await.unwrap();
    settle().await;
    assert_eq!(events.all(), vec!["start:Do:1"]);
    system.stop().await;
    assert_eq!(events.all(), vec!["start:Do:1", "dropped:Do"]);
    assert!(matches!(memory.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(_))), "given back");
}

/// Scenario: The actor is deleted while its turn runs.
#[tokio::test(start_paused = true)]
async fn a_turn_whose_actor_is_deleted_while_it_runs_is_dropped() {
    let events = Events::default();
    let test = recorded(&events).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 5, sends: None }).await;
    let system = test.system().clone();
    let runner = tokio::spawn(async move { system.run_due_turns(1).await.unwrap() });
    settle().await;
    test.system().delete_actor(&addr(&p("p"))).await.unwrap();
    assert_eq!(runner.await.unwrap(), 1);
    assert_eq!(events.all(), vec!["start:Do:1", "dropped:Do"]);
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Purged);
}

/// The first commit applies, and its answer is lost.
struct AnswerLost(AtomicBool);

#[async_trait]
impl Faults for AnswerLost {
    async fn commit(&self, inner: &dyn Backend, commit: CommitRequest) -> StoreResult<CommitResult> {
        let result = inner.commit(commit).await?;
        if !self.0.swap(true, Ordering::SeqCst) {
            return Err(StoreError::unavailable("connection reset after COMMIT"));
        }
        Ok(result)
    }
}

/// Scenario: A commit whose answer is lost.
#[tokio::test(start_paused = true)]
async fn a_turn_whose_commit_answer_is_lost_is_dropped_although_its_message_has_its_outcome() {
    let memory = virtual_store();
    let events = Events::default();
    let system = probe_node(Faulty::new(memory.clone(), AnswerLost(AtomicBool::new(false))), "n1", &events);
    system.create(&p("p"), &ProbeState::default()).await.unwrap();
    let m = accepted(system.send(&p("p"), do_it()).await.unwrap());
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(outcome_of(&memory, &m.id).await.unwrap().status, OutcomeStatus::Done, "the commit applied");
    assert_eq!(events.all(), vec!["start:Do:1", "dropped:Do"], "the node saw no commit succeed");
}

// ---- no say in a turn ----

/// Adds the header `trace` with its value and then panics where `panics_at` says: at the `start`, at the `end`, or
/// when its value is dropped without an end (`drop`).
struct Breaking {
    value: &'static str,
    panics_at: &'static str,
}

impl TurnObserver for Breaking {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        turn.add_send_header("trace", self.value);
        if self.panics_at == "start" {
            panic!("an observer broke at the start");
        }
        Box::new(BreakingTurn { panics_at: self.panics_at, ended: false })
    }
}

struct BreakingTurn {
    panics_at: &'static str,
    ended: bool,
}

impl ObservedTurn for BreakingTurn {
    fn turn_ended(mut self: Box<Self>, _end: &TurnEnd<'_>) {
        self.ended = true;
        if self.panics_at == "end" {
            panic!("an observer broke at the end");
        }
    }
}

impl Drop for BreakingTurn {
    fn drop(&mut self) {
        if self.panics_at == "drop" && !self.ended {
            panic!("an observer broke when its turn was dropped");
        }
    }
}

/// Scenario: An observer panics at the start.
#[tokio::test(start_paused = true)]
async fn an_observer_that_panics_at_the_start_is_left_out_and_its_headers_are_not_used() {
    let events = Events::default();
    let first = Arc::new(Breaking { value: "first", panics_at: "start" });
    let test = probe_test(MINUTE, vec![first, Arc::new(Recorder::new(&events))]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), do_it()).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done, "the handler ran");
    let sent = test.pending(&p("p")).await;
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].headers.contains_key("trace"), "no header of the first observer");
    assert_eq!(events.all(), vec!["start:Do:1", "end:Done:sent=1"], "the second is told the start and the end");
}

/// Scenario: An observer panics at the end.
#[tokio::test(start_paused = true)]
async fn an_observer_that_panics_at_the_end_changes_nothing_and_the_next_is_told() {
    let events = Events::default();
    let first = Arc::new(Breaking { value: "first", panics_at: "end" });
    let test = probe_test(MINUTE, vec![first, Arc::new(Recorder::new(&events))]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), do_it()).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done, "the outcome stands");
    assert_eq!(test.pending(&p("p")).await[0].headers.get("trace").map(String::as_str), Some("first"));
    assert_eq!(events.all(), vec!["start:Do:1", "end:Done:sent=1"]);
}

/// Scenario: An observer panics when its value is dropped.
#[tokio::test(start_paused = true)]
async fn an_observer_that_panics_when_its_value_is_dropped_leaves_the_node_going() {
    /// The store reports every renewal lost while the flag is set.
    struct LoseLeaseWhile(AtomicBool);

    #[async_trait]
    impl Faults for LoseLeaseWhile {
        async fn renew(
            &self,
            inner: &dyn Backend,
            a: &ActorAddress,
            e: i64,
            ttl: Duration,
        ) -> StoreResult<RenewResult> {
            if self.0.load(Ordering::SeqCst) { Ok(RenewResult::LeaseLost) } else { inner.renew(a, e, ttl).await }
        }
    }

    let memory = virtual_store();
    let events = Events::default();
    let faulty = Faulty::new(memory.clone(), LoseLeaseWhile(AtomicBool::new(true)));
    let system = node(faulty.clone(), "n1")
        .host(ProbeActor(Events::default()))
        .observe(Breaking { value: "first", panics_at: "drop" })
        .observe(Recorder::new(&events))
        .build();
    system.create(&p("p"), &ProbeState::default()).await.unwrap();
    let m = accepted(system.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 20, sends: None }).await.unwrap());
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1);
    assert_eq!(events.all(), vec!["start:Do:1", "dropped:Do"], "the value of the other observer is dropped");
    faulty.faults.0.store(false, Ordering::SeqCst);
    sleep(Duration::from_secs(31)).await; // the lease runs out
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1, "the same node runs the next claim of the message");
    assert_eq!(events.all(), vec!["start:Do:1", "dropped:Do", "start:Do:2", "end:Done:sent=1"]);
    assert_eq!(outcome_of(&memory, &m.id).await.unwrap().status, OutcomeStatus::Done);
}

// ---- order ----

/// Scenario: Two observers.
#[tokio::test(start_paused = true)]
async fn two_observers_are_called_in_the_order_they_were_added() {
    let events = Events::default();
    let a = Arc::new(Recorder::labelled("A", &events));
    let b = Arc::new(Recorder::labelled("B", &events));
    let test = probe_test(MINUTE, vec![a, b]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    test.send(&p("p"), do_it()).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(events.all(), vec!["A:start:Do:1", "B:start:Do:1", "A:end:Done:sent=1", "B:end:Done:sent=1"]);
}

// ---- headers ----

/// Adds every header of its list, and is told nothing.
struct Headers(Vec<(String, String)>);

impl Headers {
    fn of(pairs: &[(&str, &str)]) -> Arc<dyn TurnObserver> {
        Arc::new(Self(pairs.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect()))
    }
}

impl TurnObserver for Headers {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        for (name, value) in &self.0 {
            turn.add_send_header(name.clone(), value.clone());
        }
        Box::new(())
    }
}

/// The headers of the one message the turn of `message` sent to `p`.
async fn sent_headers(observers: Vec<Arc<dyn TurnObserver>>, message: ProbeMsg) -> Vec<BTreeMap<String, String>> {
    let test = probe_test(MINUTE, observers).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), message).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done, "the sends succeeded");
    test.pending(&p("p")).await.into_iter().map(|m| m.headers).collect()
}

fn tag(own: u32, headers: &[(&str, &str)]) -> ProbeMsg {
    ProbeMsg::Tag { own, headers: headers.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect() }
}

/// Scenario: Headers reach every send.
#[tokio::test(start_paused = true)]
async fn the_headers_of_an_observer_reach_every_send() {
    let all = sent_headers(
        vec![Headers::of(&[("trace", "t1")])],
        ProbeMsg::Do { fail: false, sleep_seconds: 0, sends: Some(2) },
    )
    .await;
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|h| h.get("trace").map(String::as_str) == Some("t1")), "{all:?}");
}

/// Scenario: A send's own header wins.
#[tokio::test(start_paused = true)]
async fn a_header_a_send_sets_itself_keeps_the_sends_value() {
    let all = sent_headers(vec![Headers::of(&[("trace", "observer")])], tag(0, &[("trace", "send")])).await;
    assert_eq!(all, vec![BTreeMap::from([("trace".to_owned(), "send".to_owned())])]);
}

/// Scenario: Two observers add the same header.
#[tokio::test(start_paused = true)]
async fn of_two_observers_that_add_one_header_the_one_added_later_wins() {
    let observers = vec![Headers::of(&[("trace", "A")]), Headers::of(&[("trace", "B")])];
    let all = sent_headers(observers, tag(0, &[])).await;
    assert_eq!(all, vec![BTreeMap::from([("trace".to_owned(), "B".to_owned())])]);
}

/// Scenario: A header that is too long.
#[tokio::test(start_paused = true)]
async fn a_header_value_that_is_too_long_is_left_out_and_the_send_succeeds() {
    let long = "v".repeat(5_000);
    let all = sent_headers(vec![Headers::of(&[("trace", long.as_str()), ("ok", "1")])], tag(0, &[])).await;
    assert_eq!(all, vec![BTreeMap::from([("ok".to_owned(), "1".to_owned())])]);
}

/// Scenario: A header that a message refuses.
#[tokio::test(start_paused = true)]
async fn a_header_with_a_nul_character_or_a_long_name_is_left_out_and_the_send_succeeds() {
    let long_name = "n".repeat(300);
    let refused = Headers::of(&[("trace", "a\u{0}b"), (long_name.as_str(), "v"), ("ok", "1")]);
    let all = sent_headers(vec![refused], tag(0, &[])).await;
    assert_eq!(all, vec![BTreeMap::from([("ok".to_owned(), "1".to_owned())])]);
}

/// Scenario: A message that is full.
#[tokio::test(start_paused = true)]
async fn a_message_that_is_full_takes_its_own_headers_and_then_the_observers_in_the_order_of_their_names() {
    let all = sent_headers(vec![Headers::of(&[("zz-second", "2"), ("aa-first", "1")])], tag(63, &[])).await;
    assert_eq!(all.len(), 1);
    let headers = &all[0];
    assert_eq!(headers.len(), 64);
    assert_eq!(headers.get("aa-first").map(String::as_str), Some("1"), "the name that sorts first");
    assert!(!headers.contains_key("zz-second"), "does not fit");
    assert_eq!(headers.keys().filter(|k| k.starts_with("own-")).count(), 63, "its own");
}

/// Scenario: A notification has no observer headers.
#[tokio::test(start_paused = true)]
async fn a_notification_of_a_watch_carries_no_header_of_an_observer() {
    let test = probe_test(MINUTE, vec![Headers::of(&[("trace", "t1")])]).await;
    test.create(&p("watcher"), &ProbeState::default()).await;
    test.create(&p("watched"), &ProbeState::default()).await;
    test.send(&p("watcher"), ProbeMsg::Watch { target: "watched".into() }).await;
    test.run_until_idle().await;
    let failing = test.send(&p("watched"), ProbeMsg::Do { fail: true, sleep_seconds: 0, sends: None }).await;
    assert_eq!(test.system().run_due_turns(10).await.unwrap(), 1);
    assert_eq!(test.outcome(&failing).await.unwrap().status, OutcomeStatus::Failed);
    let mailbox = test.pending(&p("watcher")).await;
    assert_eq!(mailbox.len(), 1, "{mailbox:?}");
    assert_eq!(mailbox[0].message_type, "Failed");
    assert!(mailbox[0].headers.is_empty(), "{:?}", mailbox[0].headers);
}

// ---- headers for timers and the other sends ----

/// Adds every header of its list for the sends it names, then panics when `panics`; is told nothing.
struct Picks {
    headers: Vec<(Sends, String, String)>,
    panics: bool,
}

impl Picks {
    fn of(headers: &[(Sends, &str, &str)]) -> Arc<dyn TurnObserver> {
        let headers = headers.iter().map(|(s, n, v)| (*s, (*n).to_owned(), (*v).to_owned())).collect();
        Arc::new(Self { headers, panics: false })
    }
}

impl TurnObserver for Picks {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        for (sends, name, value) in &self.headers {
            turn.add_header_for(*sends, name.clone(), value.clone());
        }
        if self.panics {
            panic!("an observer broke after it picked its headers");
        }
        Box::new(())
    }
}

/// The headers of each message the turn of `message` sent to `p` or `q`, by the text of the message.
async fn sent_by_text(
    observers: Vec<Arc<dyn TurnObserver>>,
    message: ProbeMsg,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let test = probe_test(MINUTE, observers).await;
    test.create(&p("p"), &ProbeState::default()).await;
    test.create(&p("q"), &ProbeState::default()).await;
    let m = test.send(&p("p"), message).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done, "the sends succeeded");
    let mut sent = test.pending(&p("p")).await;
    sent.extend(test.pending(&p("q")).await);
    sent.into_iter().map(|m| (m.payload["text"].as_str().unwrap().to_owned(), m.headers)).collect()
}

fn mixed(own: u32, headers: &[(&str, &str)]) -> ProbeMsg {
    ProbeMsg::Mixed { own, headers: headers.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect() }
}

fn value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers.get(name).map(String::as_str)
}

/// Scenario: A header selection distinguishes requested timer sends.
#[tokio::test(start_paused = true)]
async fn only_a_delayed_send_to_the_actor_itself_gets_the_headers_for_timers() {
    let picks = Picks::of(&[(Sends::All, "audit", "a"), (Sends::Timers, "timer", "t"), (Sends::Others, "work", "w")]);
    let sent = sent_by_text(vec![picks, Headers::of(&[("legacy", "l")])], mixed(0, &[])).await;
    assert_eq!(sent.len(), 4, "{sent:?}");
    for (text, headers) in &sent {
        assert_eq!(value(headers, "audit"), Some("a"), "{text}");
        assert_eq!(value(headers, "legacy"), Some("l"), "add_send_header reaches every send: {text}");
        let timer = text == "later-self";
        assert_eq!(value(headers, "timer"), timer.then_some("t"), "{text}");
        assert_eq!(value(headers, "work"), (!timer).then_some("w"), "{text}");
    }
}

/// Scenario: Zero delays and past times are still timer requests.
#[tokio::test(start_paused = true)]
async fn a_zero_delay_and_a_past_instant_are_timers_and_a_send_now_to_itself_is_not() {
    let picks = Picks::of(&[(Sends::Timers, "timer", "t"), (Sends::Others, "work", "w")]);
    let sent = sent_by_text(vec![picks], ProbeMsg::Timers).await;
    assert_eq!(sent.len(), 5, "{sent:?}");
    for (text, headers) in &sent {
        let timer = text != "now";
        assert_eq!(value(headers, "timer"), timer.then_some("t"), "{text}");
        assert_eq!(value(headers, "work"), (!timer).then_some("w"), "{text}");
    }
}

/// Scenario: Calls and observers override independently per kind.
#[tokio::test(start_paused = true)]
async fn calls_and_observers_override_a_header_in_each_kind_on_its_own() {
    let a = Picks::of(&[(Sends::All, "x", "all"), (Sends::Timers, "x", "timer")]);
    let b = Picks::of(&[(Sends::Others, "x", "other")]);
    let sent = sent_by_text(vec![a, b], mixed(0, &[])).await;
    assert_eq!(sent.len(), 4);
    for (text, headers) in &sent {
        let expected = if text == "later-self" { "timer" } else { "other" };
        assert_eq!(value(headers, "x"), Some(expected), "{text}");
    }
    let last = Picks::of(&[(Sends::Timers, "x", "timer"), (Sends::Others, "x", "other"), (Sends::All, "x", "last")]);
    let sent = sent_by_text(vec![last], mixed(0, &[])).await;
    assert!(sent.values().all(|h| value(h, "x") == Some("last")), "{sent:?}");
}

/// Scenario: The send's own headers and limits apply independently to each kind.
#[tokio::test(start_paused = true)]
async fn each_kind_keeps_a_sends_own_header_and_fills_up_to_the_limit_in_name_order() {
    let picks = Picks::of(&[
        (Sends::All, "x", "observer"),
        (Sends::Timers, "a-timer", "1"),
        (Sends::Timers, "b-timer", "2"),
        (Sends::Others, "a-other", "1"),
        (Sends::Others, "b-other", "2"),
    ]);
    // 62 own headers and `x` leave room for one header of the observer.
    let sent = sent_by_text(vec![picks], mixed(62, &[("x", "mine")])).await;
    assert_eq!(sent.len(), 4);
    for (text, headers) in &sent {
        assert_eq!(headers.len(), 64, "{text}");
        assert_eq!(value(headers, "x"), Some("mine"), "{text}");
        let (first, second, unselected) =
            if text == "later-self" { ("a-timer", "b-timer", "a-other") } else { ("a-other", "b-other", "a-timer") };
        assert_eq!(value(headers, first), Some("1"), "{text}");
        assert!(!headers.contains_key(second), "{text}: does not fit");
        assert!(!headers.contains_key(unselected), "{text}: not selected");
    }
}

/// Scenario: Invalid selected headers do not erase previous values.
#[tokio::test(start_paused = true)]
async fn a_refused_header_keeps_the_earlier_value_of_its_name() {
    let long = "v".repeat(5_000);
    let picks = Picks::of(&[
        (Sends::Timers, "t", "ok"),
        (Sends::Timers, "t", "a\u{0}b"),
        (Sends::Others, "o", "ok"),
        (Sends::Others, "o", long.as_str()),
        (Sends::All, "both", "fine"),
        (Sends::All, "both", "a\u{0}b"),
    ]);
    let sent = sent_by_text(vec![picks], mixed(0, &[])).await;
    assert_eq!(sent.len(), 4);
    for (text, headers) in &sent {
        let (name, other) = if text == "later-self" { ("t", "o") } else { ("o", "t") };
        assert_eq!(value(headers, name), Some("ok"), "{text}");
        assert!(!headers.contains_key(other), "{text}");
        assert_eq!(value(headers, "both"), Some("fine"), "{text}");
    }
}

/// Scenario: A panicking observer loses both selections.
#[tokio::test(start_paused = true)]
async fn an_observer_that_panics_loses_its_headers_of_both_kinds() {
    let broken = Arc::new(Picks {
        headers: vec![(Sends::Timers, "timer".into(), "t".into()), (Sends::Others, "work".into(), "w".into())],
        panics: true,
    });
    let kept = Picks::of(&[(Sends::Others, "kept", "k"), (Sends::Timers, "kept-timer", "k")]);
    let sent = sent_by_text(vec![broken, kept], mixed(0, &[])).await;
    assert_eq!(sent.len(), 4, "the handler ran: {sent:?}");
    for (text, headers) in &sent {
        assert!(!headers.contains_key("timer") && !headers.contains_key("work"), "{text}: {headers:?}");
        let name = if text == "later-self" { "kept-timer" } else { "kept" };
        assert_eq!(value(headers, name), Some("k"), "{text}: the other observer still adds its headers");
    }
}
