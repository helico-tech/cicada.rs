//! The same actors run locally and through a worker over the reference source (spec `remote-workers`, ADR 0039), and
//! the faults of a worker's completion, on virtual time. Parity: state, effects, outcomes and observer ends for typed
//! and raw sends and creates, watches, controls, a stop, retry, handler errors and panics, undecodable messages,
//! MAX_TURN, states that cannot be stored, envelope fields and observer headers. Faults: a lost acknowledgement (also
//! with a retention-hidden outcome), a rejection and the failure that replaces it for good, a refused retry of a
//! failure, malformed results, hung completions, the fixed budget, and operator cancellation through renewal, also
//! followed by a shutdown. No path runs a handler twice or releases a lease after its handler ended.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use super::common::*;
use super::remote_worker::{Complete, Scripted, TTL, secs, send, server, setup, total, worker};
use crate::address::ActorAddress;
use crate::observer::{ObservedTurn, TurnObserver, TurnStart};
use crate::prelude::*;
use crate::retention::Retention;
use crate::retry::{Backoff, RetryPolicy};
use crate::runtime::{CompleteTurnResult, NodeConfig, TurnFailure, TurnReport, Worker, WorkerConfig};
use crate::store::{MessageStatus, NodeId, OutcomeStatus};
use crate::{ActorSystem, Failed, FailureCause, Stopped};

/// type `probe`, key `{id}`: the effects the counter does not have.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Probe {
    id: String,
}

impl ActorRef for Probe {
    const TYPE: &'static str = "probe";
    type Message = ProbeMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ProbeMsg {
    Panic,
    WatchIt {
        target: String,
    },
    PauseIt {
        target: String,
    },
    /// Sends `Inspect` to the counter `target`: it logs the headers the observer added.
    Hop {
        target: String,
    },
    /// Fails, retryable.
    Flaky,
    /// Creates `counter/{id}` by its raw address.
    CreateRaw {
        id: String,
    },
    /// Sets a state nested too deep to be stored.
    Deepen,
    Failed(Failed),
    Stopped(Stopped),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct ProbeState {
    seen: Vec<String>,
    deep: Option<Value>,
}

struct ProbeActor;

impl Actor for ProbeActor {
    type Ref = Probe;
    type State = ProbeState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProbeMsg) -> HandlerResult {
        match message {
            ProbeMsg::Panic => panic!("the probe panicked"),
            ProbeMsg::WatchIt { target } => turn.watch(&Counter::new(&target))?,
            ProbeMsg::PauseIt { target } => turn.pause(&Counter::new(&target))?,
            ProbeMsg::Hop { target } => {
                turn.send(&Counter::new(&target), CounterMsg::Inspect)?;
            }
            ProbeMsg::Flaky => return Err(HandlerError::msg("flaky")),
            ProbeMsg::CreateRaw { id } => {
                let address = ActorAddress::parse(&format!("counter/{id}"))?;
                turn.create_raw(&address, json!({"total": 7})).await?;
            }
            ProbeMsg::Deepen => {
                turn.state_mut().deep = Some((0..70).fold(json!(1), |inner, _| json!([inner])));
            }
            ProbeMsg::Failed(failed) => turn.state_mut().seen.push(format!("failed:{}", failed.actor)),
            ProbeMsg::Stopped(stopped) => turn.state_mut().seen.push(format!("stopped:{}", stopped.actor)),
        }
        Ok(())
    }
}

counter_type!(QuickCounter, Duration::from_secs(1));

/// Adds the header `via` with the type of the message whose turn sends.
struct Via;

impl TurnObserver for Via {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        let via = turn.message().message_type.clone();
        turn.add_send_header("via", via);
        Box::new(())
    }
}

/// Where the turns run.
enum Runner {
    Local(ActorSystem),
    Worker(Worker),
}

impl Runner {
    async fn run_one(&self) -> usize {
        match self {
            Runner::Local(node) => node.run_due_turns(1).await.unwrap(),
            Runner::Worker(worker) => worker.run_due_turns(1).await.unwrap(),
        }
    }
}

/// A store, the system that serves sends and reads (the node itself, or the worker's server), the runner and the
/// observer's events.
struct Rig {
    system: ActorSystem,
    /// A system on the same store that hosts nothing: it sends without decoding.
    sender: ActorSystem,
    runner: Runner,
    events: Events,
}

