//! The debouncer, on virtual time with `TestActors`: last-wins replacement, the quiet period and the longest wait,
//! equal due times, public and empty flushes, failed flushes that clear nothing, recovery, redrive into a later burst,
//! lost timers, payload depth, creation and the policies of the target and the timer.

mod common;

use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::RetryResult;
use cicada::store::{Message, OutcomeStatus};
use cicada::time::{self, MAX_INSTANT, Timestamp};
use cicada::{Failed, FailureCause, WatchTarget};
use cicada_actors::{
    DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, DebouncedMessage, Debouncer, DebouncerActor,
    DebouncerMsg, DebouncerState, StoredMessage,
};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

const SECOND: Duration = Duration::from_secs(1);

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn deb() -> Debouncer {
    Debouncer::named("d")
}

/// Quiet for `quiet` seconds, at most `max_wait` seconds.
fn settings(quiet: u64, max_wait: u64) -> DebouncerState {
    DebouncerState::new(Duration::from_secs(quiet), Duration::from_secs(max_wait)).unwrap()
}

fn note(id: &str, text: &str) -> DebouncerMsg {
    DebouncerMsg::submit(&worker(id), WorkerMsg::Note { text: text.into() }, None).unwrap()
}

fn two_attempts() -> RetryPolicy {
    RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(10)).unwrap()).unwrap()
}

/// The crate's shared retry policy, which the timer carries.
fn shared_policy() -> RetryPolicy {
    let backoff = Backoff::exponential(SECOND, 2.0, Duration::from_secs(32)).unwrap();
    RetryPolicy::new(7, backoff).unwrap()
}

/// A message of a family that the worker does not take.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Shout {
    Shout { text: String },
}

/// `producer/_`: creates the debouncer `d` with the settings it is given and submits to it, in one turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Producer;

impl ActorRef for Producer {
    const TYPE: &'static str = "producer";
    type Message = ProducerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ProducerMsg {
    Go { settings: DebouncerState, text: String },
}

struct ProducerActor;

impl Actor for ProducerActor {
    type Ref = Producer;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProducerMsg) -> HandlerResult {
        let ProducerMsg::Go { settings, text } = message;
        turn.create(&deb(), &settings)?;
        turn.send(&deb(), note("a", &text))?;
        Ok(())
    }
}

/// A node at `start` with the debouncer type, the workers `worker/a` and `worker/b` (`worker/a` paused, so that what
/// the debouncer forwards waits in its mailbox), the producer, the dead letters (not created) and the witness, which
/// watches the debouncers.
async fn node_at(start: &str) -> TestActors {
    let test = TestActors::builder()
        .start_at(ts(start))
        .with(|node| {
            node.host(DebouncerActor)
                .host(WorkerActor)
                .host(WitnessActor)
                .host(ProducerActor)
                .host(DeadLettersActor::default())
        })
        .build()
        .await
        .unwrap();
    for id in ["a", "b"] {
        test.create(&worker(id), &WorkerState::default()).await;
    }
    test.system().pause_mailbox(&addr(&worker("a"))).await.unwrap();
    test.create(&Producer, &NoState {}).await;
    witness(&test, "cicada.debouncer").await;
    test
}

async fn node() -> TestActors {
    node_at("2026-01-01T00:00:00Z").await
}

/// Sends `d` `message` from a client and runs what is due.
async fn submit(test: &TestActors, message: DebouncerMsg) -> Message {
    let m = test.send(&deb(), message).await;
    test.run_until_idle().await;
    m
}

async fn held(test: &TestActors) -> DebouncerState {
    test.state(&deb()).await
}

/// What waits for `worker/a`: the text of each message and its due time as an offset from `start`.
async fn forwarded(test: &TestActors, start: Timestamp) -> Vec<(String, Duration)> {
    test.pending(&worker("a"))
        .await
        .iter()
        .map(|m| (m.payload["text"].as_str().unwrap_or_default().to_owned(), time::between(start, m.deliver_at)))
        .collect()
}

