//! The scheduler, on virtual time with `TestActors`: its state and messages, a delivery on each slot, the slot header,
//! missed and early slots, the operator's `Run` and `Update`, its own ticks, failed deliveries and a chain that ends.

mod common;

use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::{NodeConfig, RetryResult};
use cicada::store::{Message, OutcomeStatus};
use cicada::time::Timestamp;
use cicada::{Failed, FailureCause, OpError, WatchTarget};
use cicada_actors::{
    DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, Schedule, Scheduler, SchedulerActor, SchedulerMsg,
    SchedulerState, StoredMessage,
};
use cicada_test::{TestActors, claim_and_abandon};
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MINUTE: Duration = Duration::from_secs(60);

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn nightly() -> Scheduler {
    Scheduler::named("nightly")
}

fn every(seconds: i64) -> Schedule {
    Schedule::every(seconds).unwrap()
}

fn daily(hour: u32, minute: u32) -> Schedule {
    Schedule::daily(hour, minute).unwrap()
}

/// A scheduler that sends `worker/a` a `Note` with `text` on each slot of `schedule`.
fn note(schedule: Schedule, text: &str) -> SchedulerState {
    SchedulerState::new(schedule, &worker("a"), WorkerMsg::Note { text: text.into() }).unwrap()
}

/// A message of a family that the worker does not take.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Shout {
    Shout { text: String },
}

/// A node at `start` with the scheduler type, the workers `worker/a` and `worker/b`, the dead letters (not created) and
/// the witness, which watches the schedulers.
async fn node_with(start: &str, config: Option<NodeConfig>) -> TestActors {
    let test = TestActors::builder()
        .start_at(ts(start))
        .with(move |node| {
            let node = node.host(SchedulerActor).host(WorkerActor).host(WitnessActor).host(DeadLettersActor::default());
            match config {
                Some(config) => node.config(config),
                None => node,
            }
        })
        .build()
        .await
        .unwrap();
    for id in ["a", "b"] {
        test.create(&worker(id), &WorkerState::default()).await;
    }
    witness(&test, "cicada.scheduler").await;
    test
}

async fn node_at(start: &str) -> TestActors {
    node_with(start, None).await
}

async fn node() -> TestActors {
    node_at("2026-01-01T00:00:00Z").await
}

/// Creates `nightly` with `state`.
async fn created(test: &TestActors, state: SchedulerState) {
    test.create(&nightly(), &state).await;
}

/// Creates `nightly` with `state` and starts it.
async fn started(test: &TestActors, state: SchedulerState) {
    created(test, state).await;
    send(test, SchedulerMsg::Start).await;
}

/// Sends `nightly` `message` from a client and runs what is due.
async fn send(test: &TestActors, message: SchedulerMsg) -> Message {
    let m = test.send(&nightly(), message).await;
    test.run_until_idle().await;
    m
}

async fn held(test: &TestActors) -> SchedulerState {
    test.state(&nightly()).await
}

async fn log(test: &TestActors, id: &str) -> Vec<String> {
    test.state::<_, WorkerState>(&worker(id)).await.log
}

/// The `Failed` the witness was told of the runs of the scheduler.
async fn failed_runs(test: &TestActors) -> Vec<Failed> {
    told(test).await.into_iter().filter(|f| f.message_type == "Run").collect()
}

/// The one pending message of the scheduler, which is its armed tick.
async fn armed(test: &TestActors) -> Message {
    let pending = test.pending(&nightly()).await;
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].message_type, "Tick");
    assert_eq!(held(test).await.armed_tick.as_ref(), Some(&pending[0].id), "the pending tick is the armed one");
    pending.into_iter().next().unwrap()
}

/// A value `levels` deep: a chain of arrays around a number, built without recursion.
fn nested(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, _| Value::Array(vec![inner]))
}

fn two_attempts() -> RetryPolicy {
    RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(10)).unwrap()).unwrap()
}

// The types.

