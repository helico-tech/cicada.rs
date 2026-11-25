//! An executor outside the system completes its leased turns with the report of each handler through the public core
//! API (`ActorSystem::complete_turn`; ADR 0038): returned, failed and cancelled reports, the outcome and retry the
//! server derives, the identities it gives sends, the storage limits it checks, the failure it returns for a commit the
//! store turns down, and peer, unstarted, stopped and closed servers. Public API only, on virtual time; a shorter run
//! on SQLite and PostgreSQL.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use cicada::actor::contract_of;
use cicada::prelude::*;
use cicada::runtime::{
    CompleteTurnResult, LeaseTurnsResult, NodeConfig, StateReport, TurnCompletion, TurnEffects, TurnFailure, TurnLease,
    TurnReport, TurnSend,
};
use cicada::store::{
    ActorCreate, Control, ControlEffect, Delivery, Message, MessageStatus, NodeId, OutcomeStatus, PostgresOptions,
    SqliteOptions, StoreError, TurnOutcome,
};
use cicada::testing::{TokioClock, default_start};
use cicada::time::{self, MAX_DELAY, MAX_INSTANT};
use cicada::watch::WatchChange;
use cicada::{Failed, FailureCause, OpError, SendResult, Stopped, Store, TypeName, WatchTarget};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::advance;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Counter {
    id: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CounterMsg {
    Add { n: i64 },
    Failed(Failed),
    Stopped(Stopped),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct CounterState {
    total: i64,
}

/// The counter type: the servers here only know its contract, except where one hosts it.
struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        if let CounterMsg::Add { n } = message {
            turn.state_mut().total += n;
        }
        Ok(())
    }
}

fn counter(id: &str) -> Counter {
    Counter { id: id.into() }
}

fn address(id: &str) -> ActorAddress {
    counter(id).address().unwrap()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn worker() -> NodeId {
    NodeId::new("worker").unwrap()
}

/// A store on tokio's paused clock.
fn store() -> Store {
    Store::memory_on(Arc::new(TokioClock::new(default_start())))
}

fn config(ttl: Duration) -> NodeConfig {
    NodeConfig { lease_ttl: ttl, ..NodeConfig::default() }
}

/// A system that hosts nothing, with a lease TTL of 30 s.
fn server(store: &Store) -> ActorSystem {
    ActorSystem::builder(store.clone()).config(config(secs(30))).build()
}

fn selection() -> BTreeMap<TypeName, String> {
    BTreeMap::from([(TypeName::parse("counter").unwrap(), contract_of::<CounterActor>().unwrap().hash().to_owned())])
}

/// Registers the counter's contract and creates the counters `ids`.
async fn setup(system: &ActorSystem, ids: &[&str]) {
    system.register_contract(&contract_of::<CounterActor>().unwrap()).await.unwrap();
    for id in ids {
        system.create(&counter(id), &CounterState::default()).await.unwrap();
    }
}

async fn send_with(system: &ActorSystem, id: &str, options: SendOptions) -> Message {
    match system.send_with(&counter(id), CounterMsg::Add { n: 1 }, options).await.unwrap() {
        SendResult::Accepted(m) => m,
        other => panic!("expected Accepted, got {other:?}"),
    }
}

async fn send(system: &ActorSystem, id: &str) -> Message {
    send_with(system, id, SendOptions::default()).await
}

/// Leases the one due turn.
async fn lease(system: &ActorSystem) -> TurnLease {
    match system.lease_turns(&worker(), &selection(), 10).await.unwrap() {
        LeaseTurnsResult::Leased(mut leases) if leases.len() == 1 => leases.remove(0),
        other => panic!("expected one lease, got {other:?}"),
    }
}

/// Sends `id` a message and leases its turn.
async fn leased(system: &ActorSystem, id: &str) -> TurnLease {
    send(system, id).await;
    lease(system).await
}

async fn complete(system: &ActorSystem, lease: &TurnLease, report: TurnReport) -> CompleteTurnResult {
    system.complete_turn(&lease.actor, lease.epoch, &lease.message.id, report).await.unwrap()
}

fn completed(result: CompleteTurnResult) -> TurnCompletion {
    match result {
        CompleteTurnResult::Completed(completion) => completion,
        other => panic!("expected Completed, got {other:?}"),
    }
}

fn rejected(result: CompleteTurnResult) -> TurnFailure {
    match result {
        CompleteTurnResult::Rejected(failure) => failure,
        other => panic!("expected Rejected, got {other:?}"),
    }
}

fn returned(state: StateReport, effects: TurnEffects) -> TurnReport {
    TurnReport::Returned { state, effects }
}

fn failure(cause: FailureCause, retryable: bool, reason: &str) -> TurnReport {
    TurnReport::Failed(TurnFailure { cause, retryable, reason: reason.to_owned() })
}

fn sends(sends: Vec<TurnSend>) -> TurnEffects {
    TurnEffects { sends, ..TurnEffects::default() }
}

fn add(to: &str, n: i64) -> TurnSend {
    TurnSend::new(address(to), "Add", json!({"n": n}))
}

async fn status(system: &ActorSystem, m: &cicada::store::MessageId) -> MessageStatus {
    system.status(m).await.unwrap().expect("a known message")
}

async fn outcome(system: &ActorSystem, lease: &TurnLease) -> (OutcomeStatus, Option<String>) {
    let record = system.outcome(&lease.message.id).await.unwrap().expect("an outcome");
    (record.status, record.error)
}

async fn mailbox(system: &ActorSystem, id: &str) -> Vec<Message> {
    system.pending(&address(id), None, 100).await.unwrap().map(|p| p.items).unwrap_or_default()
}

async fn state(system: &ActorSystem, id: &str) -> (Value, i64) {
    let snapshot = system.get_actor(&address(id)).await.unwrap().expect("the actor");
    (snapshot.state, snapshot.version)
}

/// Asserts that the turn of `lease` is still held, its message running without an outcome.
async fn still_held(system: &ActorSystem, lease: &TurnLease) {
    assert_eq!(system.running_turn(&lease.actor).await.unwrap().expect("held").message_id, lease.message.id);
    assert!(matches!(status(system, &lease.message.id).await, MessageStatus::Running { .. }));
}

fn deep(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, _| json!([inner]))
}