/// The flushes waiting in the mailbox of `d`, as offsets from `start`; asserts that the last is the armed one.
async fn flushes(test: &TestActors, start: Timestamp) -> Vec<Duration> {
    let pending: Vec<Message> = test.pending(&deb()).await.into_iter().filter(|m| m.message_type == "Flush").collect();
    if let Some(last) = pending.last() {
        assert_eq!(held(test).await.armed.as_ref(), Some(&last.id), "the armed flush");
    }
    pending.iter().map(|m| time::between(start, m.deliver_at)).collect()
}

async fn log(test: &TestActors, id: &str) -> Vec<String> {
    test.state::<_, WorkerState>(&worker(id)).await.log
}

/// The `Failed` the witness was told of the debouncer.
async fn failed(test: &TestActors) -> Vec<Failed> {
    told(test).await.into_iter().filter(|f| f.actor == addr(&deb())).collect()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// A debouncer has fixed valid settings.

/// Scenario: Explicit creation.
#[tokio::test(start_paused = true)]
async fn a_producer_creates_a_debouncer_and_submits_in_one_turn_and_a_repeated_create_changes_nothing() {
    let test = node().await;
    let start = test.now();
    test.send(&Producer, ProducerMsg::Go { settings: settings(2, 5), text: "x".into() }).await;
    test.run_until_idle().await;
    let state = held(&test).await;
    assert_eq!(state.pending.as_ref().map(|p| p.message.payload.clone()), Some(json!({"text": "x"})));
    assert_eq!(flushes(&test, start).await, [secs(2)]);
    test.send(&Producer, ProducerMsg::Go { settings: settings(1, 1), text: "y".into() }).await;
    test.run_until_idle().await;
    let again = held(&test).await;
    assert_eq!((again.quiet, again.max_wait, again.first_at), (secs(2), secs(5), Some(start)), "the settings stay");
    assert_eq!(again.pending.unwrap().message.payload, json!({"text": "y"}), "the second submission replaced it");
    assert!(failed(&test).await.is_empty());
}

/// A debouncer created from the minimal JSON keeps nothing.
#[tokio::test(start_paused = true)]
async fn a_debouncer_created_from_the_minimal_json_is_empty() {
    let test = node().await;
    test.create(&deb(), &json!({"quiet": "PT2S", "maxWait": "PT5S"})).await;
    assert_eq!(held(&test).await, settings(2, 5));
}

// The entire latest submission replaces the previous one.

/// Scenario: A new target and policy.
#[tokio::test(start_paused = true)]
async fn the_latest_submission_replaces_target_message_retry_and_correlation_id() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    let first = test.send_with(&deb(), note("a", "1"), SendOptions::new().correlation_id("c1")).await;
    test.run_until_idle().await;
    let second =
        DebouncerMsg::submit(&worker("b"), WorkerMsg::Note { text: "2".into() }, Some(two_attempts())).unwrap();
    test.send_with(&deb(), second, SendOptions::new().correlation_id("c2")).await;
    test.run_until_idle().await;
    let pending = held(&test).await.pending.unwrap();
    let expected = DebouncedMessage {
        to: addr(&worker("b")),
        message: StoredMessage::for_ref(&worker("b"), WorkerMsg::Note { text: "2".into() }).unwrap(),
        retry: Some(two_attempts()),
        correlation_id: Some("c2".into()),
    };
    assert_eq!(pending, expected);
    assert_eq!(status(&test, &first).await, Some(OutcomeStatus::Done), "the superseded submission is done");
    assert_eq!(flushes(&test, start).await, [secs(2)], "one armed flush");
    test.advance_time_by(secs(2)).await;
    assert_eq!(log(&test, "b").await, ["2"]);
    assert!(forwarded(&test, start).await.is_empty(), "A gets nothing from that burst");
    assert!(failed(&test).await.is_empty(), "nobody is told of the superseded submission");
}

/// Scenario: A target not yet present.
#[tokio::test(start_paused = true)]
async fn a_submission_for_an_absent_instance_or_an_unregistered_type_is_kept() {
    let test = node().await;
    test.create(&deb(), &settings(2, 5)).await;
    let note = StoredMessage::for_ref(&worker("a"), WorkerMsg::Note { text: "x".into() }).unwrap();
    for to in ["worker/absent", "nobody/x"] {
        let to = ActorAddress::parse(to).unwrap();
        let m = submit(&test, DebouncerMsg::Submit { to: to.clone(), message: note.clone(), retry: None }).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
        assert_eq!(held(&test).await.pending.unwrap().to, to);
    }
    assert!(failed(&test).await.is_empty());
}