#[test]
fn the_state_and_the_update_of_the_design_decode_to_the_values_they_name() {
    let state: SchedulerState = serde_json::from_value(json!({
        "schedule": {"type": "daily", "hour": 2, "minute": 0}, "target": "reports/_",
        "message": {"type": "Build", "payload": {"full": true}},
        "retry": {"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT5M"}}
    }))
    .unwrap();
    assert_eq!(state.schedule, daily(2, 0));
    assert_eq!(state.target.as_str(), "reports/_");
    assert_eq!(state.message, StoredMessage { message_type: "Build".into(), payload: json!({"full": true}) });
    let retry = RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(300)).unwrap()).unwrap();
    assert_eq!(state.retry, Some(retry));
    assert_eq!((state.next_run, state.armed_tick, state.runs, state.last_run), (None, None, 0, None));

    let update = cicada::message::decode::<SchedulerMsg>(
        "Update",
        &json!({"message": {"type": "Build", "payload": {"full": false}}}),
    )
    .unwrap();
    let message = StoredMessage { message_type: "Build".into(), payload: json!({"full": false}) };
    assert_eq!(update, SchedulerMsg::Update { schedule: None, target: None, message: Some(message), retry: None });
    assert_eq!(cicada::message::decode::<SchedulerMsg>("Run", &json!({})).unwrap(), SchedulerMsg::Run { slot: None });
}

// A scheduler keeps a schedule, a target and a message.

/// Scenario: Not started.
#[tokio::test(start_paused = true)]
async fn a_scheduler_that_was_not_started_delivers_nothing() {
    let test = node().await;
    created(&test, note(every(60), "tick")).await;
    test.advance_time_by(3 * MINUTE).await;
    assert!(log(&test, "a").await.is_empty());
    assert!(test.pending(&nightly()).await.is_empty());
}

// A scheduler sends its message to its target on each slot.

/// Scenario: A daily schedule.
#[tokio::test(start_paused = true)]
async fn a_daily_schedule_delivers_once_at_its_slot_from_the_scheduler() {
    let test = node_at("2026-01-01T10:00:00Z").await;
    test.system().pause_mailbox(&addr(&worker("a"))).await.unwrap();
    let mut state = note(daily(2, 0), "nightly");
    state.retry = Some(two_attempts());
    started(&test, state).await;
    assert_eq!(held(&test).await.next_run, Some(ts("2026-01-02T02:00:00Z")));
    test.advance_time_by(Duration::from_secs(16 * 3600)).await;
    let pending = test.pending(&worker("a")).await;
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!((pending[0].message_type.as_str(), &pending[0].payload), ("Note", &json!({"text": "nightly"})));
    assert_eq!(pending[0].from, Some(addr(&nightly())));
    assert_eq!(pending[0].retry, Some(two_attempts()));
    assert_eq!(pending[0].correlation_id, None);
    let state = held(&test).await;
    assert_eq!((state.runs, state.last_run), (1, Some(ts("2026-01-02T02:00:00Z"))));
    assert_eq!(state.next_run, Some(ts("2026-01-03T02:00:00Z")));
}

/// Scenario: Each slot delivers.
#[tokio::test(start_paused = true)]
async fn each_slot_delivers_once() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    test.advance_time_by(3 * MINUTE).await;
    assert_eq!(log(&test, "a").await, ["tick", "tick", "tick"]);
    assert_eq!(held(&test).await.runs, 3);
}

/// Scenario: A second start.
#[tokio::test(start_paused = true)]
async fn a_second_start_keeps_one_tick_armed() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    let first = armed(&test).await;
    send(&test, SchedulerMsg::Start).await;
    let second = armed(&test).await;
    assert_ne!(first.id, second.id);
    assert_eq!(status(&test, &first).await, Some(OutcomeStatus::Cancelled));
    test.advance_time_by(MINUTE).await;
    assert_eq!(log(&test, "a").await, ["tick"]);
}