impl Rig {
    async fn new(remote: bool, quick: bool) -> Rig {
        let store = virtual_store();
        let events = Events::default();
        let sender = server(&store, "sender", TTL);
        if remote {
            let system = server(&store, "server", TTL);
            let mut builder = Worker::builder(Scripted::new(&system))
                .observe(Recorder::new(&events))
                .observe(Via)
                .config(WorkerConfig { worker_id: NodeId::new("worker").unwrap(), ..WorkerConfig::default() });
            builder = if quick { builder.host(QuickCounter) } else { builder.host(CounterActor).host(ProbeActor) };
            let worker = builder.build();
            worker.initialize().await.unwrap();
            Rig { system, sender, runner: Runner::Worker(worker), events }
        } else {
            let mut builder = ActorSystem::builder(store.clone())
                .observe(Recorder::new(&events))
                .observe(Via)
                .config(NodeConfig { node_id: NodeId::new("node").unwrap(), lease_ttl: TTL, ..NodeConfig::default() });
            builder = if quick { builder.host(QuickCounter) } else { builder.host(CounterActor).host(ProbeActor) };
            let node = builder.build();
            node.initialize().await.unwrap();
            Rig { system: node.clone(), sender, runner: Runner::Local(node), events }
        }
    }

    /// Runs one turn at a time until none is due.
    async fn run_until_idle(&self) {
        while self.runner.run_one().await > 0 {}
    }

    /// Every actor with its state and version, and every pending message type, by address.
    async fn snapshot(&self) -> Vec<(String, Value, i64, Vec<String>)> {
        let page = self.system.list_actors_with_state("", None, 100).await.unwrap();
        let mut all = Vec::new();
        for actor in page.items {
            let pending = self.system.pending(&actor.address, None, 100).await.unwrap();
            let types = pending.map(|p| p.items.into_iter().map(|m| m.message_type).collect()).unwrap_or_default();
            all.push((actor.address.to_string(), actor.state, actor.version, types));
        }
        all
    }
}

/// The outcome of message `id`: its status and error.
async fn outcome(system: &ActorSystem, id: &crate::store::MessageId) -> Option<(OutcomeStatus, Option<String>)> {
    system.outcome(id).await.unwrap().map(|o| (o.status, o.error))
}

/// One run of the parity scenario: what it ended with.
async fn scenario(remote: bool) -> (Vec<String>, Vec<(String, Value, i64, Vec<String>)>, Vec<String>) {
    let rig = Rig::new(remote, false).await;
    let system = &rig.system;
    for id in ["a", "b", "d"] {
        system.create(&Counter::new(id), &CounterState::default()).await.unwrap();
    }
    system.create(&Probe { id: "p".into() }, &ProbeState::default()).await.unwrap();
    let a = Counter::new("a");
    let p = Probe { id: "p".into() };
    let mut ids = Vec::new();
    let mut to_a = |m| ids.push(m);
    to_a(send(system, "a", CounterMsg::Add { n: 2 }).await.id);
    to_a(send(system, "a", CounterMsg::Boom { reason: "no".into() }).await.id);
    to_a(send(system, "a", CounterMsg::Forward { to: "counter/b".into(), n: 3 }).await.id);
    to_a(send(system, "a", CounterMsg::SendUnknown { to: "counter/b".into() }).await.id);
    to_a(send(system, "a", CounterMsg::Spawn { child: "c".into(), n: 4 }).await.id);
    to_a(send(system, "a", CounterMsg::Later { n: 5, delay_millis: 1_000 }).await.id);
    to_a(send(system, "a", CounterMsg::Orphan { target: "zz".into() }).await.id);
    to_a(send(system, "a", CounterMsg::InnerTimeout).await.id);
    let options = SendOptions::new().correlation_id("corr-1").header("h", "v");
    to_a(accepted(system.send_with(&a, CounterMsg::Inspect, options).await.unwrap()).id);
    let raw = rig.sender.send_message(&a.address(), "Add", json!({"n": "x"}), SendOptions::new()).await.unwrap();
    to_a(accepted(raw).id);
    let probe = |m: ProbeMsg| {
        let system = system.clone();
        let p = p.clone();
        async move { accepted(system.send(&p, m).await.unwrap()).id }
    };
    let mut more = vec![
        probe(ProbeMsg::Panic).await,
        probe(ProbeMsg::WatchIt { target: "d".into() }).await,
        probe(ProbeMsg::PauseIt { target: "b".into() }).await,
        probe(ProbeMsg::Hop { target: "a".into() }).await,
        probe(ProbeMsg::CreateRaw { id: "r".into() }).await,
        probe(ProbeMsg::Deepen).await,
    ];
    let policy = RetryPolicy::new(2, Backoff::fixed(secs(1)).unwrap()).unwrap();
    more.push(accepted(system.send_with(&p, ProbeMsg::Flaky, SendOptions::new().retry(policy)).await.unwrap()).id);
    ids.extend(more);
    rig.run_until_idle().await;
    let halt = send(system, "d", CounterMsg::Halt).await.id;
    ids.push(halt);
    rig.run_until_idle().await;
    sleep(secs(2)).await;
    rig.run_until_idle().await;
    let mut outcomes = Vec::new();
    for id in &ids {
        outcomes.push(format!("{:?}", outcome(system, id).await));
    }
    (rig.events.all(), rig.snapshot().await, outcomes)
}