// Submit replaces the timer but keeps the burst's start.

/// Scenario: Quiet delivery.
#[tokio::test(start_paused = true)]
async fn one_submission_is_delivered_after_the_quiet_period_and_not_before() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("a", "x")).await;
    assert_eq!(flushes(&test, start).await, [secs(2)]);
    test.advance_time_by(SECOND + Duration::from_millis(999)).await;
    assert!(forwarded(&test, start).await.is_empty());
    test.advance_time_by(Duration::from_millis(1)).await;
    assert_eq!(forwarded(&test, start).await, [("x".to_owned(), secs(2))]);
    assert_eq!(held(&test).await, settings(2, 5), "cleared");
}

/// Scenario: Continuous submissions reach the cap.
#[tokio::test(start_paused = true)]
async fn submissions_every_second_deliver_only_the_last_at_the_longest_wait() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    for n in 0..5 {
        submit(&test, note("a", &n.to_string())).await;
        assert_eq!(held(&test).await.first_at, Some(start), "the burst keeps its start");
        test.advance_time_by(SECOND).await;
    }
    assert_eq!(forwarded(&test, start).await, [("4".to_owned(), secs(5))]);
    assert_eq!(held(&test).await, settings(2, 5));
}

/// Scenario: A new burst.
#[tokio::test(start_paused = true)]
async fn a_submission_after_a_flush_starts_a_new_burst() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("a", "1")).await;
    test.advance_time_by(secs(10)).await;
    submit(&test, note("a", "2")).await;
    assert_eq!(held(&test).await.first_at, time::add(start, secs(10)).ok());
    assert_eq!(flushes(&test, start).await, [secs(12)]);
}

/// Scenario: Submit and timer have the same due time; the Submit comes first in the core's order.
#[tokio::test(start_paused = true)]
async fn a_submit_due_with_the_timer_and_ordered_first_replaces_the_burst() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    test.send(&deb(), note("a", "1")).await;
    // Sent before the first Submit is handled: its sequence is lower than that of the flush the first one arms.
    test.send_with(&deb(), note("a", "2"), SendOptions::new().delay(secs(2))).await;
    test.run_until_idle().await;
    test.advance_time_by(secs(2)).await;
    assert!(forwarded(&test, start).await.is_empty(), "the first value was replaced, its flush cancelled");
    let state = held(&test).await;
    assert_eq!((state.first_at, state.pending.unwrap().message.payload), (Some(start), json!({"text": "2"})));
    assert_eq!(flushes(&test, start).await, [secs(4)]);
    test.advance_time_by(secs(2)).await;
    assert_eq!(forwarded(&test, start).await, [("2".to_owned(), secs(4))]);
}

/// Scenario: Submit and timer have the same due time; the flush comes first in the core's order.
#[tokio::test(start_paused = true)]
async fn a_submit_due_with_the_timer_and_ordered_after_it_starts_the_next_burst() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("a", "1")).await;
    test.send_with(&deb(), note("a", "2"), SendOptions::new().delay(secs(2))).await;
    test.advance_time_by(secs(2)).await;
    assert_eq!(forwarded(&test, start).await, [("1".to_owned(), secs(2))], "the old value was delivered");
    assert_eq!(held(&test).await.first_at, time::add(start, secs(2)).ok(), "a new burst");
    assert_eq!(flushes(&test, start).await, [secs(4)]);
}

/// Scenario: Timestamp overflow.
#[tokio::test(start_paused = true)]
async fn a_submit_whose_flush_would_be_due_past_max_instant_is_refused_and_changes_nothing() {
    let test = node_at("9999-12-31T23:59:54.500Z").await;
    test.create(&deb(), &settings(5, 9)).await;
    submit(&test, note("a", "1")).await;
    let before = held(&test).await;
    assert_eq!(flushes(&test, test.now()).await.len(), 1, "due at {MAX_INSTANT} at the latest");
    test.advance_time_by(Duration::from_millis(500)).await;
    let m = submit(&test, note("a", "2")).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    let refused = failed(&test).await;
    assert_eq!(refused.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused]);
    assert!(refused[0].error.contains("after 9999-12-31"), "{}", refused[0].error);
    assert_eq!(held(&test).await, before);
    let pending = test.pending(&deb()).await;
    assert_eq!(pending.iter().map(|m| Some(&m.id)).collect::<Vec<_>>(), [before.armed.as_ref()], "the old timer");
}

