//! The batcher, on virtual time with `TestActors`: the strict count bound with queued Adds, single-item batches, the
//! first item's timer, ordered items and null, both orders at an equal due time, public and empty flushes, failed
//! forwarding that rolls back its turn, recovery, redrive into a later batch, lost timers, a restored batch without a
//! timer, item depth, creation and the policies of the target.

mod common;

use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::RetryResult;
use cicada::store::{Message, OutcomeStatus};
use cicada::time::{self, Timestamp};
use cicada::{Failed, FailureCause, WatchTarget};
use cicada_actors::{
    Batcher, BatcherActor, BatcherMsg, BatcherState, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState,
};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SECOND: Duration = Duration::from_secs(1);

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn bat() -> Batcher {
    Batcher::named("b")
}

/// Sends `Items` to `worker/{to}` once it holds `max_items` items, at the latest `max_wait` seconds after the first.
fn settings(to: &str, max_items: u32, max_wait: u64) -> BatcherState {
    BatcherState::new(&worker(to), "Items", max_items, secs(max_wait)).unwrap()
}

fn item(value: impl Serialize) -> BatcherMsg {
    BatcherMsg::add(value).unwrap()
}

fn two_attempts() -> RetryPolicy {
    RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(10)).unwrap()).unwrap()
}

/// The crate's shared retry policy, which the timer carries.
fn shared_policy() -> RetryPolicy {
    let backoff = Backoff::exponential(SECOND, 2.0, Duration::from_secs(32)).unwrap();
    RetryPolicy::new(7, backoff).unwrap()
}

/// `producer/_`: creates the batcher `b` with the settings it is given and adds an item to it, in one turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Producer;

impl ActorRef for Producer {
    const TYPE: &'static str = "producer";
    type Message = ProducerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ProducerMsg {
    Go { settings: BatcherState, item: String },
}

struct ProducerActor;

impl Actor for ProducerActor {
    type Ref = Producer;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProducerMsg) -> HandlerResult {
        let ProducerMsg::Go { settings, item: value } = message;
        turn.create(&bat(), &settings)?;
        turn.send(&bat(), item(value))?;
        Ok(())
    }
}

