//! The rate limiter, on virtual time with `TestActors`: its slots, burst and longest delay, the forwarded envelope,
//! creation, failed forwards that reserve nothing, recovery, dedup, cancels and a replaced limiter.

mod common;

use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::RetryResult;
use cicada::store::{Message, OutcomeStatus};
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp};
use cicada::{Failed, FailureCause, SendResult, WatchTarget};
use cicada_actors::{
    DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, RateLimiter, RateLimiterActor, RateLimiterMsg,
    RateLimiterState, StoredMessage,
};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

const SECOND: Duration = Duration::from_secs(1);

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn api() -> RateLimiter {
    RateLimiter::named("api")
}

/// `count` per second, `burst` at once, refusing a slot more than `max_delay` away.
fn rate(count: u32, burst: u32, max_delay: Duration) -> RateLimiterState {
    RateLimiterState::new(count, SECOND, burst, max_delay).unwrap()
}

fn note(id: &str, text: &str) -> RateLimiterMsg {
    RateLimiterMsg::submit(&worker(id), WorkerMsg::Note { text: text.into() }, None).unwrap()
}

fn two_attempts() -> RetryPolicy {
    RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(10)).unwrap()).unwrap()
}

/// A message of a family that the worker does not take.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Shout {
    Shout { text: String },
}

/// `producer/_`: creates the limiter `api` with the settings it is given and submits to it, in one turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Producer;

impl ActorRef for Producer {
    const TYPE: &'static str = "producer";
    type Message = ProducerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ProducerMsg {
    Go { settings: RateLimiterState, text: String },
}

struct ProducerActor;

impl Actor for ProducerActor {
    type Ref = Producer;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProducerMsg) -> HandlerResult {
        let ProducerMsg::Go { settings, text } = message;
        turn.create(&api(), &settings)?;
        turn.send(&api(), note("a", &text))?;
        Ok(())
    }
}

/// A node at `start` with the limiter type, the workers `worker/a` and `worker/b` (`worker/a` paused, so that what the
/// limiter forwards waits in its mailbox), the producer, the dead letters (not created) and the witness, which watches
/// the limiters.
async fn node_at(start: &str) -> TestActors {
    let test = TestActors::builder()
        .start_at(ts(start))
        .with(|node| {
            node.host(RateLimiterActor)
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
    witness(&test, "cicada.rate-limiter").await;
    test
}

async fn node() -> TestActors {
    node_at("2026-01-01T00:00:00Z").await
}

/// Sends `api` `message` from a client and runs what is due.
async fn submit(test: &TestActors, message: RateLimiterMsg) -> Message {
    let m = test.send(&api(), message).await;
    test.run_until_idle().await;
    m
}

async fn held(test: &TestActors) -> RateLimiterState {
    test.state(&api()).await
}

/// The due times of what waits for `worker/a`, as offsets from `start`.
async fn due(test: &TestActors, start: Timestamp) -> Vec<Duration> {
    test.pending(&worker("a")).await.iter().map(|m| time::between(start, m.deliver_at)).collect()
}

async fn log(test: &TestActors, id: &str) -> Vec<String> {
    test.state::<_, WorkerState>(&worker(id)).await.log
}

/// The `Failed` the witness was told of the submissions.
async fn failed(test: &TestActors) -> Vec<Failed> {
    told(test).await.into_iter().filter(|f| f.message_type == "Submit").collect()
}

// One limiter keeps one rate and one reservation timestamp.

/// Scenario: Minimal initial state.
#[tokio::test(start_paused = true)]
async fn a_limiter_created_from_the_minimal_json_has_its_full_burst() {
    let test = node().await;
    let start = test.now();
    test.create(&api(), &json!({"count": 2, "period": "PT1S", "burst": 2, "maxDelay": "PT1S"})).await;
    assert_eq!(held(&test).await.idle_at, None);
    submit(&test, note("a", "1")).await;
    submit(&test, note("a", "2")).await;
    assert_eq!(due(&test, start).await, [Duration::ZERO, Duration::ZERO]);
}

// Creation uses the existing core API.

/// Scenario: Atomic create and submit.
#[tokio::test(start_paused = true)]
async fn a_producer_creates_a_limiter_and_submits_in_one_turn() {
    let test = node().await;
    let start = test.now();
    test.send(&Producer, ProducerMsg::Go { settings: rate(1, 1, SECOND), text: "x".into() }).await;
    test.run_until_idle().await;
    assert_eq!(due(&test, start).await, [Duration::ZERO]);
    assert_eq!(held(&test).await.idle_at, time::add(start, SECOND).ok());
}

/// Scenario: Repeated create.
#[tokio::test(start_paused = true)]
async fn a_repeated_create_keeps_the_settings_and_the_reservation() {
    let test = node().await;
    let start = test.now();
    test.create(&api(), &rate(1, 1, Duration::from_secs(5))).await;
    submit(&test, note("a", "1")).await;
    let other = RateLimiterState::new(100, SECOND, 100, Duration::ZERO).unwrap();
    test.send(&Producer, ProducerMsg::Go { settings: other, text: "2".into() }).await;
    test.run_until_idle().await;
    assert_eq!(due(&test, start).await, [Duration::ZERO, SECOND], "the slot of the original rate");
    let state = held(&test).await;
    assert_eq!((state.count, state.burst, state.idle_at), (1, 1, time::add(start, 2 * SECOND).ok()));
}

// Timestamp arithmetic cannot overflow.

/// Scenario: Timestamp overflow.
#[tokio::test(start_paused = true)]
async fn a_slot_past_max_instant_is_refused_for_good_and_changes_nothing() {
    let test = node_at("9500-01-01T00:00:00Z").await;
    let mut state = RateLimiterState::new(1, SECOND, 1, MAX_DELAY).unwrap();
    state.idle_at = Some(MAX_INSTANT);
    test.create(&api(), &state).await;
    let m = submit(&test, note("a", "x")).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!(failed(&test).await.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused]);
    assert_eq!(held(&test).await, state);
    assert!(test.pending(&worker("a")).await.is_empty());
}

// Burst and longest delay bound admission.

/// Scenario: Burst and inclusive delay.
#[tokio::test(start_paused = true)]
async fn five_at_once_get_two_now_two_spaced_and_the_fifth_is_refused() {
    let test = node().await;
    let start = test.now();
    test.create(&api(), &rate(2, 2, SECOND)).await;
    for n in 0..5 {
        test.send(&api(), note("a", &n.to_string())).await;
    }
    test.run_until_idle().await;
    assert_eq!(test.now(), start, "all were handled at the same store time");
    assert_eq!(due(&test, start).await, [Duration::ZERO, Duration::ZERO, SECOND / 2, SECOND]);
    let refused = failed(&test).await;
    assert_eq!(refused.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused]);
    assert!(refused[0].error.contains("longest delay"), "{}", refused[0].error);
    assert_eq!(held(&test).await.idle_at, time::add(start, 2 * SECOND).ok(), "the fifth consumed nothing");
}

