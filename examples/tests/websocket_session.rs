//! Example 20: the websocket session's protocol and its host bridge, on virtual time.

use std::collections::VecDeque;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::turn::TurnError;
use cicada::{OpError, SendResult};
use cicada_examples::websocket_session::{
    Answer, Frame, MAX_BODY_BYTES, MAX_FRAMES, MAX_SEQ, Producer, ProducerActor, ProducerMsg, ProducerState, Refusal,
    Session, SessionActor, SessionMsg, SessionState,
};
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

fn session() -> Session {
    Session { stream: "s-1".into() }
}

fn producer() -> Producer {
    Producer { id: "p-1".into() }
}

async fn harness() -> TestActors {
    TestActors::builder()
        .with(|node| node.host(SessionActor).host(ProducerActor).host(NulSender))
        .build()
        .await
        .unwrap()
}

/// A harness with a session in `state` and its producer.
async fn with_session(state: SessionState) -> TestActors {
    let test = harness().await;
    test.create(&session(), &state).await;
    test.create(&producer(), &ProducerState::new(session())).await;
    test
}

/// Emits `body` through the producer, runs every turn, and returns the producer's answer.
async fn emit(test: &TestActors, body: impl Into<String>) -> Answer {
    test.send(&producer(), ProducerMsg::Emit { body: body.into() }).await;
    test.run_until_idle().await;
    let state: ProducerState = test.state(&producer()).await;
    assert_eq!(state.pending, None, "the session answered");
    state.last.expect("an answer")
}

async fn ack(test: &TestActors, stream: &str, through: i64) {
    test.send(&session(), SessionMsg::Ack { stream: stream.into(), through }).await;
    test.run_until_idle().await;
}

async fn session_state(test: &TestActors) -> SessionState {
    test.state(&session()).await
}

fn frames(range: std::ops::RangeInclusive<u64>) -> VecDeque<Frame> {
    range.map(|seq| Frame { seq, body: format!("f{seq}") }).collect()
}

fn accepted(seq: u64) -> Answer {
    Answer::Accepted { seq }
}

fn rejected(reason: Refusal) -> Answer {
    Answer::Rejected { reason }
}

#[tokio::test(start_paused = true)]
async fn an_accepted_body_becomes_the_next_frame_and_its_reply_names_the_request() {
    let test = with_session(SessionState::default()).await;
    test.send(&producer(), ProducerMsg::Emit { body: "hello".into() }).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the producer's turn");
    let pending = test.state::<_, ProducerState>(&producer()).await.pending.expect("one request in flight");
    let enqueue = test.pending(&session()).await;
    assert_eq!(enqueue.len(), 1);
    assert_eq!(enqueue[0].id, pending.request, "the producer keeps the Enqueue's message id");

    test.run_until_idle().await;
    let state = session_state(&test).await;
    assert_eq!((state.assigned, state.acked), (1, 0));
    assert_eq!(state.frames, [Frame { seq: 1, body: "hello".into() }]);
    let producer_state: ProducerState = test.state(&producer()).await;
    assert_eq!((producer_state.pending, producer_state.last), (None, Some(accepted(1))));
    assert_eq!(emit(&test, "again").await, accepted(2));
    assert!(test.failures().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_producer_keeps_at_most_one_request_in_flight() {
    let test = with_session(SessionState::default()).await;
    test.send(&producer(), ProducerMsg::Emit { body: "one".into() }).await;
    let second = test.send(&producer(), ProducerMsg::Emit { body: "two".into() }).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&second).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert!(outcome.error.unwrap().contains("A request is pending"));
    assert_eq!(session_state(&test).await.frames, [Frame { seq: 1, body: "one".into() }]);
}

#[tokio::test(start_paused = true)]
async fn the_body_limit_counts_utf8_bytes() {
    let test = with_session(SessionState::default()).await;
    assert_eq!(emit(&test, "a".repeat(MAX_BODY_BYTES)).await, accepted(1));
    assert_eq!(emit(&test, "a".repeat(MAX_BODY_BYTES + 1)).await, rejected(Refusal::InvalidBody));
    assert_eq!(emit(&test, "é".repeat(MAX_BODY_BYTES / 2)).await, accepted(2));
    // 1024 characters, 1025 bytes.
    assert_eq!(emit(&test, format!("{}é", "a".repeat(MAX_BODY_BYTES - 1))).await, rejected(Refusal::InvalidBody));
    assert_eq!(session_state(&test).await.assigned, 2);
}

#[tokio::test(start_paused = true)]
async fn a_full_buffer_refuses_until_an_ack_frees_space() {
    let test = with_session(SessionState::default()).await;
    for seq in 1..=MAX_FRAMES as u64 {
        assert_eq!(emit(&test, format!("f{seq}")).await, accepted(seq));
    }
    let full = session_state(&test).await;
    assert_eq!(emit(&test, "one too many").await, rejected(Refusal::BufferFull));
    assert_eq!(session_state(&test).await, full, "a refusal changes nothing");

    ack(&test, "s-1", 1).await;
    assert_eq!(emit(&test, "fits").await, accepted(MAX_FRAMES as u64 + 1));
    let state = session_state(&test).await;
    assert_eq!(state.frames.len(), MAX_FRAMES);
    assert_eq!(state.frames.front().map(|f| f.seq), Some(2));
}

#[tokio::test(start_paused = true)]
async fn refusals_follow_their_precedence_and_change_nothing() {
    let full_and_exhausted =
        SessionState { assigned: MAX_SEQ, acked: MAX_SEQ - 100, frames: frames(MAX_SEQ - 99..=MAX_SEQ) };
    let test = with_session(full_and_exhausted.clone()).await;
    assert_eq!(emit(&test, "a".repeat(MAX_BODY_BYTES + 1)).await, rejected(Refusal::InvalidBody));
    assert_eq!(emit(&test, "valid").await, rejected(Refusal::BufferFull));
    assert_eq!(session_state(&test).await, full_and_exhausted);

    ack(&test, "s-1", (MAX_SEQ - 99) as i64).await;
    assert_eq!(emit(&test, "valid").await, rejected(Refusal::SequenceExhausted));
    assert_eq!(session_state(&test).await.assigned, MAX_SEQ);
}

#[tokio::test(start_paused = true)]
async fn the_last_sequence_is_exact_in_json() {
    let test = with_session(SessionState { assigned: MAX_SEQ - 1, acked: MAX_SEQ - 1, frames: VecDeque::new() }).await;
    assert_eq!(emit(&test, "last").await, accepted(MAX_SEQ));
    assert_eq!(emit(&test, "after").await, rejected(Refusal::SequenceExhausted));
    let json = test.state_json(&session()).await.unwrap();
    assert_eq!(json["frames"][0]["seq"].to_string(), "9007199254740991");
    assert_eq!(json["frames"][0]["seq"].as_f64(), Some(9_007_199_254_740_991.0), "exact as a double");
}

#[tokio::test(start_paused = true)]
async fn only_a_new_ack_within_the_assigned_range_of_this_stream_trims() {
    let test = with_session(SessionState { assigned: 5, acked: 2, frames: frames(3..=5) }).await;
    let unchanged = session_state(&test).await;
    for (stream, through) in [
        ("s-1", 2),                  // repeated
        ("s-1", 1),                  // backwards
        ("s-1", 0),                  // not positive
        ("s-1", -3),                 // negative
        ("s-1", 6),                  // above the assigned sequence
        ("s-1", MAX_SEQ as i64 + 1), // out of range
        ("s-2", 4),                  // another stream
    ] {
        ack(&test, stream, through).await;
        assert_eq!(session_state(&test).await, unchanged, "ack {stream} {through}");
    }
    ack(&test, "s-1", 4).await;
    assert_eq!(session_state(&test).await, SessionState { assigned: 5, acked: 4, frames: frames(5..=5) });
    ack(&test, "s-1", 5).await;
    assert_eq!(session_state(&test).await, SessionState { assigned: 5, acked: 5, frames: VecDeque::new() });
}

#[tokio::test(start_paused = true)]
async fn a_missing_reply_actor_rejects_the_commit_and_keeps_no_frame() {
    let test = with_session(SessionState::default()).await;
    let gone = Producer { id: "gone".into() };
    let enqueue = test.send(&session(), SessionMsg::Enqueue { body: "lost".into(), reply_to: gone }).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&enqueue).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert!(outcome.error.unwrap().contains("ws.producers/gone"), "the commit names the absent reply actor");
    assert_eq!(session_state(&test).await, SessionState::default(), "no frame, no sequence");
}