/// A node at `start` with the batcher type, the workers `worker/a` and `worker/b` (`worker/a` paused, so that the
/// batches wait in its mailbox), the producer, the dead letters (not created) and the witness, which watches the
/// batchers.
async fn node_at(start: &str) -> TestActors {
    let test = TestActors::builder()
        .start_at(ts(start))
        .with(|node| {
            node.host(BatcherActor)
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
    witness(&test, "cicada.batcher").await;
    test
}

async fn node() -> TestActors {
    node_at("2026-01-01T00:00:00Z").await
}

/// Sends `b` `message` from a client and runs what is due.
async fn send(test: &TestActors, message: BatcherMsg) -> Message {
    let m = test.send(&bat(), message).await;
    test.run_until_idle().await;
    m
}

async fn held(test: &TestActors) -> BatcherState {
    test.state(&bat()).await
}

/// The batches waiting for `worker/a`: the items of each and its due time as an offset from `start`.
async fn batches(test: &TestActors, start: Timestamp) -> Vec<(Value, Duration)> {
    test.pending(&worker("a"))
        .await
        .iter()
        .map(|m| (m.payload["items"].clone(), time::between(start, m.deliver_at)))
        .collect()
}

/// The flushes waiting in the mailbox of `b`, as offsets from `start`; asserts that the last is the armed one.
async fn flushes(test: &TestActors, start: Timestamp) -> Vec<Duration> {
    let pending: Vec<Message> = test.pending(&bat()).await.into_iter().filter(|m| m.message_type == "Flush").collect();
    if let Some(last) = pending.last() {
        assert_eq!(held(test).await.armed.as_ref(), Some(&last.id), "the armed flush");
    }
    pending.iter().map(|m| time::between(start, m.deliver_at)).collect()
}

async fn log(test: &TestActors, id: &str) -> Vec<String> {
    test.state::<_, WorkerState>(&worker(id)).await.log
}

/// The `Failed` the witness was told of the batcher.
async fn failed(test: &TestActors) -> Vec<Failed> {
    told(test).await.into_iter().filter(|f| f.actor == addr(&bat())).collect()
}

/// The causes of the failures of the batcher's messages, as the core told them to the witness, read from the
/// witness's own failed turns: a `Failed` that holds a deep item is too deep for the witness to read.
async fn causes_too_deep_to_tell(test: &TestActors, m: &Message) -> Vec<Value> {
    test.failures()
        .await
        .into_iter()
        .filter(|f| f.envelope.message_type == "Failed" && f.envelope.payload["messageId"] == json!(m.id))
        .map(|f| f.envelope.payload["cause"].clone())
        .collect()
}

/// An item `levels` arrays deep.
fn deep(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, _| Value::Array(vec![inner]))
}

// A batcher has fixed validated settings.

/// A batcher created from the minimal JSON holds nothing.
#[tokio::test(start_paused = true)]
async fn a_batcher_created_from_the_minimal_json_is_empty() {
    let test = node().await;
    test.create(&bat(), &json!({"target": "worker/a", "messageType": "Items", "maxItems": 3, "maxWait": "PT10S"}))
        .await;
    assert_eq!(held(&test).await, settings("a", 3, 10));
}

// Add appends in handling order and flushes inline at the limit.

/// Scenario: Count flush.
#[tokio::test(start_paused = true)]
async fn queued_adds_flush_on_the_one_that_fills_the_batch_and_no_stored_batch_reaches_the_limit() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    for x in ["a", "b", "c", "d"] {
        test.send(&bat(), item(x)).await;
    }
    while test.system().run_due_turns(1).await.unwrap() > 0 {
        assert!(held(&test).await.items.len() < 3, "a stored batch holds fewer than maxItems");
    }
    assert_eq!(batches(&test, start).await, [(json!(["a", "b", "c"]), Duration::ZERO)]);
    let state = held(&test).await;
    assert_eq!((state.items, state.first_at), (vec![json!("d")], Some(start)), "d starts a new batch");
    assert_eq!(flushes(&test, start).await, [secs(10)], "one timer: the first batch's was cancelled");
    assert!(failed(&test).await.is_empty());
}

/// Scenario: Single-item batches.
#[tokio::test(start_paused = true)]
async fn with_max_items_1_each_add_is_sent_at_once_without_a_timer() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 1, 10)).await;
    send(&test, item("x")).await;
    send(&test, item("y")).await;
    assert_eq!(batches(&test, start).await, [(json!(["x"]), Duration::ZERO), (json!(["y"]), Duration::ZERO)]);
    assert_eq!(held(&test).await, settings("a", 1, 10));
    assert!(test.pending(&bat()).await.is_empty(), "no timer");
}

/// Scenario: A null item; the items keep their handling order.
#[tokio::test(start_paused = true)]
async fn a_null_item_counts_and_is_forwarded_as_null_in_order() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    send(&test, item(())).await;
    assert_eq!(held(&test).await.items, [Value::Null]);
    send(&test, item(2)).await;
    send(&test, item(json!({"k": [3]}))).await;
    assert_eq!(batches(&test, start).await, [(json!([null, 2, {"k": [3]}]), Duration::ZERO)]);
}

// The first retained item arms the longest wait.

/// Scenario: Time flush.
#[tokio::test(start_paused = true)]
async fn one_item_is_flushed_at_the_longest_wait_and_not_before() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    send(&test, item("a")).await;
    let timer = test.pending(&bat()).await.pop().unwrap();
    assert_eq!(timer.retry, Some(shared_policy()), "the timer has the shared retry policy");
    assert_eq!(flushes(&test, start).await, [secs(10)]);
    test.advance_time_by(secs(9) + Duration::from_millis(999)).await;
    assert!(batches(&test, start).await.is_empty());
    test.advance_time_by(Duration::from_millis(1)).await;
    assert_eq!(batches(&test, start).await, [(json!(["a"]), secs(10))]);
    assert_eq!(held(&test).await, settings("a", 3, 10), "cleared");
}