/// Scenario: The target's run is retried.
#[tokio::test(start_paused = true)]
async fn the_run_of_the_target_is_retried_by_the_retry_policy_of_the_scheduler() {
    let test = node().await;
    let mut state = SchedulerState::new(every(60), &worker("a"), WorkerMsg::FailOnce { text: "again".into() }).unwrap();
    state.retry = Some(two_attempts());
    started(&test, state).await;
    test.advance_time_by(MINUTE).await;
    assert!(log(&test, "a").await.is_empty(), "the first run failed");
    test.advance_time_by(Duration::from_secs(10)).await;
    assert_eq!(log(&test, "a").await, ["again"]);
}

/// Without a retry policy a failed run of the target ends `failed` at the target.
#[tokio::test(start_paused = true)]
async fn without_a_retry_policy_the_run_of_the_target_is_not_retried() {
    let test = node().await;
    started(&test, SchedulerState::new(every(60), &worker("a"), WorkerMsg::FailOnce { text: "again".into() }).unwrap())
        .await;
    test.advance_time_by(MINUTE + Duration::from_secs(30)).await;
    assert!(log(&test, "a").await.is_empty());
    let failures = test.failures().await;
    assert!(failures.iter().any(|f| f.envelope.message_type == "FailOnce"), "{failures:?}");
}

// The target learns its slot.

/// Scenario: A late tick.
#[tokio::test(start_paused = true)]
async fn a_late_tick_names_its_slot_in_the_header() {
    let test = node_at("2026-01-01T10:00:00Z").await;
    started(&test, note(daily(2, 0), "nightly")).await;
    test.system().pause_mailbox(&addr(&nightly())).await.unwrap();
    test.advance_time_by(Duration::from_secs(17 * 3600)).await;
    test.system().pause_mailbox(&addr(&worker("a"))).await.unwrap();
    test.system().resume_mailbox(&addr(&nightly())).await.unwrap();
    test.run_until_idle().await;
    let pending = test.pending(&worker("a")).await;
    assert_eq!(pending.len(), 1);
    let slot = pending[0].headers.get("cicada-slot").expect("the header");
    assert_eq!(slot.as_bytes(), b"2026-01-02T02:00:00Z");
    assert_eq!(held(&test).await.last_run, Some(ts("2026-01-02T03:00:00Z")));
}

/// Scenario: A run sent by an operator.
#[tokio::test(start_paused = true)]
async fn a_run_without_a_slot_has_no_slot_header() {
    let test = node().await;
    test.system().pause_mailbox(&addr(&worker("a"))).await.unwrap();
    created(&test, note(every(60), "now")).await;
    send(&test, SchedulerMsg::Run { slot: None }).await;
    let pending = test.pending(&worker("a")).await;
    assert_eq!(pending.len(), 1);
    assert!(!pending[0].headers.contains_key("cicada-slot"), "{:?}", pending[0].headers);
}

// A scheduler skips the slots it missed.

/// Scenario: A pause over three slots.
#[tokio::test(start_paused = true)]
async fn a_pause_over_three_slots_delivers_once_at_the_resume() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    test.advance_time_by(Duration::from_secs(30)).await;
    test.system().pause_mailbox(&addr(&nightly())).await.unwrap();
    test.advance_time_by(3 * MINUTE).await;
    assert!(log(&test, "a").await.is_empty());
    test.system().resume_mailbox(&addr(&nightly())).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(log(&test, "a").await, ["tick"], "once at the resume");
    assert_eq!(held(&test).await.next_run, Some(ts("2026-01-01T00:04:00Z")));
    test.advance_time_by(2 * MINUTE).await;
    assert_eq!(log(&test, "a").await.len(), 3, "then once at each slot");
}