/// Scenario: Immediate capacity only.
#[tokio::test(start_paused = true)]
async fn a_zero_longest_delay_refuses_the_second_even_with_an_envelope_retry() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, Duration::ZERO)).await;
    submit(&test, note("a", "1")).await;
    let second = test.send_with(&api(), note("a", "2"), SendOptions::new().retry(two_attempts())).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&second).await.expect("an outcome");
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    let refused = failed(&test).await;
    assert_eq!(refused.iter().map(|f| (f.cause, f.attempts)).collect::<Vec<_>>(), [(FailureCause::Refused, 1)]);
    assert_eq!(test.pending(&worker("a")).await.len(), 1);
}

/// Scenario: Idle replenishment.
#[tokio::test(start_paused = true)]
async fn a_second_of_idle_time_gives_the_burst_back() {
    let test = node().await;
    let start = test.now();
    test.create(&api(), &rate(2, 2, Duration::ZERO)).await;
    submit(&test, note("a", "1")).await;
    submit(&test, note("a", "2")).await;
    test.advance_time_by(SECOND).await;
    submit(&test, note("a", "3")).await;
    submit(&test, note("a", "4")).await;
    assert_eq!(due(&test, start).await, [Duration::ZERO, Duration::ZERO, SECOND, SECOND]);
    assert!(failed(&test).await.is_empty());
}

// A submission forwards once with its selected target policy.

/// Scenario: Target envelope.
#[tokio::test(start_paused = true)]
async fn the_forwarded_message_carries_the_correlation_id_the_limiter_and_the_target_policy() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, SECOND)).await;
    let target_policy = two_attempts();
    let own_policy = RetryPolicy::new(5, Backoff::fixed(SECOND).unwrap()).unwrap();
    let submission =
        RateLimiterMsg::submit(&worker("a"), WorkerMsg::Note { text: "x".into() }, Some(target_policy.clone()))
            .unwrap();
    let options = SendOptions::new().correlation_id("c").header("h", "v").retry(own_policy);
    let m = test.send_with(&api(), submission, options).await;
    test.run_until_idle().await;
    let pending = test.pending(&worker("a")).await;
    assert_eq!(pending.len(), 1);
    let forwarded = &pending[0];
    assert_eq!((forwarded.message_type.as_str(), &forwarded.payload), ("Note", &json!({"text": "x"})));
    assert_eq!(forwarded.correlation_id.as_deref(), Some("c"));
    assert_eq!(forwarded.from, Some(addr(&api())));
    assert_ne!(forwarded.id, m.id);
    assert_eq!(forwarded.retry, Some(target_policy));
    assert_eq!(forwarded.retention, None);
    assert!(!forwarded.headers.contains_key("h"), "{:?}", forwarded.headers);
}