/// Scenario: Local and Worker parity.
#[tokio::test(start_paused = true)]
async fn a_worker_ends_every_turn_as_a_local_node_does() {
    let (local_events, local_state, local_outcomes) = scenario(false).await;
    let (remote_events, remote_state, remote_outcomes) = scenario(true).await;
    assert_eq!(remote_outcomes, local_outcomes);
    assert_eq!(remote_state, local_state);
    assert_eq!(remote_events, local_events);
    // The scenario covers what it claims.
    let joined = local_outcomes.join("\n");
    for expected in [
        "Done",
        "Some(\"no\")",
        "Send to absent instance counter/zz",
        "the probe panicked",
        "cannot be decoded",
        "State cannot be stored",
        "Some(\"flaky\")",
    ] {
        assert!(joined.contains(expected), "no outcome with {expected}:\n{joined}");
    }
    let state = format!("{local_state:?}");
    for expected in ["rejected:Nope", "corr=corr-1", r#"\"via\": \"Hop\""#, "stopped:counter/d", "counter/r"] {
        assert!(state.contains(expected), "no {expected} in {state}");
    }
    assert!(local_events.iter().any(|e| e.starts_with("end:Rescheduled")), "{local_events:?}");
}

/// Scenario: Local and Worker parity.
#[tokio::test(start_paused = true)]
async fn max_turn_ends_a_worker_turn_as_it_ends_a_local_one() {
    let mut ends = Vec::new();
    for remote in [false, true] {
        let rig = Rig::new(remote, true).await;
        rig.system.create(&Counter::new("q"), &CounterState::default()).await.unwrap();
        let m = send(&rig.system, "q", CounterMsg::Sleep { millis: 5_000 }).await;
        rig.run_until_idle().await;
        ends.push((outcome(&rig.system, &m.id).await, rig.events.all()));
    }
    assert_eq!(ends[0], ends[1]);
    assert_eq!(ends[0].0, Some((OutcomeStatus::Failed, Some("timeout".to_owned()))));
}

// ---- completion faults ----

struct Fixture {
    server: ActorSystem,
    source: Arc<Scripted>,
    worker: Worker,
    events: Events,
}

async fn fixture() -> Fixture {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    setup(&server, &["a", "b"]).await;
    let source = Scripted::new(&server);
    let events = Events::default();
    let worker = worker(&source, &events);
    Fixture { server, source, worker, events }
}

fn log(state: &Value) -> Vec<String> {
    serde_json::from_value::<CounterState>(state.clone()).unwrap().log
}

async fn counter_log(server: &ActorSystem, id: &str) -> Vec<String> {
    log(&server.get_actor(&Counter::new(id).address()).await.unwrap().unwrap().state)
}