/// Scenario: New batch deadline; later items keep the first item's timer.
#[tokio::test(start_paused = true)]
async fn a_count_flush_cancels_the_timer_and_the_next_item_arms_a_new_one_from_its_own_time() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    send(&test, item("a")).await;
    let timer = test.pending(&bat()).await.pop().unwrap();
    test.advance_time_by(SECOND).await;
    send(&test, item("b")).await;
    let state = held(&test).await;
    assert_eq!((state.first_at, state.armed), (Some(start), Some(timer.id.clone())), "b keeps the timer");
    test.advance_time_by(SECOND).await;
    send(&test, item("c")).await;
    assert_eq!(batches(&test, start).await, [(json!(["a", "b", "c"]), secs(2))]);
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Cancelled));
    test.advance_time_by(secs(2)).await;
    send(&test, item("d")).await;
    assert_eq!(flushes(&test, start).await, [secs(14)]);
}

/// Scenario: Overdue Add. The timer failed, so the batch missed its deadline.
#[tokio::test(start_paused = true)]
async fn an_add_after_the_deadline_sends_the_held_items_with_its_own() {
    let test = node().await;
    test.create(&bat(), &settings("c", 3, 10)).await;
    send(&test, item("a")).await;
    test.advance_time_by(secs(10)).await;
    let failure = failed(&test).await.pop().expect("the flush to the absent worker failed");
    assert_eq!((failure.cause, failure.message_type.as_str()), (FailureCause::Commit, "Flush"));
    assert_eq!(held(&test).await.items, [json!("a")], "nothing cleared");
    test.create(&worker("c"), &WorkerState::default()).await;
    test.advance_time_by(secs(2)).await;
    send(&test, item("b")).await;
    assert_eq!(log(&test, "c").await, [r#"["a","b"]"#]);
    assert_eq!(held(&test).await, settings("c", 3, 10));
    assert!(test.pending(&bat()).await.is_empty());
}

/// Scenario: Deadline overflow; a count flush needs no deadline.
#[tokio::test(start_paused = true)]
async fn an_add_whose_deadline_would_pass_max_instant_fails_for_good_and_a_full_batch_is_still_sent() {
    let test = node_at("9999-12-31T23:59:55Z").await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    let m = send(&test, item("a")).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    let refused = failed(&test).await;
    assert_eq!(refused.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused]);
    assert!(refused[0].error.contains("after 9999-12-31"), "{}", refused[0].error);
    assert_eq!(held(&test).await, settings("a", 3, 10), "no item");
    assert!(test.pending(&bat()).await.is_empty(), "no timer");
    let one = Batcher::named("one");
    test.create(&one, &settings("a", 1, 10)).await;
    test.send(&one, item("x")).await;
    test.run_until_idle().await;
    assert_eq!(batches(&test, start).await, [(json!(["x"]), Duration::ZERO)]);
}

// Public Flush sends the current batch and clears atomically.

/// Scenario: Early manual flush.
#[tokio::test(start_paused = true)]
async fn a_manual_flush_before_the_timer_sends_the_partial_batch_and_cancels_the_timer() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    send(&test, item("a")).await;
    let timer = test.pending(&bat()).await.pop().unwrap();
    test.advance_time_by(SECOND).await;
    send(&test, BatcherMsg::Flush).await;
    assert_eq!(batches(&test, start).await, [(json!(["a"]), SECOND)]);
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Cancelled));
    assert_eq!(held(&test).await, settings("a", 3, 10));
    assert!(test.pending(&bat()).await.is_empty());
}