// Submit selects the target policy independently.

/// Scenario: No target retry requested.
#[tokio::test(start_paused = true)]
async fn a_submission_without_a_target_retry_forwards_with_none() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, SECOND)).await;
    test.send_with(&api(), note("a", "x"), SendOptions::new().retry(two_attempts())).await;
    test.run_until_idle().await;
    assert_eq!(test.pending(&worker("a")).await[0].retry, None);
}

/// Scenario: Target retries bypass the limiter.
#[tokio::test(start_paused = true)]
async fn a_retry_at_the_target_does_not_pass_the_limiter_again() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, SECOND)).await;
    let submission =
        RateLimiterMsg::submit(&worker("b"), WorkerMsg::FailOnce { text: "again".into() }, Some(two_attempts()))
            .unwrap();
    submit(&test, submission).await;
    let after = held(&test).await;
    assert!(log(&test, "b").await.is_empty(), "the first run failed");
    test.advance_time_by(Duration::from_secs(10)).await;
    assert_eq!(log(&test, "b").await, ["again"]);
    assert_eq!(held(&test).await, after);
}

// A failed forwarding consumes no slot.

/// Scenario: Absent target.
#[tokio::test(start_paused = true)]
async fn a_submission_to_an_absent_target_fails_with_commit_and_leaves_its_slot() {
    let test = node().await;
    let start = test.now();
    test.create(&api(), &rate(1, 1, SECOND)).await;
    let m = submit(&test, note("absent", "x")).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!(failed(&test).await.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Commit]);
    assert_eq!(held(&test).await.idle_at, None);
    submit(&test, note("a", "y")).await;
    assert_eq!(due(&test, start).await, [Duration::ZERO], "the unconsumed slot");
}

/// Scenario: Refused target message type.
#[tokio::test(start_paused = true)]
async fn a_submission_of_an_unregistered_type_or_a_message_it_does_not_take_is_refused() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, SECOND)).await;
    let nobody = ActorAddress::parse("nobody/x").unwrap();
    let shout = StoredMessage::of(&Shout::Shout { text: "hey".into() }).unwrap();
    let note = StoredMessage::for_ref(&worker("a"), WorkerMsg::Note { text: "x".into() }).unwrap();
    submit(&test, RateLimiterMsg::Submit { to: nobody, message: note, retry: None }).await;
    submit(&test, RateLimiterMsg::Submit { to: addr(&worker("a")), message: shout, retry: None }).await;
    let refused = failed(&test).await;
    assert_eq!(refused.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused; 2]);
    assert!(refused[1].error.contains("Shout"), "{}", refused[1].error);
    assert_eq!(held(&test).await.idle_at, None);
    assert!(test.pending(&worker("a")).await.is_empty());
}

/// Scenario: Target rejects the payload.
#[tokio::test(start_paused = true)]
async fn a_payload_the_target_cannot_decode_fails_there_and_keeps_the_reservation() {
    let test = node().await;
    let start = test.now();
    test.create(&api(), &rate(1, 1, SECOND)).await;
    let bad = StoredMessage { message_type: "Note".into(), payload: json!({"text": 7}) };
    submit(&test, RateLimiterMsg::Submit { to: addr(&worker("b")), message: bad, retry: None }).await;
    let failures = test.failures().await;
    assert!(failures.iter().any(|f| f.envelope.message_type == "Note"), "{failures:?}");
    assert!(failed(&test).await.is_empty(), "the submission itself is done");
    assert_eq!(held(&test).await.idle_at, time::add(start, SECOND).ok());
}

// Core recovery and control retain their meaning.