fn failure_of(report: &TurnReport) -> &TurnFailure {
    match report {
        TurnReport::Failed(failure) => failure,
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// Scenario: An acknowledgement is lost.
#[tokio::test(start_paused = true)]
async fn a_lost_acknowledgement_is_retried_with_the_same_report_and_never_runs_the_handler_again() {
    for hidden in [false, true] {
        let f = fixture().await;
        let options = if hidden { SendOptions::new().retention(Retention::REMOVE_ON_DONE) } else { SendOptions::new() };
        let m = accepted(f.server.send_with(&Counter::new("a"), CounterMsg::Add { n: 1 }, options).await.unwrap());
        f.source.script([Complete::LoseAfter]);
        assert_eq!(f.worker.run_due_turns(10).await.unwrap(), 1);
        assert_eq!(f.source.reports().len(), 2, "the same report again");
        assert_eq!(f.source.reports()[0], f.source.reports()[1]);
        assert_eq!(Scripted::count(&f.source.applied), 1);
        assert_eq!(counter_log(&f.server, "a").await, ["add:1"], "applied once, handler ran once");
        assert_eq!(f.events.all(), ["start:Add:1", "dropped:Add"], "uncertain: no end");
        assert_eq!(Scripted::count(&f.source.releases), 0);
        let status = f.server.status(&m.id).await.unwrap();
        assert!(!matches!(status, Some(MessageStatus::Pending(_) | MessageStatus::Running { .. })), "{status:?}");
    }
}

/// Scenario: The actual completed outcome.
#[tokio::test(start_paused = true)]
async fn the_observer_ends_with_the_failure_the_server_derived_from_a_returned_report() {
    let store = virtual_store();
    let server = server(&store, "server", TTL);
    server.register_contract(&crate::actor::contract_of::<ProbeActor>().unwrap()).await.unwrap();
    server.create(&Probe { id: "p".into() }, &ProbeState::default()).await.unwrap();
    let m = accepted(server.send(&Probe { id: "p".into() }, ProbeMsg::Deepen).await.unwrap());
    let source = Scripted::new(&server);
    let events = Events::default();
    let worker = Worker::builder(source.clone()).host(ProbeActor).observe(Recorder::new(&events)).build();
    worker.run_due_turns(10).await.unwrap();
    assert!(matches!(source.reports()[0], TurnReport::Returned { .. }));
    let (status, error) = outcome(&server, &m.id).await.unwrap();
    assert_eq!(status, OutcomeStatus::Failed);
    assert!(error.as_deref().unwrap().starts_with("State cannot be stored"), "{error:?}");
    assert_eq!(events.len(), 2);
    assert!(events.last().starts_with("end:Failed(error=State cannot be stored"), "{:?}", events.all());
}

/// Scenario: Returned report rejected.
#[tokio::test(start_paused = true)]
async fn a_rejected_report_is_replaced_for_good_and_an_uncertain_failure_commit_never_restores_its_effects() {
    let f = fixture().await;
    let m = send(&f.server, "a", CounterMsg::Orphan { target: "zz".into() }).await;
    // The real rejection (no `zz`), then a failure commit whose answer is lost, then one more try.
    f.source.script([Complete::Pass, Complete::LoseAfter]);
    let batch = tokio::spawn({
        let worker = f.worker.clone();
        async move { worker.run_due_turns(10).await }
    });
    sleep(Duration::from_millis(10)).await;
    // The target exists now: the original effects would be valid, and are never sent again.
    f.server.create(&Counter::new("zz"), &CounterState::default()).await.unwrap();
    batch.await.unwrap().unwrap();
    let reports = f.source.reports();
    assert_eq!(reports.len(), 3);
    assert!(matches!(reports[0], TurnReport::Returned { .. }));
    assert_eq!(failure_of(&reports[1]).cause, FailureCause::Commit);
    assert_eq!(reports[1], reports[2]);
    let (status, _) = outcome(&f.server, &m.id).await.unwrap();
    assert_eq!(status, OutcomeStatus::Failed);
    assert_eq!(total(&f.server, "zz").await, 0);
    assert!(f.server.pending(&Counter::new("zz").address(), None, 10).await.unwrap().unwrap().items.is_empty());
    assert_eq!(f.events.all(), ["start:Orphan:1", "dropped:Orphan"]);
    assert_eq!(Scripted::count(&f.source.releases), 0);
}

/// Scenario: A retryable failure's reschedule is refused.
#[tokio::test(start_paused = true)]
async fn a_refused_retry_of_a_failure_switches_to_the_failure_it_was_given() {
    let f = fixture().await;
    let policy = RetryPolicy::new(3, Backoff::fixed(secs(1)).unwrap()).unwrap();
    let options = SendOptions::new().retry(policy);
    let m = accepted(
        f.server.send_with(&Counter::new("a"), CounterMsg::Boom { reason: "boom".into() }, options).await.unwrap(),
    );
    let refusal = TurnFailure {
        cause: FailureCause::Handler,
        retryable: false,
        reason: "boom (commit refused: out of range)".into(),
    };
    f.source.script([Complete::Answer(CompleteTurnResult::Rejected(refusal.clone()))]);
    f.worker.run_due_turns(10).await.unwrap();
    let reports = f.source.reports();
    assert!(failure_of(&reports[0]).retryable);
    assert_eq!(reports[1], TurnReport::Failed(refusal));
    assert_eq!(
        outcome(&f.server, &m.id).await,
        Some((OutcomeStatus::Failed, Some("boom (commit refused: out of range)".into())))
    );
    assert_eq!(f.events.last(), "end:Failed(error=boom (commit refused: out of range)):sent=0");
}

/// Scenario: A retryable failure's reschedule is refused.
#[tokio::test(start_paused = true)]
async fn malformed_completion_results_abandon_the_turn_without_release() {
    let terminal = TurnFailure { cause: FailureCause::Commit, retryable: false, reason: "x".into() };
    let retryable = TurnFailure { cause: FailureCause::Handler, retryable: true, reason: "x".into() };
    let limit = TurnFailure { cause: FailureCause::ClaimLimit, retryable: false, reason: "x".into() };
    // A rejection of a failure that cannot be replaced, and rejections that carry a failure nobody may submit.
    let cases = [
        (CounterMsg::Add { n: 1 }, false, retryable),
        (CounterMsg::Add { n: 1 }, false, limit),
        (CounterMsg::Add { n: 1 }, true, terminal),
    ];
    for (message, undecodable, rejection) in cases {
        let f = fixture().await;
        let m = if undecodable {
            accepted(
                f.server
                    .send_message(&Counter::new("a").address(), "Add", json!({"n": "x"}), SendOptions::new())
                    .await
                    .unwrap(),
            )
        } else {
            send(&f.server, "a", message).await
        };
        f.source.script([Complete::Answer(CompleteTurnResult::Rejected(rejection.clone()))]);
        f.worker.run_due_turns(10).await.unwrap();
        assert_eq!(Scripted::count(&f.source.completes), 1, "{rejection:?}: no further attempt");
        assert_eq!(Scripted::count(&f.source.releases), 0, "{rejection:?}");
        assert!(f.events.last().starts_with("dropped:"), "{rejection:?}");
        assert!(matches!(f.server.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })));
    }
}