/// Scenario: A returned handler.
#[tokio::test(start_paused = true)]
async fn a_returned_report_commits_its_state_and_every_effect_together_and_returns_what_it_applied() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1", "c2", "c3", "c4"]).await;
    let purged = send(&server, "c4").await;
    server.pause_mailbox(&address("c4")).await.unwrap(); // so that only c1 is leased
    let held = leased(&server, "c1").await;
    let later = send_with(&server, "c1", SendOptions::new().delay(secs(60))).await;
    let mut to_c2 = add("c2", 5);
    to_c2.correlation_id = Some("order-7".into());
    to_c2.headers.insert("tenant".into(), "acme".into());
    let to_new = add("c5", 1);
    let effects = TurnEffects {
        watches: vec![WatchChange { target: WatchTarget::from(address("c2")), set: true, existing: false }],
        creates: vec![
            ActorCreate { address: address("c5"), state: json!({"total": 10}), child: false, pinned_to: None },
            ActorCreate { address: address("c6"), state: json!({"total": 0}), child: true, pinned_to: None },
        ],
        sends: vec![to_c2.clone(), to_new.clone()],
        cancels: vec![later.id.clone()],
        controls: vec![
            Control { effect: ControlEffect::Pause, actor: address("c3") },
            Control { effect: ControlEffect::Purge, actor: address("c4") },
        ],
    };

    let done = completed(complete(&server, &held, returned(StateReport::Set(json!({"total": 42})), effects)).await);

    assert_eq!((done.outcome, done.cause, done.version), (TurnOutcome::Done, None, 1));
    assert_eq!(done.sent.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec![to_c2.id.clone(), to_new.id.clone()]);
    for sent in &done.sent {
        assert_eq!((sent.from.as_ref(), sent.caused_by.as_ref()), (Some(&held.actor), Some(&held.message.id)));
    }
    assert_eq!((&done.sent[0].correlation_id, &done.sent[0].headers), (&to_c2.correlation_id, &to_c2.headers));
    assert_eq!(done.cancelled, vec![later.id.clone()]);
    assert_eq!(outcome(&server, &held).await, (OutcomeStatus::Done, None));
    assert_eq!(state(&server, "c1").await, (json!({"total": 42}), 1));
    assert!(server.running_turn(&held.actor).await.unwrap().is_none(), "the lease is gone");
    assert_eq!(mailbox(&server, "c2").await, vec![done.sent[0].clone()]);
    assert_eq!(mailbox(&server, "c5").await, vec![done.sent[1].clone()]);
    assert_eq!(state(&server, "c5").await, (json!({"total": 10}), 0));
    assert_eq!(server.get_actor(&address("c6")).await.unwrap().unwrap().parent, Some(held.actor.clone()));
    assert!(server.get_actor(&address("c3")).await.unwrap().unwrap().paused);
    assert_eq!(server.outcome(&purged.id).await.unwrap().unwrap().status, OutcomeStatus::Purged);
    assert_eq!(server.outcome(&later.id).await.unwrap().unwrap().status, OutcomeStatus::Cancelled);
    let watches = server.watches(Some(&held.actor), None, None, 10).await.unwrap().items;
    assert_eq!(watches.iter().map(|w| w.target.clone()).collect::<Vec<_>>(), vec![WatchTarget::from(address("c2"))]);
}