// Public Flush delivers and clears atomically.

/// Scenario: A manual early Flush.
#[tokio::test(start_paused = true)]
async fn a_manual_flush_before_the_timer_delivers_now_and_cancels_the_timer() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("a", "x")).await;
    let timer = test.pending(&deb()).await.pop().unwrap();
    test.advance_time_by(SECOND).await;
    submit(&test, DebouncerMsg::Flush).await;
    assert_eq!(forwarded(&test, start).await, [("x".to_owned(), SECOND)]);
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Cancelled));
    assert_eq!(held(&test).await, settings(2, 5));
    assert!(test.pending(&deb()).await.is_empty());
}

/// Scenario: The timer flushes itself.
#[tokio::test(start_paused = true)]
async fn the_armed_flush_delivers_and_clears_without_cancelling_itself() {
    let test = node().await;
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("b", "x")).await;
    let timer = test.pending(&deb()).await.pop().unwrap();
    assert_eq!(held(&test).await.armed, Some(timer.id.clone()));
    test.advance_time_by(secs(2)).await;
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Done));
    assert_eq!(log(&test, "b").await, ["x"]);
    assert_eq!(held(&test).await, settings(2, 5));
}

/// Scenario: Empty Flush.
#[tokio::test(start_paused = true)]
async fn an_empty_flush_is_done_sends_nothing_and_later_submissions_work() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    let m = submit(&test, DebouncerMsg::Flush).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(held(&test).await, settings(2, 5));
    assert!(test.pending(&deb()).await.is_empty() && forwarded(&test, start).await.is_empty());
    submit(&test, note("a", "x")).await;
    test.advance_time_by(secs(2)).await;
    assert_eq!(forwarded(&test, start).await, [("x".to_owned(), secs(2))]);
    assert!(test.exists(&deb()).await, "it never stops itself");
}

// Forwarding retains the latest submission's selected context.

/// Scenario: Distinct target and timer policies.
#[tokio::test(start_paused = true)]
async fn the_target_gets_the_kept_policy_and_correlation_id_and_the_timer_the_shared_policy() {
    let test = node().await;
    test.create(&deb(), &settings(2, 5)).await;
    let submission =
        DebouncerMsg::submit(&worker("a"), WorkerMsg::Note { text: "x".into() }, Some(two_attempts())).unwrap();
    let options = SendOptions::new().correlation_id("c").header("h", "v");
    let m = test.send_with(&deb(), submission, options).await;
    test.run_until_idle().await;
    let timer = test.pending(&deb()).await.pop().unwrap();
    assert_eq!(timer.retry, Some(shared_policy()));
    test.send_with(&deb(), DebouncerMsg::Flush, SendOptions::new().correlation_id("manual")).await;
    test.run_until_idle().await;
    let pending = test.pending(&worker("a")).await;
    assert_eq!(pending.len(), 1);
    let target = &pending[0];
    assert_eq!((target.message_type.as_str(), &target.payload), ("Note", &json!({"text": "x"})));
    assert_eq!(target.correlation_id.as_deref(), Some("c"));
    assert_eq!(target.from, Some(addr(&deb())));
    assert_ne!(target.id, m.id);
    assert_eq!(target.retry, Some(two_attempts()));
    assert_eq!(target.retention, None);
    assert!(!target.headers.contains_key("h"), "{:?}", target.headers);
}

/// A submission without a target retry forwards with none; a retry at the target does not pass the debouncer again.
#[tokio::test(start_paused = true)]
async fn a_retry_at_the_target_bypasses_the_debouncer() {
    let test = node().await;
    test.create(&deb(), &settings(2, 5)).await;
    let submission =
        DebouncerMsg::submit(&worker("b"), WorkerMsg::FailOnce { text: "again".into() }, Some(two_attempts())).unwrap();
    submit(&test, submission).await;
    test.advance_time_by(secs(2)).await;
    assert!(log(&test, "b").await.is_empty(), "the first run failed");
    assert_eq!(held(&test).await, settings(2, 5));
    test.advance_time_by(secs(10)).await;
    assert_eq!(log(&test, "b").await, ["again"]);
    assert!(test.pending(&deb()).await.is_empty());
}