/// Scenario: A completion request never returns.
#[tokio::test(start_paused = true)]
async fn a_hung_completion_is_dropped_at_the_end_of_its_fixed_budget_and_never_released() {
    for (label, mode) in [("before", Complete::Hang), ("after", Complete::HangAfter)] {
        let f = fixture().await;
        let m = send(&f.server, "a", CounterMsg::Add { n: 1 }).await;
        f.source.script([mode]);
        let started = Instant::now();
        f.worker.run_due_turns(10).await.unwrap();
        let applied = label == "after";
        if applied {
            // The next renewal finds the lease gone: the lost lease cuts the call off.
            assert_eq!(started.elapsed(), secs(10), "{label}: at the first renewal");
        } else {
            assert_eq!(started.elapsed(), TTL, "{label}: one TTL from the start of the completion");
            assert!(Scripted::count(&f.source.renews) >= 2, "{label}: the keeper renewed meanwhile");
        }
        assert_eq!(Scripted::count(&f.source.completes), 1, "{label}: no other report");
        assert_eq!(Scripted::count(&f.source.releases), 0, "{label}");
        assert_eq!(f.events.all(), ["start:Add:1", "dropped:Add"], "{label}");
        assert_eq!(counter_log(&f.server, "a").await.len(), usize::from(applied), "{label}");
        let running = matches!(f.server.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. }));
        assert_eq!(running, !applied, "{label}");
    }
}