/// Scenario: Timer flush and empty flush.
#[tokio::test(start_paused = true)]
async fn the_armed_flush_sends_without_cancelling_itself_and_an_empty_flush_does_nothing() {
    let test = node().await;
    test.create(&bat(), &settings("b", 3, 10)).await;
    send(&test, item("x")).await;
    let timer = test.pending(&bat()).await.pop().unwrap();
    test.advance_time_by(secs(10)).await;
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Done));
    assert_eq!(log(&test, "b").await, [r#"["x"]"#]);
    assert_eq!(held(&test).await, settings("b", 3, 10));
    let m = send(&test, BatcherMsg::Flush).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(log(&test, "b").await.len(), 1, "nothing sent");
    assert!(test.pending(&bat()).await.is_empty());
    assert!(test.exists(&bat()).await, "it never stops itself");
    assert!(failed(&test).await.is_empty());
}

/// Scenario: Equal-time Add and Flush; the Add comes first in the core's order and joins the batch.
#[tokio::test(start_paused = true)]
async fn an_add_due_with_the_timer_and_ordered_first_joins_the_batch() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    test.send(&bat(), item("a")).await;
    // Sent before the first Add is handled: its sequence is lower than that of the timer the first one arms.
    test.send_with(&bat(), item("b"), SendOptions::new().delay(secs(10))).await;
    test.run_until_idle().await;
    let timer = test.pending(&bat()).await.into_iter().find(|m| m.message_type == "Flush").unwrap();
    test.advance_time_by(secs(10)).await;
    assert_eq!(batches(&test, start).await, [(json!(["a", "b"]), secs(10))], "the Add at the deadline sent it");
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Cancelled));
    assert_eq!(held(&test).await, settings("a", 3, 10));
}

/// Scenario: Equal-time Add and Flush; the timer comes first and the Add starts the next batch.
#[tokio::test(start_paused = true)]
async fn an_add_due_with_the_timer_and_ordered_after_it_starts_the_next_batch() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    send(&test, item("a")).await;
    let timer = test.pending(&bat()).await.pop().unwrap();
    test.send_with(&bat(), item("b"), SendOptions::new().delay(secs(10))).await;
    test.advance_time_by(secs(10)).await;
    assert_eq!(batches(&test, start).await, [(json!(["a"]), secs(10))]);
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Done), "it did not cancel itself");
    let state = held(&test).await;
    assert_eq!((state.items, state.first_at), (vec![json!("b")], time::add(start, secs(10)).ok()));
    assert_eq!(flushes(&test, start).await, [secs(20)]);
}

// A delivered batch carries the selected target policy.

/// Scenario: Distinct policies and context.
#[tokio::test(start_paused = true)]
async fn the_batch_has_the_target_policy_no_correlation_id_a_new_id_and_the_batcher_as_sender() {
    let test = node().await;
    let mut state = settings("a", 2, 10);
    state.retry = Some(two_attempts());
    test.create(&bat(), &state).await;
    let incoming = RetryPolicy::new(3, Backoff::fixed(SECOND).unwrap()).unwrap();
    let options = || SendOptions::new().correlation_id("c").header("h", "v").retry(incoming.clone());
    test.send_with(&bat(), item("x"), options()).await;
    let m = test.send_with(&bat(), item("y"), options()).await;
    test.run_until_idle().await;
    let pending = test.pending(&worker("a")).await;
    assert_eq!(pending.len(), 1);
    let target = &pending[0];
    assert_eq!((target.message_type.as_str(), &target.payload), ("Items", &json!({"items": ["x", "y"]})));
    assert_eq!(target.retry, Some(two_attempts()));
    assert_eq!(target.correlation_id, None);
    assert_eq!(target.from, Some(addr(&bat())));
    assert_ne!(target.id, m.id);
    assert_eq!(target.retention, None);
    assert!(!target.headers.contains_key("h"), "{:?}", target.headers);
}

