//! One test per row of the architecture's outcomes table, plus the handler-context rules.

mod common;

use std::time::Duration;

use cicada::runtime::SendResult;
use cicada::store::{MessageStatus, OutcomeStatus, OutgoingMessage, Store};
use cicada::time;
use cicada::turn::SendOptions;
use cicada::{ActorRef, ActorType};
use cicada_test::TestActors;
use common::*;
use serde_json::json;

async fn harness() -> TestActors {
    TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap()
}

fn a() -> Counter {
    Counter::new("a")
}

#[tokio::test(start_paused = true)]
async fn handler_returns_done_effects_applied_atomically_version_increments() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    let m = test.send(&a(), CounterMsg::Add { n: 2 }).await;
    assert_eq!(test.run_until_idle().await, 1);
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, CounterState>(&a()).await.total, 2);
    assert_eq!(test.version(&a()).await, 1);
}

#[tokio::test(start_paused = true)]
async fn handler_that_does_not_assign_state_leaves_the_version_unchanged() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    test.send(&a(), CounterMsg::Later { n: 1, delay_millis: 1000 }).await;
    test.run_until_idle().await;
    assert_eq!(test.version(&a()).await, 0);
    assert_eq!(test.pending(&a()).await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn handler_fails_failed_with_error_no_effects_mailbox_continues() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    let boom = test.send(&a(), CounterMsg::Boom { reason: "kaput".into() }).await;
    let next = test.send(&a(), CounterMsg::Add { n: 1 }).await;
    test.run_until_idle().await;
    let record = test.outcome(&boom).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert_eq!(record.error.as_deref(), Some("kaput"));
    assert_eq!(record.envelope.attempt, 1);
    assert_eq!(test.outcome(&next).await.unwrap().status, OutcomeStatus::Done);
    // Neither the state change nor the self-send of the failed turn was applied.
    assert_eq!(test.state::<_, CounterState>(&a()).await, CounterState { total: 1, log: vec!["add:1".into()] });
    assert_eq!(test.failures().await.iter().map(|f| f.message_id.clone()).collect::<Vec<_>>(), vec![boom.id]);
}

#[tokio::test(start_paused = true)]
async fn state_that_cannot_be_decoded_at_load_failed_state_untouched() {
    let test = harness().await;
    let bad = json!({"total": "not a number"});
    test.store().create(&a().address(), &template::<Counter>(), Some(bad.clone())).await.unwrap();
    let m = test.send(&a(), CounterMsg::Add { n: 1 }).await;
    test.run_until_idle().await;
    let record = test.outcome(&m).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    let error = record.error.unwrap();
    assert!(error.starts_with("State cannot be decoded"), "{error}");
    assert_eq!(test.state_json(&a()).await, Some(bad));
    assert_eq!(test.version(&a()).await, 0);
}

#[tokio::test(start_paused = true)]
async fn commit_rejected_by_a_send_to_an_absent_instance_failed_with_reason_no_effects() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    let m = test.send(&a(), CounterMsg::Orphan { target: "ghost".into() }).await;
    test.run_until_idle().await;
    let record = test.outcome(&m).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert!(record.error.as_deref().unwrap().contains("actor://counter/ghost"), "{:?}", record.error);
    assert_eq!(test.state::<_, CounterState>(&a()).await, CounterState::default());
    assert!(!test.exists(&Counter::new("ghost")).await);
}

#[tokio::test(start_paused = true)]
async fn turn_exceeding_max_turn_failed_timeout_no_effects_handler_not_re_run() {
    let test = TestActors::builder()
        .with(|n| n.host(ActorType::new(CounterActor).max_turn(Duration::from_secs(30))))
        .build()
        .await
        .unwrap();
    test.create(&a(), &CounterState::default()).await;
    let m = test.send(&a(), CounterMsg::Sleep { millis: 5 * 60 * 1000 }).await;
    let start = test.now();
    test.run_until_idle().await;
    let record = test.outcome(&m).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert_eq!(record.error.as_deref(), Some("timeout"));
    assert_eq!(time::between(start, record.finished_at), Duration::from_secs(30));
    assert_eq!(test.state::<_, CounterState>(&a()).await, CounterState::default());
    assert_eq!(test.run_until_idle().await, 0); // G6: nothing is retried
}

#[tokio::test(start_paused = true)]
async fn a_handlers_own_timeout_error_is_a_failure_not_the_turn_timeout() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    let inner = test.send(&a(), CounterMsg::InnerTimeout).await;
    let ok = test.send(&a(), CounterMsg::Sleep { millis: 10_000 }).await; // within max_turn (1 min)
    test.run_until_idle().await;
    let record = test.outcome(&inner).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    // tokio's `Elapsed` of the handler's own timeout, not the runtime's "timeout".
    assert_eq!(record.error.as_deref(), Some("deadline has elapsed"));
    assert_eq!(test.outcome(&ok).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, CounterState>(&a()).await.log, vec!["sleep:10000", "woke"]);
}