// A failed Flush leaves pending data available for recovery.

/// Scenario: Absent target and recovery.
#[tokio::test(start_paused = true)]
async fn a_flush_to_an_absent_target_fails_with_commit_keeps_the_value_and_a_retry_delivers_it_once() {
    let test = node().await;
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("c", "x")).await;
    let before = held(&test).await;
    test.advance_time_by(secs(2)).await;
    let failure = failed(&test).await.pop().expect("a failed flush");
    assert_eq!((failure.cause, failure.message_type.as_str()), (FailureCause::Commit, "Flush"));
    assert_eq!(held(&test).await, before, "nothing cleared");
    test.create(&worker("c"), &WorkerState::default()).await;
    let RetryResult::Accepted(_) = test.system().retry_message(&failure.message_id).await.unwrap() else {
        panic!("not retried")
    };
    test.run_until_idle().await;
    assert_eq!(log(&test, "c").await, ["x"]);
    assert_eq!(held(&test).await, settings(2, 5));
}

/// Scenario: Rejected message type.
#[tokio::test(start_paused = true)]
async fn a_flush_to_an_unregistered_type_or_of_a_message_the_target_does_not_take_is_refused_and_keeps_the_value() {
    let test = node().await;
    test.create(&deb(), &settings(2, 5)).await;
    let nobody = ActorAddress::parse("nobody/x").unwrap();
    let shout = StoredMessage::of(&Shout::Shout { text: "hey".into() }).unwrap();
    let note = StoredMessage::for_ref(&worker("a"), WorkerMsg::Note { text: "x".into() }).unwrap();
    for (to, message) in [(nobody, note), (addr(&worker("a")), shout)] {
        submit(&test, DebouncerMsg::Submit { to, message, retry: None }).await;
        let before = held(&test).await;
        submit(&test, DebouncerMsg::Flush).await;
        assert_eq!(held(&test).await, before, "the value stays");
    }
    let refused = failed(&test).await;
    assert_eq!(refused.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused; 2]);
    assert!(refused[1].error.contains("Shout"), "{}", refused[1].error);
    assert!(test.pending(&worker("a")).await.is_empty());
}

/// Scenario: Target failure after forwarding.
#[tokio::test(start_paused = true)]
async fn a_payload_the_target_cannot_decode_fails_there_and_the_debouncer_stays_cleared() {
    let test = node().await;
    test.create(&deb(), &settings(2, 5)).await;
    let bad = StoredMessage { message_type: "Note".into(), payload: json!({"text": 7}) };
    submit(&test, DebouncerMsg::Submit { to: addr(&worker("b")), message: bad, retry: None }).await;
    test.advance_time_by(secs(2)).await;
    let failures = test.failures().await;
    assert!(failures.iter().any(|f| f.envelope.message_type == "Note"), "{failures:?}");
    assert!(failed(&test).await.is_empty(), "the flush itself is done");
    assert_eq!(held(&test).await, settings(2, 5));
}

// Timer loss and redrive act on current state.

/// Scenario: Rearming after the cap: the timer was cancelled or purged.
#[tokio::test(start_paused = true)]
async fn a_lost_timer_keeps_the_value_and_a_later_submit_past_the_cap_flushes_at_once() {
    for purge in [false, true] {
        let test = node().await;
        let start = test.now();
        test.create(&deb(), &settings(2, 5)).await;
        submit(&test, note("a", "1")).await;
        let timer = test.pending(&deb()).await.pop().unwrap();
        if purge {
            test.system().purge_mailbox(&addr(&deb())).await.unwrap();
        } else {
            test.system().cancel(&timer.id).await.unwrap();
        }
        test.advance_time_by(secs(10)).await;
        assert!(forwarded(&test, start).await.is_empty());
        let state = held(&test).await;
        assert_eq!((state.first_at, state.armed), (Some(start), Some(timer.id.clone())), "kept, purge {purge}");
        assert!(failed(&test).await.is_empty(), "nobody is told of a cancel or a purge");
        submit(&test, note("a", "2")).await;
        assert_eq!(forwarded(&test, start).await, [("2".to_owned(), secs(10))], "due at once, purge {purge}");
        assert_eq!(held(&test).await, settings(2, 5));
    }
}