/// A retry at the target does not pass the batcher again.
#[tokio::test(start_paused = true)]
async fn a_retry_at_the_target_bypasses_the_batcher() {
    let test = node().await;
    let mut state = BatcherState::new(&worker("b"), "ItemsFailOnce", 1, secs(10)).unwrap();
    state.retry = Some(two_attempts());
    test.create(&bat(), &state).await;
    send(&test, item("x")).await;
    assert!(log(&test, "b").await.is_empty(), "the first run failed");
    assert_eq!(held(&test).await, state);
    test.advance_time_by(secs(10)).await;
    assert_eq!(log(&test, "b").await, [r#"["x"]"#]);
    assert!(test.pending(&bat()).await.is_empty());
}

// Failed forwarding rolls back the triggering turn.

/// Scenario: Count flush to an absent target, and Failed final Add is redriven.
#[tokio::test(start_paused = true)]
async fn a_full_batch_for_an_absent_target_keeps_the_earlier_items_and_its_redriven_add_sends_them_once() {
    let test = node().await;
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    let watched = WatchTarget::parse("cicada.batcher/b").unwrap();
    test.system().watch(&watched, &addr(&DeadLetters::default())).await.unwrap();
    test.create(&bat(), &settings("c", 3, 60)).await;
    send(&test, item("a")).await;
    send(&test, item("b")).await;
    let before = held(&test).await;
    let timer = test.pending(&bat()).await.pop().unwrap();
    let c = send(&test, item("c")).await;
    let failure = failed(&test).await.pop().expect("Add c failed");
    assert_eq!((failure.cause, failure.message_id.clone()), (FailureCause::Commit, c.id.clone()));
    assert_eq!(held(&test).await, before, "[a, b] remain");
    assert_eq!(status(&test, &timer).await, None, "the timer was not cancelled");
    send(&test, item("d")).await;
    assert_eq!(held(&test).await.items.len(), 2, "failing Adds do not grow the batch");
    test.create(&worker("c"), &WorkerState::default()).await;
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: c.id.clone() }).await;
    test.run_until_idle().await;
    assert_eq!(log(&test, "c").await, [r#"["a","b","c"]"#], "once");
    assert_eq!(held(&test).await, settings("c", 3, 60));
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Cancelled));
}