/// Scenario: An early tick.
#[tokio::test(start_paused = true)]
async fn an_early_tick_delivers_its_slot_once_and_arms_the_slot_after_it() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    test.advance_time_by(Duration::from_secs(40)).await;
    let tick = armed(&test).await;
    test.system().deliver_now(&tick.id).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(log(&test, "a").await, ["tick"]);
    assert_eq!(held(&test).await.next_run, Some(ts("2026-01-01T00:02:00Z")));
    test.advance_time_by(Duration::from_secs(50)).await;
    assert_eq!(log(&test, "a").await, ["tick"], "the slot 00:01 is not delivered again");
    test.advance_time_by(Duration::from_secs(30)).await;
    assert_eq!(log(&test, "a").await.len(), 2);
}

/// Scenario: A delete.
#[tokio::test(start_paused = true)]
async fn a_deleted_scheduler_delivers_nothing_more() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    let tick = armed(&test).await;
    test.system().delete_actor(&addr(&nightly())).await.unwrap();
    test.advance_time_by(3 * MINUTE).await;
    assert!(log(&test, "a").await.is_empty());
    assert_eq!(status(&test, &tick).await, Some(OutcomeStatus::Purged));
}

// An operator runs a schedule now.

/// Scenario: Run now.
#[tokio::test(start_paused = true)]
async fn a_run_between_two_slots_delivers_at_once_and_keeps_the_armed_tick() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    let tick = armed(&test).await;
    test.advance_time_by(Duration::from_secs(20)).await;
    send(&test, SchedulerMsg::Run { slot: None }).await;
    assert_eq!(log(&test, "a").await, ["tick"]);
    assert_eq!(armed(&test).await.id, tick.id);
    test.advance_time_by(Duration::from_secs(40)).await;
    assert_eq!(log(&test, "a").await, ["tick", "tick"]);
    assert_eq!(held(&test).await.runs, 2);
}

/// Scenario: Run before the start.
#[tokio::test(start_paused = true)]
async fn a_run_before_the_start_delivers_once_and_arms_nothing() {
    let test = node().await;
    created(&test, note(every(60), "now")).await;
    send(&test, SchedulerMsg::Run { slot: None }).await;
    assert_eq!(log(&test, "a").await, ["now"]);
    let state = held(&test).await;
    assert_eq!((state.armed_tick, state.runs, state.last_run), (None, 1, Some(test.now())));
    test.advance_time_by(3 * MINUTE).await;
    assert_eq!(log(&test, "a").await, ["now"]);
}

// An operator edits a schedule.

fn update() -> SchedulerMsg {
    SchedulerMsg::Update { schedule: None, target: None, message: None, retry: None }
}

/// Scenario: A new schedule.
#[tokio::test(start_paused = true)]
async fn a_new_schedule_cancels_the_old_tick_and_arms_the_next_slot() {
    let test = node_at("2026-01-01T01:58:30Z").await;
    started(&test, note(daily(2, 0), "tick")).await;
    let old = armed(&test).await;
    send(&test, SchedulerMsg::Update { schedule: Some(every(60)), target: None, message: None, retry: None }).await;
    assert_eq!(status(&test, &old).await, Some(OutcomeStatus::Cancelled));
    assert_eq!(held(&test).await.next_run, Some(ts("2026-01-01T01:59:00Z")));
    test.advance_time_by(2 * MINUTE).await;
    assert_eq!(log(&test, "a").await.len(), 2, "01:59 and 02:00, and none more from the old tick");
}

/// Scenario: A new message.
#[tokio::test(start_paused = true)]
async fn a_new_message_is_sent_from_the_next_slot_and_the_runs_count_on() {
    let test = node().await;
    started(&test, note(every(60), "old")).await;
    test.advance_time_by(MINUTE).await;
    let message = StoredMessage::for_ref(&worker("a"), WorkerMsg::Note { text: "new".into() }).unwrap();
    send(&test, SchedulerMsg::Update { schedule: None, target: None, message: Some(message), retry: None }).await;
    test.advance_time_by(MINUTE).await;
    assert_eq!(log(&test, "a").await, ["old", "new"]);
    assert_eq!(held(&test).await.runs, 2);
}