/// Scenario: No change, new state and stop.
#[tokio::test(start_paused = true)]
async fn unchanged_set_and_stop_change_the_state_as_a_turn_does() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1"]).await;

    let held = leased(&server, "c1").await;
    let done =
        completed(complete(&server, &held, returned(StateReport::Set(json!({"total": 3})), sends(vec![]))).await);
    assert_eq!(done.version, 1);
    let held = leased(&server, "c1").await;
    let done = completed(complete(&server, &held, returned(StateReport::Unchanged, sends(vec![]))).await);
    assert_eq!(done.version, 1);
    assert_eq!(state(&server, "c1").await, (json!({"total": 3}), 1), "state and version untouched");

    let held = leased(&server, "c1").await;
    let waiting = send(&server, "c1").await;
    let done = completed(complete(&server, &held, returned(StateReport::Stop, sends(vec![]))).await);
    assert_eq!((done.outcome, done.version), (TurnOutcome::Done, 1), "the version before removal");
    assert!(server.get_actor(&address("c1")).await.unwrap().is_none(), "stopped");
    assert_eq!(server.outcome(&waiting.id).await.unwrap().unwrap().status, OutcomeStatus::Purged);
}

/// Scenario: An invalid new own state.
#[tokio::test(start_paused = true)]
async fn a_null_nul_or_too_deep_new_state_fails_the_message_and_discards_every_effect() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1", "c2"]).await;
    for (value, problem) in [
        (Value::Null, "The state is null"),
        (json!({"total": "a\0b"}), "The state contains a NUL character"),
        (deep(65), "The state is nested deeper than 64 levels"),
    ] {
        let held = leased(&server, "c1").await;
        let report = returned(StateReport::Set(value), sends(vec![add("c2", 1)]));

        let done = completed(complete(&server, &held, report).await);

        let TurnOutcome::Failed(error) = &done.outcome else { panic!("failed, got {:?}", done.outcome) };
        assert!(error.starts_with(&format!("State cannot be stored: {problem}")), "{error}");
        assert_eq!((done.cause, done.version, done.sent.len()), (Some(FailureCause::Handler), 0, 0));
        assert!(mailbox(&server, "c2").await.is_empty(), "the send is discarded");
        assert_eq!(state(&server, "c1").await, (json!({"total": 0}), 0));
    }
}

/// Scenario: Derived send identity.
#[tokio::test(start_paused = true)]
async fn a_repeated_report_after_its_commit_applied_is_lease_lost_and_sends_nothing_twice() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1", "c2"]).await;
    let held = leased(&server, "c1").await;
    let report = returned(StateReport::Set(json!({"total": 1})), sends(vec![add("c2", 1)]));

    let done = completed(complete(&server, &held, report.clone()).await);
    let TurnReport::Returned { effects, .. } = &report else { unreachable!() };
    assert_eq!(done.sent[0].id, effects.sends[0].id, "the caller's id");

    assert_eq!(complete(&server, &held, report).await, CompleteTurnResult::LeaseLost);
    assert_eq!(mailbox(&server, "c2").await.len(), 1, "sent once");
    assert_eq!(state(&server, "c1").await, (json!({"total": 1}), 1), "applied once");
}