/// Scenario: Operator recovery.
#[tokio::test(start_paused = true)]
async fn a_failed_submission_that_an_operator_retries_forwards_at_a_fresh_slot() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, SECOND)).await;
    submit(&test, note("c", "x")).await;
    let failure = failed(&test).await.pop().expect("a failed submission");
    assert_eq!(failure.cause, FailureCause::Commit);
    test.advance_time_by(Duration::from_secs(30)).await;
    test.create(&worker("c"), &WorkerState::default()).await;
    let RetryResult::Accepted(_) = test.system().retry_message(&failure.message_id).await.unwrap() else {
        panic!("not retried")
    };
    test.run_until_idle().await;
    assert_eq!(log(&test, "c").await, ["x"]);
    assert_eq!(
        held(&test).await.idle_at,
        time::add(test.now(), SECOND).ok(),
        "a slot from the store time of the retry"
    );
}

/// Scenario: Dead-letter redrive.
#[tokio::test(start_paused = true)]
async fn a_submission_refused_for_its_delay_that_the_dead_letters_redrive_is_admitted_after_idle_time() {
    let test = node().await;
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    let watched = WatchTarget::parse("cicada.rate-limiter/api").unwrap();
    test.system().watch(&watched, &addr(&DeadLetters::default())).await.unwrap();
    test.create(&api(), &rate(1, 1, Duration::ZERO)).await;
    submit(&test, note("a", "1")).await;
    submit(&test, note("a", "2")).await;
    let refused = failed(&test).await.pop().expect("the refused submission");
    assert_eq!(refused.cause, FailureCause::Refused);
    test.advance_time_by(SECOND).await;
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: refused.message_id }).await;
    test.run_until_idle().await;
    let pending = test.pending(&worker("a")).await;
    assert_eq!(pending.iter().map(|m| m.payload["text"].as_str().unwrap()).collect::<Vec<_>>(), ["1", "2"]);
    assert_eq!(pending[1].deliver_at, test.now());
}

/// Scenario: A repeated incoming id.
#[tokio::test(start_paused = true)]
async fn a_repeated_submission_id_forwards_and_reserves_once() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, Duration::from_secs(5))).await;
    let id = MessageId::new("submission-1").unwrap();
    test.send_with(&api(), note("a", "x"), SendOptions::new().id(id.clone())).await;
    test.run_until_idle().await;
    let before = held(&test).await;
    let again = test.system().send_with(&api(), note("a", "x"), SendOptions::new().id(id)).await.unwrap();
    assert!(matches!(again, SendResult::Duplicate(_)), "{again:?}");
    test.run_until_idle().await;
    assert_eq!(test.pending(&worker("a")).await.len(), 1);
    assert_eq!(held(&test).await, before);
}

/// Scenario: Cancelling a target message.
#[tokio::test(start_paused = true)]
async fn a_cancelled_forwarded_message_refunds_no_slot() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, Duration::from_secs(5))).await;
    submit(&test, note("a", "x")).await;
    let before = held(&test).await;
    let forwarded = test.pending(&worker("a")).await.pop().unwrap();
    test.system().cancel(&forwarded.id).await.unwrap();
    assert_eq!(status(&test, &forwarded).await, Some(OutcomeStatus::Cancelled));
    assert_eq!(held(&test).await, before);
}

// The guide states the scheduling limits.

/// Scenario: Overdue target messages.
#[tokio::test(start_paused = true)]
async fn a_paused_target_runs_its_overdue_messages_together_when_it_resumes() {
    let test = node().await;
    test.create(&api(), &rate(1, 1, Duration::from_secs(5))).await;
    for n in 0..3 {
        submit(&test, note("a", &n.to_string())).await;
    }
    test.advance_time_by(Duration::from_secs(10)).await;
    assert!(log(&test, "a").await.is_empty());
    test.system().resume_mailbox(&addr(&worker("a"))).await.unwrap();
    let start = test.now();
    test.run_until_idle().await;
    assert_eq!(log(&test, "a").await, ["0", "1", "2"]);
    assert_eq!(test.now(), start, "all three ran at one store time");
}

/// Scenario: A replacement limiter.
#[tokio::test(start_paused = true)]
async fn a_replaced_limiter_starts_fresh_and_forwarded_messages_keep_their_due_times() {
    let test = node().await;
    let start = test.now();
    test.create(&api(), &rate(1, 1, Duration::from_secs(5))).await;
    submit(&test, note("a", "1")).await;
    submit(&test, note("a", "2")).await;
    test.system().delete_actor(&addr(&api())).await.unwrap();
    assert_eq!(due(&test, start).await, [Duration::ZERO, SECOND], "the deleted limiter's messages stay");
    test.create(&api(), &rate(10, 1, Duration::from_secs(5))).await;
    assert_eq!(held(&test).await.idle_at, None);
    submit(&test, note("a", "3")).await;
    assert_eq!(due(&test, start).await, [Duration::ZERO, Duration::ZERO, SECOND]);
}