/// Scenario: A new target.
#[tokio::test(start_paused = true)]
async fn a_new_target_gets_the_message_from_the_next_slot() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    send(&test, SchedulerMsg::Update { schedule: None, target: Some(addr(&worker("b"))), message: None, retry: None })
        .await;
    test.advance_time_by(MINUTE).await;
    assert_eq!(log(&test, "b").await, ["tick"]);
    assert!(log(&test, "a").await.is_empty());
}

/// Scenario: A new retry policy.
#[tokio::test(start_paused = true)]
async fn a_new_retry_policy_retries_the_run_of_the_target() {
    let test = node().await;
    started(&test, SchedulerState::new(every(60), &worker("a"), WorkerMsg::FailOnce { text: "again".into() }).unwrap())
        .await;
    send(&test, SchedulerMsg::Update { schedule: None, target: None, message: None, retry: Some(two_attempts()) })
        .await;
    test.advance_time_by(MINUTE + Duration::from_secs(10)).await;
    assert_eq!(log(&test, "a").await, ["again"]);
}

/// Scenario: An empty update.
#[tokio::test(start_paused = true)]
async fn an_empty_update_changes_nothing() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    let (before, tick) = (held(&test).await, armed(&test).await);
    let m = send(&test, update()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(held(&test).await, before);
    assert_eq!(armed(&test).await.id, tick.id);
}

/// Scenario: A waiting run sees an update.
#[tokio::test(start_paused = true)]
async fn a_waiting_run_sends_the_message_of_an_update_ahead_of_it() {
    let test = node().await;
    created(&test, note(every(60), "old")).await;
    let message = StoredMessage::for_ref(&worker("a"), WorkerMsg::Note { text: "new".into() }).unwrap();
    test.send(&nightly(), SchedulerMsg::Update { schedule: None, target: None, message: Some(message), retry: None })
        .await;
    test.send(&nightly(), SchedulerMsg::Run { slot: None }).await;
    test.run_until_idle().await;
    assert_eq!(log(&test, "a").await, ["new"]);
}

/// Scenario: A new schedule before the start.
#[tokio::test(start_paused = true)]
async fn a_new_schedule_before_the_start_is_kept_and_delivers_nothing() {
    let test = node().await;
    created(&test, note(daily(2, 0), "tick")).await;
    send(&test, SchedulerMsg::Update { schedule: Some(every(60)), target: None, message: None, retry: None }).await;
    let state = held(&test).await;
    assert_eq!((state.schedule, state.armed_tick), (every(60), None));
    test.advance_time_by(3 * MINUTE).await;
    assert!(log(&test, "a").await.is_empty());
}

// The ticks of a scheduler are its own.

/// Scenario: A tick sent by a client.
#[tokio::test(start_paused = true)]
async fn a_tick_sent_by_a_client_ends_done_and_changes_nothing() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    let (before, tick) = (held(&test).await, armed(&test).await);
    let m = send(&test, SchedulerMsg::Tick { slot: ts("2026-01-01T00:01:00Z") }).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(log(&test, "a").await.is_empty());
    assert_eq!(held(&test).await, before);
    assert_eq!(armed(&test).await.id, tick.id);
}

// A failed delivery does not end the chain.

/// Scenario: A target that is gone.
#[tokio::test(start_paused = true)]
async fn a_target_that_is_gone_fails_each_run_with_the_cause_commit_and_the_chain_goes_on() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    test.system().delete_actor(&addr(&worker("a"))).await.unwrap();
    test.advance_time_by(2 * MINUTE).await;
    let failed = failed_runs(&test).await;
    assert_eq!(failed.iter().map(|f| (f.cause, f.attempts)).collect::<Vec<_>>(), [(FailureCause::Commit, 1); 2]);
    assert_eq!(armed(&test).await.deliver_at, ts("2026-01-01T00:03:00Z"));
    assert_eq!(held(&test).await.runs, 0);
}