/// Scenario: Failed and cancelled reports.
#[tokio::test(start_paused = true)]
async fn failed_and_cancelled_reports_end_the_message_without_effects_and_watchers_hear_of_the_failure() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1", "w"]).await;
    server.watch(&WatchTarget::from(address("c1")), &address("w")).await.unwrap();
    server.pause_mailbox(&address("w")).await.unwrap(); // its notifications wait, unleased

    let held = leased(&server, "c1").await;
    let done = completed(complete(&server, &held, failure(FailureCause::Refused, false, "declined")).await);
    assert_eq!(
        (done.outcome, done.cause, done.sent.len()),
        (TurnOutcome::Failed("declined".into()), Some(FailureCause::Refused), 0)
    );
    assert_eq!(outcome(&server, &held).await, (OutcomeStatus::Failed, Some("declined".into())));
    let notes = mailbox(&server, "w").await;
    assert_eq!(notes.len(), 1);
    assert_eq!((notes[0].message_type.as_str(), notes[0].notification), ("Failed", true));
    assert_eq!(notes[0].payload["cause"], json!("refused"));
    assert_eq!(notes[0].payload["messageId"], json!(held.message.id.as_str()));

    let held = leased(&server, "c1").await;
    let done = completed(complete(&server, &held, TurnReport::Cancelled).await);
    assert_eq!((done.outcome, done.cause), (TurnOutcome::Cancelled, None));
    assert_eq!(outcome(&server, &held).await, (OutcomeStatus::Cancelled, None));
    assert_eq!(mailbox(&server, "w").await.len(), 1, "a cancel tells nobody");
    assert_eq!(state(&server, "c1").await, (json!({"total": 0}), 0));
}

/// Scenario: Bad failure combination. Scenario: No retry remains.
#[tokio::test(start_paused = true)]
async fn every_failure_combination_is_checked_before_the_store_and_each_allowed_one_fails_a_message_without_policy() {
    use FailureCause::*;
    let store = store();
    let server = server(&store);
    setup(&server, &["c1"]).await;
    let causes = [Handler, Node, Refused, Timeout, Message, State, Commit, ClaimLimit];
    let allowed = |cause: FailureCause, retryable: bool| match cause {
        Handler | Node | Timeout => true,
        Refused | Message | State | Commit => !retryable,
        ClaimLimit => false,
    };
    for cause in causes {
        for retryable in [false, true] {
            if allowed(cause, retryable) {
                let held = leased(&server, "c1").await;
                let done = completed(complete(&server, &held, failure(cause, retryable, "no")).await);
                assert_eq!((done.outcome, done.cause), (TurnOutcome::Failed("no".into()), Some(cause)), "{cause}");
            }
        }
    }
    let held = leased(&server, "c1").await;
    store.close().await;
    for cause in causes {
        for retryable in [false, true] {
            let result =
                server.complete_turn(&held.actor, held.epoch, &held.message.id, failure(cause, retryable, "no"));
            match result.await {
                Err(OpError::Invalid(_)) => assert!(!allowed(cause, retryable), "{cause} {retryable}"),
                Err(OpError::Store(StoreError::Unavailable(_))) => assert!(allowed(cause, retryable)),
                other => panic!("{cause} {retryable}: {other:?}"),
            }
        }
    }
}

/// Scenario: Failure of a currently running message. Scenario: No retry remains.
#[tokio::test(start_paused = true)]
async fn a_retryable_failure_is_rescheduled_by_the_messages_policy_until_its_attempts_are_spent() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1"]).await;
    let policy = RetryPolicy::new(3, Backoff::exponential(secs(10), 2.0, secs(15)).unwrap()).unwrap();
    let m = send_with(&server, "c1", SendOptions::new().retry(policy)).await;

    for (failures, delay) in [(1, secs(10)), (2, secs(15))] {
        let held = lease(&server).await;
        let now = server.now().await.unwrap();
        let done = completed(complete(&server, &held, failure(FailureCause::Node, true, "store down")).await);
        let expected = TurnOutcome::Rescheduled { delivery: Delivery::After(delay), error: "store down".into() };
        assert_eq!((done.outcome, done.cause, done.sent.len()), (expected, None, 0));
        let MessageStatus::Pending(waiting) = status(&server, &m.id).await else { panic!("pending") };
        assert_eq!((waiting.failures, waiting.last_error.as_deref()), (failures, Some("store down")));
        assert_eq!(waiting.deliver_at, now + chrono::Duration::from_std(delay).unwrap());
        assert!(server.running_turn(&held.actor).await.unwrap().is_none());
        advance(delay).await;
    }
    let held = lease(&server).await;
    let done = completed(complete(&server, &held, failure(FailureCause::Node, true, "store down")).await);
    assert_eq!((done.outcome, done.cause), (TurnOutcome::Failed("store down".into()), Some(FailureCause::Node)));

    // Not retryable, or without a policy: failed at once.
    let m = send_with(
        &server,
        "c1",
        SendOptions::new().retry(RetryPolicy::new(3, Backoff::fixed(secs(1)).unwrap()).unwrap()),
    )
    .await;
    let held = lease(&server).await;
    assert_eq!(held.message.id, m.id);
    let done = completed(complete(&server, &held, failure(FailureCause::Handler, false, "no")).await);
    assert_eq!(done.outcome, TurnOutcome::Failed("no".into()));
    let held = leased(&server, "c1").await;
    let done = completed(complete(&server, &held, failure(FailureCause::Timeout, true, "timeout")).await);
    assert_eq!((done.outcome, done.cause), (TurnOutcome::Failed("timeout".into()), Some(FailureCause::Timeout)));
}