#[tokio::test(start_paused = true)]
async fn stop_purges_the_mailbox_including_the_turns_own_self_sends() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    let halt = test.send(&a(), CounterMsg::Halt).await;
    let later = test.send_with(&a(), CounterMsg::Add { n: 5 }, SendOptions::new().delay(Duration::from_secs(60))).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&halt).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.outcome(&later).await.unwrap().status, OutcomeStatus::Purged);
    assert!(!test.exists(&a()).await);
    assert!(test.system().pending(&a().address(), None, 100).await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn create_then_send_in_one_turn() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    test.send(&a(), CounterMsg::Spawn { child: "child".into(), n: 7 }).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, CounterState>(&Counter::new("child")).await.total, 107);
}

#[tokio::test(start_paused = true)]
async fn sends_with_equal_deliver_at_keep_send_order_and_delayed_ones_come_later() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    test.send(&a(), CounterMsg::Later { n: 99, delay_millis: 500 }).await;
    test.send(&a(), CounterMsg::Burst { count: 4 }).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, CounterState>(&a()).await.log, vec!["add:1", "add:2", "add:3", "add:4"]);
    test.advance_time_by(Duration::from_millis(500)).await;
    assert_eq!(test.state::<_, CounterState>(&a()).await.log, vec!["add:1", "add:2", "add:3", "add:4", "add:99"]);
}

#[tokio::test(start_paused = true)]
async fn a_message_is_never_delivered_early() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    test.send_with(&a(), CounterMsg::Add { n: 1 }, SendOptions::new().delay(Duration::from_secs(10))).await;
    assert_eq!(test.run_until_idle().await, 0);
    test.advance_time_by(Duration::from_secs(9)).await;
    assert_eq!(test.state::<_, CounterState>(&a()).await.total, 0);
    test.advance_time_by(Duration::from_secs(1)).await;
    assert_eq!(test.state::<_, CounterState>(&a()).await.total, 1);
}

#[tokio::test(start_paused = true)]
async fn handler_sees_the_envelope_and_store_time_at_claim() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    let m =
        test.send_with(&a(), CounterMsg::Inspect, SendOptions::new().correlation_id("c-1").header("trace", "t")).await;
    test.run_until_idle().await;
    assert_eq!(
        test.state::<_, CounterState>(&a()).await.log,
        vec![format!(
            "attempt=1 from=null type=Inspect now={} deliverAt={} corr=c-1 headers={{\"trace\": \"t\"}}",
            time::format_instant(test.now()),
            time::format_instant(m.deliver_at)
        )]
    );
}

#[tokio::test(start_paused = true)]
async fn reply_to_message_from() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    test.create(&Counter::new("b"), &CounterState::default()).await;
    // b asks a for a ping; a replies to b (raw address from the envelope).
    let mut ping = OutgoingMessage::new(a().address(), "Ping", json!({"n": 3}));
    ping.from = Some(Counter::new("b").address());
    enqueue(&**test.store(), ping).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, CounterState>(&Counter::new("b")).await.total, 3);
    assert!(test.failures().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn raw_send_is_checked_against_the_registry_when_called_and_the_error_is_catchable() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    test.send(&a(), CounterMsg::SendUnknown { to: "actor://counter/a".into() }).await;
    test.send(&a(), CounterMsg::SendUnknown { to: "actor://nowhere/x".into() }).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, CounterState>(&a()).await.log, vec!["rejected:Nope", "rejected:Nope"]);
}

#[tokio::test(start_paused = true)]
async fn raw_send_of_a_known_type_is_buffered_and_delivered() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    test.create(&Counter::new("b"), &CounterState::default()).await;
    test.send(&a(), CounterMsg::Forward { to: "actor://counter/b".into(), n: 4 }).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, CounterState>(&Counter::new("b")).await.total, 4);
}

#[tokio::test(start_paused = true)]
async fn stateless_type_forwards_and_has_null_state() {
    let test = TestActors::builder().with(|n| n.host(CounterActor).host(EchoActor)).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    test.create_stateless(&Echo::new("e")).await;
    test.send(&Echo::new("e"), EchoMsg::Hello { counter: "a".into() }).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, CounterState>(&a()).await.total, 1);
    assert_eq!(test.state_json(&Echo::new("e")).await, None);
}

#[tokio::test(start_paused = true)]
async fn g6_a_failed_message_is_terminal_and_attempt_stays_1() {
    let test = harness().await;
    test.create(&a(), &CounterState::default()).await;
    let m = test.send(&a(), CounterMsg::Boom { reason: "x".into() }).await;
    test.run_until_idle().await;
    test.advance_time_by(Duration::from_secs(60)).await;
    match test.system().status(&m.id).await.unwrap() {
        Some(MessageStatus::Finished(outcome)) => assert_eq!(outcome.envelope.attempt, 1),
        other => panic!("not finished: {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn payload_json_for_a_stateless_send() {
    let test = TestActors::builder().with(|n| n.host(EchoActor).host(CounterActor)).build().await.unwrap();
    test.create_stateless(&Echo::new("e")).await;
    test.create(&a(), &CounterState::default()).await;
    let r = test
        .send_raw(&ActorRef::address(&Echo::new("e")).unwrap(), "Hello", json!({"counter": "a"}), SendOptions::new())
        .await;
    assert!(matches!(r, SendResult::Accepted(_)), "{r:?}");
    test.run_until_idle().await;
    assert_eq!(test.state::<_, CounterState>(&a()).await.total, 1);
}