/// Scenario: Rearming after the cap: the timer failed for good.
#[tokio::test(start_paused = true)]
async fn a_submit_after_a_failed_flush_replaces_the_value_and_rearms_from_the_first_submission() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("c", "1")).await;
    test.advance_time_by(secs(2)).await;
    assert_eq!(failed(&test).await.len(), 1, "the flush to the absent worker failed");
    submit(&test, note("a", "2")).await;
    assert_eq!(held(&test).await.first_at, Some(start), "the burst keeps its start");
    assert_eq!(flushes(&test, start).await, [secs(4)], "capped at 5 s would be later: the quiet period wins");
    test.advance_time_by(secs(10)).await;
    assert_eq!(forwarded(&test, start).await, [("2".to_owned(), secs(4))]);
}

/// Scenario: Redrive into a later burst.
#[tokio::test(start_paused = true)]
async fn an_old_failed_flush_redriven_flushes_the_new_burst_now() {
    let test = node().await;
    let start = test.now();
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    let watched = WatchTarget::parse("cicada.debouncer/d").unwrap();
    test.system().watch(&watched, &addr(&DeadLetters::default())).await.unwrap();
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("c", "old")).await;
    test.advance_time_by(secs(2)).await;
    let old = failed(&test).await.pop().expect("the failed flush");
    test.advance_time_by(secs(10)).await;
    submit(&test, note("a", "new")).await;
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: old.message_id }).await;
    test.run_until_idle().await;
    assert_eq!(forwarded(&test, start).await, [("new".to_owned(), secs(12))], "the new burst, early");
    assert_eq!(held(&test).await, settings(2, 5));
    assert!(test.pending(&deb()).await.is_empty(), "the new burst's timer was cancelled");
}

// Deep pending values fail without losing the earlier burst.

/// Scenario: Payload depth boundary.
#[tokio::test(start_paused = true)]
async fn a_payload_of_61_levels_is_kept_and_flushed_and_one_of_62_fails_and_keeps_the_earlier_burst() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    let deep = |levels: usize| {
        let payload = (0..levels).fold(json!(1), |inner, _| serde_json::Value::Array(vec![inner]));
        DebouncerMsg::Submit {
            to: addr(&worker("a")),
            message: StoredMessage { message_type: "Note".into(), payload },
            retry: None,
        }
    };
    submit(&test, deep(61)).await;
    submit(&test, DebouncerMsg::Flush).await;
    assert_eq!(test.pending(&worker("a")).await.len(), 1, "61 levels were kept and flushed");
    submit(&test, note("a", "kept")).await;
    let before = held(&test).await;
    let m = submit(&test, deep(62)).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    let error = test.outcome(&m).await.unwrap().error.unwrap_or_default();
    assert!(error.contains("64"), "the state check: {error}");
    // The `Failed` holds the payload a level deeper than the Submit did, too deep for the witness to read: the
    // witness's own turn fails, and its envelope tells the cause.
    let told: Vec<_> = test
        .failures()
        .await
        .into_iter()
        .filter(|f| f.envelope.message_type == "Failed" && f.envelope.payload["messageId"] == json!(m.id))
        .map(|f| f.envelope.payload["cause"].clone())
        .collect();
    assert_eq!(told, [json!("handler")]);
    assert_eq!(held(&test).await, before);
    assert_eq!(flushes(&test, start).await, [secs(2)], "the earlier timer stays");
}

// Documentation and samples state the deadlines' limits.

/// Scenario: Paused past the deadline.
#[tokio::test(start_paused = true)]
async fn a_paused_debouncer_flushes_when_it_resumes() {
    let test = node().await;
    let start = test.now();
    test.create(&deb(), &settings(2, 5)).await;
    submit(&test, note("a", "x")).await;
    test.system().pause_mailbox(&addr(&deb())).await.unwrap();
    test.advance_time_by(secs(10)).await;
    assert!(forwarded(&test, start).await.is_empty());
    test.system().resume_mailbox(&addr(&deb())).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(forwarded(&test, start).await, [("x".to_owned(), secs(10))], "flushed when claimed");
}