/// Scenario: An expired but still held message.
#[tokio::test(start_paused = true)]
async fn an_expired_lease_nobody_replaced_uses_the_policy_of_its_pending_message() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1"]).await;
    let policy = RetryPolicy::new(2, Backoff::fixed(secs(5)).unwrap()).unwrap();
    let m = send_with(&server, "c1", SendOptions::new().retry(policy)).await;
    let held = lease(&server).await;
    advance(secs(40)).await;
    assert!(matches!(status(&server, &m.id).await, MessageStatus::Pending(_)), "expired");

    let done = completed(complete(&server, &held, failure(FailureCause::Handler, true, "flaky")).await);

    assert!(matches!(done.outcome, TurnOutcome::Rescheduled { .. }), "{:?}", done.outcome);
    let MessageStatus::Pending(waiting) = status(&server, &m.id).await else { panic!("pending") };
    assert_eq!(waiting.failures, 1);
}

/// Scenario: A refused reschedule.
#[tokio::test(start_paused = true)]
async fn a_reschedule_the_store_cannot_schedule_is_rejected_with_the_failure_to_submit_next() {
    let start = time::parse_instant("9500-01-01T00:00:00Z").unwrap();
    let store = Store::memory_on(Arc::new(TokioClock::new(start)));
    let server = server(&store);
    setup(&server, &["c1"]).await;
    let policy = RetryPolicy::new(5, Backoff::fixed(MAX_DELAY).unwrap()).unwrap();
    send_with(&server, "c1", SendOptions::new().retry(policy)).await;
    let held = lease(&server).await;

    let next = rejected(complete(&server, &held, failure(FailureCause::Handler, true, "flaky")).await);

    assert_eq!((next.cause, next.retryable), (FailureCause::Handler, false));
    assert!(next.reason.starts_with("flaky (commit refused: "), "{}", next.reason);
    still_held(&server, &held).await;
    let done = completed(complete(&server, &held, TurnReport::Failed(next.clone())).await);
    assert_eq!((done.outcome, done.cause), (TurnOutcome::Failed(next.reason.clone()), Some(FailureCause::Handler)));
}

/// Scenario: Identity and structural refusal. Scenario: A known rejection. Scenario: A refused terminal failure.
#[tokio::test(start_paused = true)]
async fn a_commit_the_store_turns_down_is_rejected_and_a_turned_down_failure_is_invalid() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1", "parent"]).await;
    let held = leased(&server, "c1").await;
    let wrong = send_with(&server, "parent", SendOptions::new().delay(secs(60))).await.id;
    let ok = || returned(StateReport::Set(json!({"total": 9})), sends(vec![]));
    let with = |effects: TurnEffects| returned(StateReport::Unchanged, effects);
    let control = |effect, id: &str| Control { effect, actor: address(id) };
    let create =
        |id: &str, child| ActorCreate { address: address(id), state: json!({"total": 0}), child, pinned_to: None };

    // A wrong message id: refused, and so is the failure that follows with the same id.
    let next = rejected(server.complete_turn(&held.actor, held.epoch, &wrong, ok()).await.unwrap());
    assert_eq!((next.cause, next.retryable), (FailureCause::Commit, false));
    assert!(next.reason.starts_with("Commit refused: Message "), "{}", next.reason);
    let refused = server.complete_turn(&held.actor, held.epoch, &wrong, TurnReport::Failed(next)).await;
    assert!(matches!(refused, Err(OpError::Invalid(_))), "{refused:?}");
    still_held(&server, &held).await;

    let cases = [
        (with(TurnEffects { cancels: vec![held.message.id.clone()], ..TurnEffects::default() }), "Commit refused: "),
        (with(sends(vec![add("absent", 1)])), "Send to absent instance counter/absent"),
        (
            with(TurnEffects { controls: vec![control(ControlEffect::Delete, "c1")], ..TurnEffects::default() }),
            "Commit refused: ",
        ),
        (
            with(TurnEffects {
                controls: vec![control(ControlEffect::Pause, "parent"), control(ControlEffect::Resume, "parent")],
                ..TurnEffects::default()
            }),
            "Commit refused: ",
        ),
        (
            with(TurnEffects { creates: vec![create("n", true), create("n", false)], ..TurnEffects::default() }),
            "Commit refused: ",
        ),
        (
            with(TurnEffects { creates: vec![create("parent", true)], ..TurnEffects::default() }),
            "Child create of counter/parent",
        ),
    ];
    for (report, start) in cases {
        let next = rejected(complete(&server, &held, report.clone()).await);
        assert_eq!((next.cause, next.retryable), (FailureCause::Commit, false), "{report:?}");
        assert!(next.reason.starts_with(start), "{}", next.reason);
        still_held(&server, &held).await;
        assert_eq!(state(&server, "c1").await, (json!({"total": 0}), 0), "nothing applied");
    }

    let next = rejected(complete(&server, &held, with(sends(vec![add("absent", 1)]))).await);
    let done = completed(complete(&server, &held, TurnReport::Failed(next.clone())).await);
    assert_eq!((done.outcome, done.cause), (TurnOutcome::Failed(next.reason), Some(FailureCause::Commit)));
}