#[tokio::test(start_paused = true)]
async fn an_uncertain_send_repeats_its_id_and_appends_once() {
    let test = with_session(SessionState::default()).await;
    let id = MessageId::random();
    let enqueue = || SessionMsg::Enqueue { body: "once".into(), reply_to: producer() };
    let first = test.system().send_with(&session(), enqueue(), SendOptions::new().id(id.clone())).await.unwrap();
    assert!(matches!(first, SendResult::Accepted(_)));
    let again = test.system().send_with(&session(), enqueue(), SendOptions::new().id(id)).await.unwrap();
    assert!(matches!(again, SendResult::Duplicate(_)), "{again:?}");
    test.run_until_idle().await;
    assert_eq!(session_state(&test).await.frames, [Frame { seq: 1, body: "once".into() }]);
}

/// Sends an Enqueue whose body holds NUL from a turn, and keeps what the send returned.
struct NulSender;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NulSenderRef;

impl ActorRef for NulSenderRef {
    const TYPE: &'static str = "ws.nul-senders";
    type Message = NulSenderMsg;
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum NulSenderMsg {
    Try,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
struct NulSenderState {
    refused_as_invalid: bool,
}

impl Actor for NulSender {
    type Ref = NulSenderRef;
    type State = NulSenderState;

    async fn handle(&self, turn: &mut Turn<Self>, _: NulSenderMsg) -> HandlerResult {
        let enqueue = SessionMsg::Enqueue { body: "a\0b".into(), reply_to: producer() };
        let result = turn.send(&session(), enqueue);
        turn.state_mut().refused_as_invalid = matches!(result, Err(TurnError::Invalid(_)));
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn nul_is_refused_at_submission_before_the_session_can_answer() {
    let test = with_session(SessionState::default()).await;
    let enqueue = SessionMsg::Enqueue { body: "a\0b".into(), reply_to: producer() };
    let refused = test.system().send(&session(), enqueue).await;
    assert!(matches!(refused, Err(OpError::Invalid(_))), "{refused:?}");

    test.create(&NulSenderRef, &NulSenderState::default()).await;
    test.send(&NulSenderRef, NulSenderMsg::Try).await;
    test.run_until_idle().await;
    assert!(test.state::<_, NulSenderState>(&NulSenderRef).await.refused_as_invalid);

    assert!(test.pending(&session()).await.is_empty(), "no Enqueue was queued");
    assert!(test.pending(&producer()).await.is_empty(), "and so no reply");
    assert_eq!(session_state(&test).await, SessionState::default());
    assert_eq!(test.state::<_, ProducerState>(&producer()).await, ProducerState::new(session()));
}