/// A failed final Add retried by an operator sends the batch once, too.
#[tokio::test(start_paused = true)]
async fn a_failed_final_add_retried_by_an_operator_sends_the_batch_once() {
    let test = node().await;
    test.create(&bat(), &settings("c", 2, 60)).await;
    send(&test, item("a")).await;
    let b = send(&test, item("b")).await;
    assert_eq!(status(&test, &b).await, Some(OutcomeStatus::Failed));
    test.create(&worker("c"), &WorkerState::default()).await;
    let RetryResult::Accepted(_) = test.system().retry_message(&b.id).await.unwrap() else { panic!("not retried") };
    test.run_until_idle().await;
    assert_eq!(log(&test, "c").await, [r#"["a","b"]"#]);
    assert_eq!(held(&test).await, settings("c", 2, 60));
}

/// Scenario: Refused message type: an unregistered target type, and a message type the worker does not take.
#[tokio::test(start_paused = true)]
async fn a_batch_for_an_unregistered_type_or_of_a_message_the_target_does_not_take_is_refused_and_keeps_the_items() {
    for (target, message_type) in [("nobody/x", "Items"), ("worker/a", "Shout")] {
        let test = node().await;
        let json = json!({"target": target, "messageType": message_type, "maxItems": 2, "maxWait": "PT10S"});
        test.create(&bat(), &json).await;
        send(&test, item("a")).await;
        let before = held(&test).await;
        send(&test, BatcherMsg::Flush).await;
        assert_eq!(held(&test).await, before, "the items stay after a Flush");
        send(&test, item("b")).await;
        assert_eq!(held(&test).await, before, "the items stay after a full Add");
        let refused = failed(&test).await;
        assert_eq!(refused.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused; 2], "{target}");
        assert_eq!(refused.iter().map(|f| f.message_type.as_str()).collect::<Vec<_>>(), ["Flush", "Add"]);
        assert!(test.pending(&worker("a")).await.is_empty());
    }
}

/// Scenario: Target cannot decode the batch.
#[tokio::test(start_paused = true)]
async fn a_batch_the_target_cannot_decode_fails_there_with_the_whole_batch_and_the_batcher_stays_cleared() {
    let test = node().await;
    test.create(&bat(), &BatcherState::new(&worker("b"), "Note", 2, secs(10)).unwrap()).await;
    send(&test, item("x")).await;
    send(&test, item("y")).await;
    let failures = test.failures().await;
    let at_target: Vec<_> = failures.iter().filter(|f| f.envelope.message_type == "Note").collect();
    assert_eq!(at_target.len(), 1, "{failures:?}");
    assert_eq!(at_target[0].envelope.payload, json!({"items": ["x", "y"]}));
    assert!(failed(&test).await.is_empty(), "the batcher's own turns are done");
    assert_eq!(held(&test).await.items, Vec::<Value>::new());
}

// Recovery flushes current data.

/// Scenario: A cancelled timer, and a purged one.
#[tokio::test(start_paused = true)]
async fn a_lost_timer_keeps_the_items_unarmed_until_an_overdue_add_or_a_manual_flush() {
    for purge in [false, true] {
        let test = node().await;
        let start = test.now();
        test.create(&bat(), &settings("a", 4, 10)).await;
        send(&test, item("a")).await;
        let timer = test.pending(&bat()).await.pop().unwrap();
        if purge {
            test.system().purge_mailbox(&addr(&bat())).await.unwrap();
        } else {
            test.system().cancel(&timer.id).await.unwrap();
        }
        test.advance_time_by(secs(5)).await;
        send(&test, item("b")).await;
        let state = held(&test).await;
        assert_eq!(state.items, [json!("a"), json!("b")], "purge {purge}");
        assert_eq!(state.armed, Some(timer.id.clone()), "the old reference stays, purge {purge}");
        assert!(test.pending(&bat()).await.is_empty(), "no replacement timer, purge {purge}");
        assert!(failed(&test).await.is_empty(), "nobody is told of a cancel or a purge");
        test.advance_time_by(secs(10)).await;
        assert!(batches(&test, start).await.is_empty());
        if purge {
            send(&test, BatcherMsg::Flush).await;
            assert_eq!(batches(&test, start).await, [(json!(["a", "b"]), secs(15))]);
        } else {
            send(&test, item("c")).await;
            assert_eq!(batches(&test, start).await, [(json!(["a", "b", "c"]), secs(15))]);
        }
        assert_eq!(held(&test).await, settings("a", 4, 10));
    }
}

/// Scenario: Old flush redriven into a new batch.
#[tokio::test(start_paused = true)]
async fn an_old_failed_flush_redriven_flushes_the_new_batch_early() {
    let test = node().await;
    let start = test.now();
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    let watched = WatchTarget::parse("cicada.batcher/b").unwrap();
    test.system().watch(&watched, &addr(&DeadLetters::default())).await.unwrap();
    test.create(&bat(), &settings("c", 3, 10)).await;
    send(&test, item("old")).await;
    test.advance_time_by(secs(10)).await;
    let old = failed(&test).await.pop().expect("the failed flush");
    let letters: DeadLettersState = test.state(&DeadLetters::default()).await;
    assert!(letters.letters.contains_key(&old.message_id), "the dead letters keep the failed flush");
    // The old batch ends by a manual flush once its target is there; its dead letter stays.
    test.create(&worker("c"), &WorkerState::default()).await;
    send(&test, BatcherMsg::Flush).await;
    assert_eq!(log(&test, "c").await, [r#"["old"]"#]);
    test.advance_time_by(secs(10)).await;
    send(&test, item("new")).await;
    let timer = test.pending(&bat()).await.pop().expect("the new batch's timer");
    assert_eq!(time::between(start, timer.deliver_at), secs(30), "due in the future");
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: old.message_id }).await;
    test.run_until_idle().await;
    assert_eq!(log(&test, "c").await, [r#"["old"]"#, r#"["new"]"#], "the new batch, early");
    assert_eq!(held(&test).await, settings("c", 3, 10));
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Cancelled), "the new batch's timer was cancelled");
}

/// Scenario: A restored partial batch without a timer.
#[tokio::test(start_paused = true)]
async fn a_restored_batch_without_a_timer_stays_unarmed_until_an_overdue_add_or_a_flush() {
    for manual in [false, true] {
        let test = node().await;
        let start = test.now();
        let restored = json!({"target": "worker/a", "messageType": "Items", "maxItems": 4, "maxWait": "PT10S",
            "items": ["r"], "firstAt": time::format_instant(start)});
        test.create(&bat(), &restored).await;
        test.advance_time_by(secs(2)).await;
        send(&test, item("b")).await;
        let state = held(&test).await;
        assert_eq!((state.items.len(), state.armed), (2, None), "unarmed, manual {manual}");
        assert!(test.pending(&bat()).await.is_empty());
        test.advance_time_by(secs(8)).await;
        if manual {
            send(&test, BatcherMsg::Flush).await;
            assert_eq!(batches(&test, start).await, [(json!(["r", "b"]), secs(10))]);
        } else {
            send(&test, item("c")).await;
            assert_eq!(batches(&test, start).await, [(json!(["r", "b", "c"]), secs(10))], "at the deadline");
        }
        assert_eq!(held(&test).await, settings("a", 4, 10));
    }
}

// Core depth checks protect the batch transaction.

/// Scenario: Depth boundaries.
#[tokio::test(start_paused = true)]
async fn an_item_of_62_levels_is_kept_and_sent_and_one_of_63_fails_and_keeps_the_earlier_batch() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    send(&test, BatcherMsg::Add { item: deep(62) }).await;
    send(&test, BatcherMsg::Flush).await;
    assert_eq!(batches(&test, start).await, [(json!([deep(62)]), Duration::ZERO)], "62 levels kept and sent");

    send(&test, item("kept")).await;
    let before = held(&test).await;
    let m = send(&test, BatcherMsg::Add { item: deep(63) }).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    let error = test.outcome(&m).await.unwrap().error.unwrap_or_default();
    assert!(error.contains("64"), "the state check: {error}");
    assert_eq!(causes_too_deep_to_tell(&test, &m).await, [json!("handler")]);
    assert_eq!(held(&test).await, before);
    assert_eq!(flushes(&test, start).await, [secs(10)], "the earlier timer stays");

    send(&test, item("second")).await;
    let before = held(&test).await;
    let m = send(&test, BatcherMsg::Add { item: deep(63) }).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    let error = test.outcome(&m).await.unwrap().error.unwrap_or_default();
    assert!(error.contains("64"), "the send check: {error}");
    assert_eq!(causes_too_deep_to_tell(&test, &m).await, [json!("refused")]);
    assert_eq!(held(&test).await, before);
    assert_eq!(batches(&test, start).await.len(), 1, "nothing more sent");
}