/// Scenario: Identity and structural refusal, a delete of an ancestor.
#[tokio::test(start_paused = true)]
async fn a_report_that_deletes_an_ancestor_is_rejected() {
    let store = store();
    let server = server(&store);
    setup(&server, &["parent"]).await;
    let held = leased(&server, "parent").await;
    let effects = TurnEffects {
        creates: vec![ActorCreate {
            address: address("child"),
            state: json!({"total": 0}),
            child: true,
            pinned_to: None,
        }],
        ..TurnEffects::default()
    };
    completed(complete(&server, &held, returned(StateReport::Unchanged, effects)).await);
    let held = leased(&server, "child").await;
    let effects = TurnEffects {
        controls: vec![Control { effect: ControlEffect::Delete, actor: address("parent") }],
        ..TurnEffects::default()
    };

    let next = rejected(complete(&server, &held, returned(StateReport::Unchanged, effects)).await);

    assert!(next.reason.starts_with("Delete of counter/parent"), "{}", next.reason);
    still_held(&server, &held).await;
}

/// Scenario: An invalid send or creation value.
#[tokio::test(start_paused = true)]
async fn effects_beyond_the_storage_limits_fail_the_message_with_cause_commit_and_no_effects() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1", "c2"]).await;
    let too_late = time::from_micros(time::to_micros(MAX_INSTANT) + 1);
    let mut long_id = add("c2", 1);
    long_id.id = cicada::store::MessageId::new("x".repeat(201)).unwrap();
    let mut nul = add("c2", 1);
    nul.payload = json!({"n": "a\0b"});
    let mut late = add("c2", 1);
    late.delivery = Delivery::At(too_late);
    let mut header = add("c2", 1);
    header.headers.insert("h".into(), "v".repeat(4_001));
    let create = |state: Value| TurnEffects {
        creates: vec![ActorCreate { address: address("c9"), state, child: false, pinned_to: None }],
        sends: vec![add("c2", 1)],
        ..TurnEffects::default()
    };
    let cases = [
        (StateReport::Unchanged, sends(vec![add("c2", 1), long_id]), "The id is too long"),
        (StateReport::Unchanged, sends(vec![nul]), "The payload contains a NUL character"),
        (StateReport::Unchanged, sends(vec![late]), "deliverAt"),
        (StateReport::Unchanged, sends(vec![header]), "The header 'h' is too long"),
        (StateReport::Stop, create(Value::Null), "The initial state is null"),
        (StateReport::Unchanged, create(json!({"total": "\0"})), "The initial state contains a NUL character"),
        (StateReport::Set(json!({"total": 1})), create(deep(65)), "The initial state is nested deeper than 64"),
    ];
    for (state_report, effects, problem) in cases {
        let held = leased(&server, "c1").await;

        let done = completed(complete(&server, &held, returned(state_report, effects)).await);

        let TurnOutcome::Failed(error) = &done.outcome else { panic!("failed, got {:?}", done.outcome) };
        assert!(error.starts_with("Commit refused: "), "{error}");
        assert!(error.contains(problem), "{error}");
        assert_eq!((done.cause, done.sent.len()), (Some(FailureCause::Commit), 0));
        assert!(mailbox(&server, "c2").await.is_empty());
        assert!(server.get_actor(&address("c9")).await.unwrap().is_none());
        assert_eq!(state(&server, "c1").await, (json!({"total": 0}), 0), "still there, unchanged");
    }
}