/// Scenario: The target is back.
#[tokio::test(start_paused = true)]
async fn a_target_created_again_gets_the_next_slot() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    test.system().delete_actor(&addr(&worker("a"))).await.unwrap();
    test.advance_time_by(MINUTE).await;
    test.create(&worker("a"), &WorkerState::default()).await;
    test.advance_time_by(MINUTE).await;
    assert_eq!(log(&test, "a").await, ["tick"]);
}

/// Scenario: A message the target does not take.
#[tokio::test(start_paused = true)]
async fn a_message_the_target_does_not_take_is_refused_on_each_slot_and_the_chain_goes_on() {
    let test = node().await;
    let mut state = note(every(60), "tick");
    state.message = StoredMessage::of(&Shout::Shout { text: "hey".into() }).unwrap();
    started(&test, state).await;
    test.advance_time_by(2 * MINUTE).await;
    let failed = failed_runs(&test).await;
    assert_eq!(failed.iter().map(|f| (f.cause, f.attempts)).collect::<Vec<_>>(), [(FailureCause::Refused, 1); 2]);
    assert!(failed[0].error.contains("Shout"), "{}", failed[0].error);
    armed(&test).await;
}

/// Scenario: A failed run is retried.
#[tokio::test(start_paused = true)]
async fn a_failed_run_that_an_operator_retries_delivers() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    test.system().delete_actor(&addr(&worker("a"))).await.unwrap();
    test.advance_time_by(MINUTE).await;
    let failed = failed_runs(&test).await.pop().expect("a failed run");
    test.create(&worker("a"), &WorkerState::default()).await;
    let RetryResult::Accepted(_) = test.system().retry_message(&failed.message_id).await.unwrap() else {
        panic!("not retried")
    };
    test.run_until_idle().await;
    assert_eq!(log(&test, "a").await, ["tick"]);
}

/// A failed run that the dead letters redrive delivers.
#[tokio::test(start_paused = true)]
async fn a_failed_run_that_the_dead_letters_redrive_delivers() {
    let test = node().await;
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    let watched = WatchTarget::parse("cicada.scheduler/nightly").unwrap();
    test.system().watch(&watched, &addr(&DeadLetters::default())).await.unwrap();
    started(&test, note(every(60), "tick")).await;
    test.system().delete_actor(&addr(&worker("a"))).await.unwrap();
    test.advance_time_by(MINUTE).await;
    let failed = failed_runs(&test).await.pop().expect("a failed run");
    test.create(&worker("a"), &WorkerState::default()).await;
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: failed.message_id }).await;
    test.run_until_idle().await;
    assert_eq!(log(&test, "a").await, ["tick"]);
}

// A start or a new schedule arms a chain again.

/// A node with a claim limit of 2 whose started scheduler `every(60)` lost its armed tick to the claim limit; returns
/// the tick.
async fn tick_failed_for_good() -> (TestActors, Message) {
    let config = NodeConfig { max_lost_claims: Some(2), ..NodeConfig::default() };
    let test = node_with("2026-01-01T00:00:00Z", Some(config)).await;
    started(&test, note(every(60), "tick")).await;
    let tick = armed(&test).await;
    tokio::time::sleep(MINUTE).await;
    for _ in 0..2 {
        let lease = Duration::from_secs(30);
        assert_eq!(claim_and_abandon(test.store(), &types::<Scheduler>(), lease, 1).await.unwrap().len(), 1);
        tokio::time::sleep(lease + Duration::from_secs(1)).await;
    }
    test.run_until_idle().await;
    let failed = told(&test).await.into_iter().find(|f| f.message_id == tick.id).expect("the tick failed");
    assert_eq!(failed.cause, FailureCause::ClaimLimit);
    assert!(test.pending(&nightly()).await.is_empty());
    assert!(log(&test, "a").await.is_empty());
    (test, tick)
}