// Creation and documentation retain the core semantics.

/// Scenario: Repeated create; create-and-add in one turn.
#[tokio::test(start_paused = true)]
async fn a_producer_creates_and_adds_in_one_turn_and_a_repeated_create_keeps_settings_and_items() {
    let test = node().await;
    let start = test.now();
    test.send(&Producer, ProducerMsg::Go { settings: settings("a", 3, 10), item: "x".into() }).await;
    test.run_until_idle().await;
    assert_eq!(held(&test).await.items, [json!("x")]);
    assert_eq!(flushes(&test, start).await, [secs(10)]);
    test.send(&Producer, ProducerMsg::Go { settings: settings("b", 2, 5), item: "y".into() }).await;
    test.run_until_idle().await;
    let state = held(&test).await;
    assert_eq!((state.target, state.max_items, state.max_wait), (addr(&worker("a")), 3, secs(10)), "settings stay");
    assert_eq!(state.items, [json!("x"), json!("y")]);
    assert!(failed(&test).await.is_empty());
}

/// Scenario: Pause and delete.
#[tokio::test(start_paused = true)]
async fn a_paused_batcher_flushes_when_it_resumes_and_a_delete_discards_only_its_held_items() {
    let test = node().await;
    let start = test.now();
    test.create(&bat(), &settings("a", 3, 10)).await;
    send(&test, item("a")).await;
    test.system().pause_mailbox(&addr(&bat())).await.unwrap();
    test.advance_time_by(secs(20)).await;
    assert!(batches(&test, start).await.is_empty());
    test.system().resume_mailbox(&addr(&bat())).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(batches(&test, start).await, [(json!(["a"]), secs(20))], "flushed when claimed");
    send(&test, item("b")).await;
    test.system().delete_actor(&addr(&bat())).await.unwrap();
    assert!(!test.exists(&bat()).await);
    assert_eq!(batches(&test, start).await, [(json!(["a"]), secs(20))], "the forwarded batch stays");
}