/// Scenario: Raw effects are not an operation log.
#[tokio::test(start_paused = true)]
async fn a_send_and_a_cancel_of_its_id_in_one_report_get_the_stores_order() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1"]).await;
    let held = leased(&server, "c1").await;
    let to_self = add("c1", 1);
    let effects = TurnEffects { cancels: vec![to_self.id.clone()], ..sends(vec![to_self.clone()]) };

    let done = completed(complete(&server, &held, returned(StateReport::Unchanged, effects)).await);

    assert_eq!(done.sent.iter().map(|m| &m.id).collect::<Vec<_>>(), vec![&to_self.id], "the cancel came first");
    assert!(done.cancelled.is_empty());
    assert!(matches!(status(&server, &to_self.id).await, MessageStatus::Pending(_)));
}

/// Scenario: A server hosts a target implementation.
#[tokio::test(start_paused = true)]
async fn the_types_a_server_hosts_do_not_change_what_it_completes() {
    let store = store();
    let plain = server(&store);
    let hosting = ActorSystem::builder(store.clone()).host(CounterActor).config(config(secs(30))).build();
    setup(&plain, &["c1", "c2"]).await;
    // A state no counter can decode: a worker's checks would refuse it, the server's do not.
    let undecodable = |id: &str| TurnEffects {
        creates: vec![ActorCreate {
            address: address(id),
            state: json!({"total": "many"}),
            child: false,
            pinned_to: None,
        }],
        ..TurnEffects::default()
    };
    for (system, actor, created) in [(&plain, "c1", "x1"), (&hosting, "c2", "x2")] {
        let held = leased(system, actor).await;
        let done = completed(complete(system, &held, returned(StateReport::Unchanged, undecodable(created))).await);
        assert_eq!(done.outcome, TurnOutcome::Done);
        assert_eq!(state(system, created).await, (json!({"total": "many"}), 0));
    }
}

/// Scenario: Peer and stopped servers.
#[tokio::test(start_paused = true)]
async fn a_lease_completes_through_an_unstarted_peer_with_an_invalid_configuration_and_through_a_stopped_system() {
    let store = store();
    let granting = server(&store);
    setup(&granting, &["c1", "c2"]).await;
    let broken = ActorSystem::builder(store.clone()).host(CounterActor).config(config(Duration::ZERO)).build();
    let stopped = server(&store);
    stopped.start().await.unwrap();
    stopped.stop().await;

    let held = leased(&granting, "c1").await;
    let done =
        completed(complete(&broken, &held, returned(StateReport::Set(json!({"total": 1})), sends(vec![]))).await);
    assert_eq!(done.version, 1);
    let held = leased(&granting, "c2").await;
    let done = completed(complete(&stopped, &held, failure(FailureCause::Handler, false, "no")).await);
    assert_eq!(done.outcome, TurnOutcome::Failed("no".into()));
    assert_eq!(broken.state(), cicada::runtime::NodeState::New);
}

/// Scenario: Closed store.
#[tokio::test(start_paused = true)]
async fn a_closed_store_answers_unavailable_and_nothing_is_released() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1"]).await;
    let held = leased(&server, "c1").await;
    store.close().await;

    let result = server.complete_turn(&held.actor, held.epoch, &held.message.id, TurnReport::Cancelled).await;

    assert!(matches!(result, Err(OpError::Store(StoreError::Unavailable(_)))), "{result:?}");
}

/// Scenario: Reason normalization.
#[tokio::test(start_paused = true)]
async fn a_nul_in_a_reason_is_replaced_and_only_a_rescheduled_error_is_cut() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1"]).await;
    let long = "e".repeat(5_000);

    let held = leased(&server, "c1").await;
    let done = completed(complete(&server, &held, failure(FailureCause::Handler, false, "a\0b")).await);
    assert_eq!(done.outcome, TurnOutcome::Failed("a\u{FFFD}b".into()));
    let held = leased(&server, "c1").await;
    let done = completed(complete(&server, &held, failure(FailureCause::Handler, false, &long)).await);
    assert_eq!(done.outcome, TurnOutcome::Failed(long.clone()), "no new cap on a terminal reason");

    let policy = RetryPolicy::new(2, Backoff::fixed(secs(1)).unwrap()).unwrap();
    let m = send_with(&server, "c1", SendOptions::new().retry(policy)).await;
    let held = lease(&server).await;
    completed(complete(&server, &held, failure(FailureCause::Handler, true, &format!("\0{long}"))).await);
    let MessageStatus::Pending(waiting) = status(&server, &m.id).await else { panic!("pending") };
    let last_error = waiting.last_error.unwrap();
    assert_eq!(last_error.chars().count(), TurnOutcome::MAX_ERROR_LENGTH);
    assert!(last_error.starts_with("\u{FFFD}eee"), "{last_error}");
}