/// Scenario: A start after a tick that failed for good.
#[tokio::test(start_paused = true)]
async fn a_start_after_a_tick_that_failed_for_good_arms_the_chain_again() {
    let (test, _) = tick_failed_for_good().await;
    send(&test, SchedulerMsg::Start).await;
    armed(&test).await;
    test.advance_time_by(MINUTE).await;
    assert_eq!(log(&test, "a").await, ["tick"]);
}

/// Scenario: An update after a tick that failed for good.
#[tokio::test(start_paused = true)]
async fn an_update_with_a_schedule_after_a_tick_that_failed_for_good_arms_the_new_schedule() {
    let (test, _) = tick_failed_for_good().await;
    send(&test, SchedulerMsg::Update { schedule: Some(every(600)), target: None, message: None, retry: None }).await;
    assert_eq!(armed(&test).await.deliver_at, ts("2026-01-01T00:10:00Z"));
    assert_eq!(held(&test).await.next_run, Some(ts("2026-01-01T00:10:00Z")));
}

/// Scenario: A purge.
#[tokio::test(start_paused = true)]
async fn a_purge_ends_the_chain_until_a_start() {
    let test = node().await;
    started(&test, note(every(60), "tick")).await;
    test.system().purge_mailbox(&addr(&nightly())).await.unwrap();
    test.advance_time_by(2 * MINUTE).await;
    assert!(log(&test, "a").await.is_empty());
    assert!(held(&test).await.armed_tick.is_some(), "it still reads as started");
    send(&test, SchedulerMsg::Start).await;
    test.advance_time_by(MINUTE).await;
    assert_eq!(log(&test, "a").await, ["tick"]);
}

/// Scenario: A retried tick.
#[tokio::test(start_paused = true)]
async fn a_retried_tick_ends_done_and_arms_nothing() {
    let (test, tick) = tick_failed_for_good().await;
    let RetryResult::Accepted(retried) = test.system().retry_message(&tick.id).await.unwrap() else {
        panic!("not retried")
    };
    test.run_until_idle().await;
    assert_eq!(status(&test, &retried).await, Some(OutcomeStatus::Done));
    assert!(test.pending(&nightly()).await.is_empty(), "no tick is armed");
    test.advance_time_by(3 * MINUTE).await;
    assert!(log(&test, "a").await.is_empty());
}

// How deep.

fn deep(levels: usize) -> StoredMessage {
    StoredMessage { message_type: "Note".into(), payload: nested(levels) }
}

/// A payload of 62 levels is a state of 64: the create of the scheduler takes it, and refuses one of 63.
#[tokio::test(start_paused = true)]
async fn the_create_takes_a_payload_of_62_levels_and_refuses_63() {
    let test = node().await;
    let mut state = note(every(60), "x");
    state.message = deep(62);
    test.system().create(&nightly(), &state).await.unwrap();
    state.message = deep(63);
    let refused = test.system().create(&Scheduler::named("deeper"), &state).await;
    assert!(matches!(&refused, Err(OpError::Invalid(why)) if why.contains("64")), "{refused:?}");
}

/// An `Update` holds its message as deep as the state does.
#[tokio::test(start_paused = true)]
async fn an_update_takes_a_payload_of_62_levels_and_refuses_63() {
    let test = node().await;
    created(&test, note(every(60), "x")).await;
    let ok = test
        .system()
        .send(&nightly(), SchedulerMsg::Update { schedule: None, target: None, message: Some(deep(62)), retry: None })
        .await;
    assert!(matches!(ok, Ok(cicada::SendResult::Accepted(_))), "{ok:?}");
    let refused = test
        .system()
        .send(&nightly(), SchedulerMsg::Update { schedule: None, target: None, message: Some(deep(63)), retry: None })
        .await;
    assert!(matches!(&refused, Err(OpError::Invalid(why)) if why.contains("64")), "{refused:?}");
}