/// Scenario: Completion retry budget.
#[tokio::test(start_paused = true)]
async fn an_unavailable_source_is_retried_with_backoff_until_the_fixed_budget_is_spent() {
    let f = fixture().await;
    send(&f.server, "a", CounterMsg::Add { n: 1 }).await;
    f.source.script(std::iter::repeat_n(Complete::Unavailable, 100));
    let started = Instant::now();
    f.worker.run_due_turns(10).await.unwrap();
    // 50, 100, 200, 400, 800, 1600 ms, then every 2 s, within 30 s that renewals do not extend.
    assert_eq!(started.elapsed(), TTL);
    let attempts = Scripted::count(&f.source.completes);
    assert_eq!(attempts, 7 + 13, "{attempts} attempts");
    let reports = f.source.reports();
    assert!(reports.iter().all(|r| *r == reports[0]), "the identical report");
    assert_eq!(Scripted::count(&f.source.releases), 0);
    assert_eq!(f.events.all(), ["start:Add:1", "dropped:Add"]);
}

/// Scenario: Completion retry budget.
#[tokio::test(start_paused = true)]
async fn an_invalid_or_fatal_completion_ends_the_attempts_without_release() {
    let f = fixture().await;
    send(&f.server, "a", CounterMsg::Add { n: 1 }).await;
    f.source.script([Complete::Unavailable, Complete::Invalid]);
    f.worker.run_due_turns(10).await.unwrap();
    assert_eq!(Scripted::count(&f.source.completes), 2);
    assert_eq!(Scripted::count(&f.source.releases), 0);
    assert_eq!(f.events.all(), ["start:Add:1", "dropped:Add"]);
}

/// Scenario: Handler-end arbitration.
#[tokio::test(start_paused = true)]
async fn an_operator_cancel_received_through_renewal_before_the_handler_ended_completes_cancelled() {
    let f = fixture().await;
    let m = send(&f.server, "a", CounterMsg::Sleep { millis: 20_000 }).await;
    let batch = tokio::spawn({
        let worker = f.worker.clone();
        async move { worker.run_due_turns(10).await }
    });
    sleep(secs(1)).await;
    assert!(f.server.cancel_turn(&Counter::new("a").address()).await.unwrap());
    batch.await.unwrap().unwrap();
    assert_eq!(outcome(&f.server, &m.id).await.map(|o| o.0), Some(OutcomeStatus::Cancelled));
    assert_eq!(f.source.reports(), [TurnReport::Cancelled]);
    assert_eq!(f.events.all(), ["start:Sleep:1", "end:Cancelled:sent=0"]);
    assert!(counter_log(&f.server, "a").await.is_empty(), "nothing of the handler was committed");
    assert_eq!(Scripted::count(&f.source.releases), 0);
}

/// Scenario: Operator cancellation followed by shutdown.
#[tokio::test(start_paused = true)]
async fn an_operator_cancel_that_won_is_completed_cancelled_also_when_a_shutdown_follows() {
    for stop_at in [Duration::ZERO, Duration::from_millis(20)] {
        let f = fixture().await;
        let m = send(&f.server, "a", CounterMsg::Sleep { millis: 20_000 }).await;
        f.source.script([Complete::Unavailable]);
        f.worker.start().await.unwrap();
        sleep(secs(1)).await;
        assert!(f.server.cancel_turn(&Counter::new("a").address()).await.unwrap());
        // The renewal at 10 s brings the cancel; the shutdown comes at once or while the completion is retried.
        sleep(secs(9) + stop_at).await;
        f.worker.stop_with_grace(Duration::ZERO).await;
        assert_eq!(outcome(&f.server, &m.id).await.map(|o| o.0), Some(OutcomeStatus::Cancelled), "{stop_at:?}");
        assert_eq!(Scripted::count(&f.source.releases), 0, "{stop_at:?}: not released as unfinished work");
        assert_eq!(f.events.last(), "end:Cancelled:sent=0", "{stop_at:?}");
    }
}

/// Scenario: Handler-end arbitration.
#[tokio::test(start_paused = true)]
async fn a_shutdown_after_the_handler_ended_completes_within_the_budget_and_never_releases() {
    let f = fixture().await;
    let m = send(&f.server, "a", CounterMsg::Add { n: 1 }).await;
    f.source.script([Complete::Unavailable, Complete::Unavailable]);
    f.worker.start().await.unwrap();
    sleep(Duration::from_millis(60)).await;
    f.worker.stop_with_grace(Duration::ZERO).await;
    settle().await;
    sleep(secs(1)).await;
    assert_eq!(outcome(&f.server, &m.id).await.map(|o| o.0), Some(OutcomeStatus::Done));
    assert_eq!(Scripted::count(&f.source.releases), 0);
    assert_eq!(f.events.last(), "end:Done:sent=0");
}