/// The same completions on any store: a returned report, its repeat, a rejection and the failure that follows it.
async fn completes_on(store: Store) {
    let server = server(&store);
    setup(&server, &["c1", "c2"]).await;
    server.pause_mailbox(&address("c2")).await.unwrap(); // what it is sent waits, unleased
    let held = leased(&server, "c1").await;
    let report = returned(StateReport::Set(json!({"total": 2})), sends(vec![add("c2", 1)]));
    let done = completed(complete(&server, &held, report.clone()).await);
    assert_eq!((done.version, done.sent.len()), (1, 1));
    assert_eq!(done.sent[0].from.as_ref(), Some(&held.actor));
    assert_eq!(complete(&server, &held, report).await, CompleteTurnResult::LeaseLost);
    assert_eq!(mailbox(&server, "c2").await.len(), 1);

    let held = leased(&server, "c1").await;
    let next = rejected(complete(&server, &held, returned(StateReport::Unchanged, sends(vec![add("zz", 1)]))).await);
    still_held(&server, &held).await;
    let done = completed(complete(&server, &held, TurnReport::Failed(next)).await);
    assert_eq!(done.cause, Some(FailureCause::Commit));
    assert_eq!(state(&server, "c1").await, (json!({"total": 2}), 1));
    store.close().await;
}

/// The completions on SQLite.
#[tokio::test]
async fn reports_complete_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    completes_on(Store::sqlite(dir.path().join("cicada.db"), SqliteOptions::default()).await.unwrap()).await;
}

/// The completions on PostgreSQL (skipped without `CICADA_PG_URL`).
#[tokio::test(flavor = "multi_thread")]
async fn reports_complete_on_postgres() {
    let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
        return;
    };
    let schema = format!("completion_{}_{}", std::process::id(), &cicada::store::MessageId::random().as_str()[..8]);
    completes_on(Store::postgres(&url, PostgresOptions::for_schema(&schema)).await.unwrap()).await;
    use sqlx::Connection;
    let mut c = sqlx::PgConnection::connect(&url).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA \"{schema}\" CASCADE"))).execute(&mut c).await.unwrap();
}

/// Scenarios: Reject inconsistent buffered creates; Wire and client compatibility (ADR 0052). A returned create applies
/// its pin; a pin beyond its bounds is invalid before the store, with the lease still held; creates of one address
/// that disagree on their pin are refused by the store and replaced by a failure, which applies no pin.
#[tokio::test(start_paused = true)]
async fn a_returned_create_applies_its_pin_and_bad_or_disagreeing_pins_are_refused() {
    let store = store();
    let server = server(&store);
    setup(&server, &["c1"]).await;
    let pinned = |id: &str, pin: &str| ActorCreate {
        address: address(id),
        state: json!({"total": 0}),
        child: false,
        pinned_to: Some(NodeId::new(pin).unwrap()),
    };
    let with =
        |creates: Vec<ActorCreate>| returned(StateReport::Unchanged, TurnEffects { creates, ..TurnEffects::default() });

    let held = leased(&server, "c1").await;
    for bad in ["x".repeat(201), "a\0b".to_owned()] {
        let refused = server.complete_turn(&held.actor, held.epoch, &held.message.id, with(vec![pinned("c9", &bad)]));
        assert!(matches!(refused.await, Err(OpError::Invalid(_))), "a pin beyond its bounds");
    }
    still_held(&server, &held).await;
    let unpinned = ActorCreate { pinned_to: None, ..pinned("c8", "host-a") };
    let next = rejected(complete(&server, &held, with(vec![pinned("c8", "host-a"), unpinned])).await);
    assert_eq!(next.cause, FailureCause::Commit);
    assert!(server.get_actor(&address("c8")).await.unwrap().is_none(), "nothing is created");
    completed(complete(&server, &held, TurnReport::Failed(next)).await);

    let held = leased(&server, "c1").await;
    completed(complete(&server, &held, with(vec![pinned("c7", "host-a")])).await);
    let snapshot = server.get_actor(&address("c7")).await.unwrap().unwrap();
    assert_eq!(snapshot.pinned_to, Some(NodeId::new("host-a").unwrap()));
}
