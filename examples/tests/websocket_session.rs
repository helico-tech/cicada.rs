//! Example 20: the websocket session's protocol and its host bridge, on virtual time.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::turn::TurnError;
use cicada::{ActorChange, OpError, SendResult};
use cicada_examples::websocket_session::bridge::{
    ClientMessage, Closed, IO_TIMEOUT, Inbound, POLL_INTERVAL, ServerMessage, SessionStore, serve,
};
use cicada_examples::websocket_session::{
    Answer, Frame, MAX_BODY_BYTES, MAX_FRAMES, MAX_SEQ, Pending, Producer, ProducerActor, ProducerMsg, ProducerState,
    Refusal, Session, SessionActor, SessionMsg, SessionState, adapter, emit_and_wait,
};
use cicada_test::TestActors;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;

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

// ---- The host bridge, through a bounded in-memory socket ----

/// The test's end of a socket: what the client writes to the host, and what the host wrote.
struct Client {
    to_host: Option<mpsc::Sender<Result<Inbound, String>>>,
    from_host: mpsc::Receiver<String>,
}

impl Client {
    /// The next message the host wrote, or `None` once the host's half is gone.
    async fn next(&mut self) -> Option<ServerMessage> {
        let text = timeout(Duration::from_secs(30), self.from_host.next()).await.expect("a message within 30 s")?;
        Some(serde_json::from_str(&text).expect("server messages are JSON"))
    }

    async fn expect(&mut self, messages: &[ServerMessage]) {
        for want in messages {
            assert_eq!(self.next().await.as_ref(), Some(want));
        }
    }

    /// Writes `inbound` to the host; false when the host no longer reads.
    async fn send(&mut self, inbound: Inbound) -> bool {
        self.to_host.as_mut().expect("open").send(Ok(inbound)).await.is_ok()
    }

    async fn ack(&mut self, through: i64) {
        let ack = ClientMessage::Ack { stream: "s-1".into(), through };
        assert!(self.send(Inbound::Text(serde_json::to_string(&ack).unwrap())).await, "the host reads");
    }

    /// Closes the client's direction, as a client closing its socket.
    fn close(&mut self) {
        self.to_host = None;
    }
}

/// One connection served by [`serve`] in its own task.
struct Connection {
    client: Client,
    shutdown: CancellationToken,
    task: JoinHandle<Closed>,
}

impl Connection {
    /// Why the connection ended, once its task has ended.
    async fn closed(&mut self) -> Closed {
        timeout(Duration::from_secs(60), &mut self.task).await.expect("the connection ends").unwrap()
    }
}

/// Serves the session `s-1` from `store` on a socket whose directions each buffer `capacity` messages.
fn connect<S: SessionStore + Clone + 'static>(store: &S, capacity: usize) -> Connection {
    let (host_tx, from_host) = mpsc::channel::<String>(capacity);
    let (to_host, mut host_rx) = mpsc::channel::<Result<Inbound, String>>(capacity);
    let shutdown = CancellationToken::new();
    let task = tokio::spawn({
        let (store, shutdown) = (store.clone(), shutdown.clone());
        async move {
            let mut host_tx = host_tx.sink_map_err(|e| e.to_string());
            serve(&store, &session(), &mut host_tx, &mut host_rx, shutdown.cancelled()).await
        }
    });
    Connection { client: Client { to_host: Some(to_host), from_host }, shutdown, task }
}

fn hello(acked: u64) -> ServerMessage {
    ServerMessage::Hello { stream: "s-1".into(), acked }
}

fn frame(seq: u64) -> ServerMessage {
    ServerMessage::Frame { stream: "s-1".into(), seq, body: format!("f{seq}") }
}

/// Emits the frames `range`, with the bodies of [`frames`].
async fn emit_frames(test: &TestActors, range: std::ops::RangeInclusive<u64>) {
    for seq in range {
        assert_eq!(emit(test, format!("f{seq}")).await, accepted(seq));
    }
}

/// Runs turns, letting the connections work in between, until the session's state passes `check`.
async fn until(test: &TestActors, check: impl Fn(&SessionState) -> bool) -> SessionState {
    for _ in 0..100 {
        test.run_until_idle().await;
        let state = session_state(test).await;
        if check(&state) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the session never reached the expected state: {:?}", session_state(test).await);
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Fault {
    #[default]
    None,
    Fail,
    Stall,
}

#[derive(Default)]
enum Feed {
    #[default]
    Real,
    /// No hint ever arrives.
    Silent,
    /// The feed ends at once.
    Ended,
    /// The test sends the hints.
    Manual(Option<mpsc::UnboundedReceiver<ActorChange>>),
}

#[derive(Default)]
struct Faults {
    read: Fault,
    ack: Fault,
    feed: Feed,
    reads: usize,
}

/// The node's public operations with the faults a test chooses; the example itself has no fault hook.
#[derive(Clone)]
struct Faulty {
    system: ActorSystem,
    faults: Arc<Mutex<Faults>>,
}

impl Faulty {
    fn new(test: &TestActors, faults: Faults) -> Self {
        Self { system: test.system().clone(), faults: Arc::new(Mutex::new(faults)) }
    }

    fn reads(&self) -> usize {
        self.faults.lock().unwrap().reads
    }
}

impl SessionStore for Faulty {
    async fn read_session(&self, session: &Session) -> Result<Option<SessionState>, String> {
        let fault = {
            let mut faults = self.faults.lock().unwrap();
            faults.reads += 1;
            faults.read
        };
        match fault {
            Fault::None => self.system.read_session(session).await,
            Fault::Fail => Err("the read failed".into()),
            Fault::Stall => std::future::pending().await,
        }
    }

    async fn send_ack(&self, session: &Session, stream: String, through: i64) -> Result<(), String> {
        let fault = self.faults.lock().unwrap().ack;
        match fault {
            Fault::None => self.system.send_ack(session, stream, through).await,
            Fault::Fail => Err("the send failed".into()),
            Fault::Stall => std::future::pending().await,
        }
    }

    fn follow(&self, session: &Session) -> BoxStream<'static, ActorChange> {
        match &mut self.faults.lock().unwrap().feed {
            Feed::Real => self.system.follow(session),
            Feed::Silent => futures::stream::pending().boxed(),
            Feed::Ended => futures::stream::empty().boxed(),
            Feed::Manual(hints) => hints.take().expect("one connection").boxed(),
        }
    }
}

#[test]
fn the_socket_messages_are_tagged_json() {
    let hello = serde_json::to_string(&hello(0)).unwrap();
    assert_eq!(hello, r#"{"type":"Hello","stream":"s-1","acked":0}"#);
    let last = ServerMessage::Frame { stream: "s-1".into(), seq: MAX_SEQ, body: "x".into() };
    let last = serde_json::to_string(&last).unwrap();
    assert_eq!(last, r#"{"type":"Frame","stream":"s-1","seq":9007199254740991,"body":"x"}"#);
    let ack: ClientMessage = serde_json::from_str(r#"{"type":"Ack","stream":"s-1","through":-3}"#).unwrap();
    assert_eq!(ack, ClientMessage::Ack { stream: "s-1".into(), through: -3 });
    for invalid in [
        r#"{"type":"Ack","stream":"s-1","through":1.5}"#,
        r#"{"type":"Ack","stream":"s-1","through":9223372036854775808}"#,
        r#"{"type":"Ack","stream":"s-1"}"#,
        r#"{"type":"Hello","stream":"s-1","acked":0}"#,
    ] {
        assert!(serde_json::from_str::<ClientMessage>(invalid).is_err(), "{invalid}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_socket_gets_hello_then_the_retained_frames_then_new_output() {
    let test = with_session(SessionState::default()).await;
    emit_frames(&test, 1..=2).await;
    let mut connection = connect(test.system(), 8);
    connection.client.expect(&[hello(0), frame(1), frame(2)]).await;

    let before = Instant::now();
    emit_frames(&test, 3..=3).await;
    connection.client.expect(&[frame(3)]).await;
    assert!(before.elapsed() < POLL_INTERVAL, "the change hint, not the periodic read");
    connection.client.ack(3).await;
    let state = until(&test, |s| s.acked == 3).await;
    assert!(state.frames.is_empty(), "the ack trimmed them");

    connection.client.close();
    assert_eq!(connection.closed().await, Closed::ClientClosed);
    assert_eq!(connection.client.next().await, None, "the host's half is gone");
}

#[tokio::test(start_paused = true)]
async fn a_disconnect_before_the_ack_replays_from_the_durable_ack() {
    let test = with_session(SessionState::default()).await;
    emit_frames(&test, 1..=2).await;
    let mut first = connect(test.system(), 8);
    first.client.expect(&[hello(0), frame(1), frame(2)]).await;
    first.client.ack(1).await;
    until(&test, |s| s.acked == 1).await;
    first.client.close();
    assert_eq!(first.closed().await, Closed::ClientClosed);
    assert_eq!(session_state(&test).await.frames, frames(2..=2), "frame 2 was written, never acknowledged");

    let mut second = connect(test.system(), 8);
    second.client.expect(&[hello(1), frame(2)]).await;
}

#[tokio::test(start_paused = true)]
async fn a_lost_ack_closes_and_the_reconnect_replays() {
    for (fault, why) in
        [(Fault::Fail, Closed::Store("the send failed".into())), (Fault::Stall, Closed::TimedOut("ack"))]
    {
        let test = with_session(SessionState::default()).await;
        emit_frames(&test, 1..=1).await;
        let faulty = Faulty::new(&test, Faults { ack: fault, ..Faults::default() });
        let mut lost = connect(&faulty, 8);
        lost.client.expect(&[hello(0), frame(1)]).await;
        lost.client.ack(1).await;
        let start = Instant::now();
        assert_eq!(lost.closed().await, why);
        if fault == Fault::Stall {
            assert!(start.elapsed() >= IO_TIMEOUT);
        }
        assert_eq!(lost.client.next().await, None, "the writer stopped too");
        assert_eq!(session_state(&test).await, SessionState { assigned: 1, acked: 0, frames: frames(1..=1) });

        let mut again = connect(test.system(), 8);
        again.client.expect(&[hello(0), frame(1)]).await;
        again.client.ack(1).await;
        until(&test, |s| s.acked == 1 && s.frames.is_empty()).await;
    }
}

#[tokio::test(start_paused = true)]
async fn another_system_on_the_same_store_serves_the_reconnect() {
    let test = with_session(SessionState::default()).await;
    emit_frames(&test, 1..=1).await;
    let mut first = connect(test.system(), 8);
    first.client.expect(&[hello(0), frame(1)]).await;
    first.client.close();
    assert_eq!(first.closed().await, Closed::ClientClosed);

    // A host that hosts no actor type: it reads, follows and sends through the shared store.
    let other = ActorSystem::builder(test.store().clone()).build();
    let mut second = connect(&other, 8);
    second.client.expect(&[hello(0), frame(1)]).await;
    let before = Instant::now();
    emit_frames(&test, 2..=2).await;
    second.client.expect(&[frame(2)]).await;
    assert!(before.elapsed() < POLL_INTERVAL, "a hint of the other system's commit");
    second.client.ack(2).await;
    until(&test, |s| s.acked == 2 && s.frames.is_empty()).await;
}

#[tokio::test(start_paused = true)]
async fn overlapping_sockets_of_one_client_get_the_output_and_stale_acks_are_harmless() {
    let test = with_session(SessionState::default()).await;
    emit_frames(&test, 1..=2).await;
    let mut old = connect(test.system(), 8);
    let mut new = connect(test.system(), 8);
    old.client.expect(&[hello(0), frame(1), frame(2)]).await;
    new.client.expect(&[hello(0), frame(1), frame(2)]).await;
    new.client.ack(2).await;
    until(&test, |s| s.acked == 2).await;
    old.client.ack(1).await;
    test.run_until_idle().await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    test.run_until_idle().await;
    assert_eq!(session_state(&test).await, SessionState { assigned: 2, acked: 2, frames: VecDeque::new() });

    emit_frames(&test, 3..=3).await;
    old.client.expect(&[frame(3)]).await;
    new.client.expect(&[frame(3)]).await;
}

#[tokio::test(start_paused = true)]
async fn the_periodic_read_recovers_missed_hints() {
    let test = with_session(SessionState::default()).await;
    let faulty = Faulty::new(&test, Faults { feed: Feed::Silent, ..Faults::default() });
    let mut connection = connect(&faulty, 8);
    connection.client.expect(&[hello(0)]).await;
    let before = Instant::now();
    emit_frames(&test, 1..=1).await;
    connection.client.expect(&[frame(1)]).await;
    assert!(before.elapsed() <= POLL_INTERVAL);
}

#[tokio::test(start_paused = true)]
async fn a_resync_reads_at_once_and_hints_of_other_addresses_are_skipped() {
    let test = with_session(SessionState::default()).await;
    let (hints, feed) = mpsc::unbounded();
    let faulty = Faulty::new(&test, Faults { feed: Feed::Manual(Some(feed)), ..Faults::default() });
    let mut connection = connect(&faulty, 8);
    let opened = Instant::now();
    connection.client.expect(&[hello(0)]).await;
    emit_frames(&test, 1..=1).await;
    hints.unbounded_send(ActorChange::Resync).unwrap();
    connection.client.expect(&[frame(1)]).await;
    assert!(opened.elapsed() < POLL_INTERVAL, "the resync, not the periodic read");

    // `ws.sessions/s-10` is under the literal prefix `ws.sessions/s-1`: its hints cause no read.
    let real = Faulty::new(&test, Faults::default());
    let mut follower = connect(&real, 8);
    follower.client.expect(&[hello(0), frame(1)]).await;
    let reads = real.reads();
    let neighbour = Session { stream: "s-10".into() };
    test.create(&neighbour, &SessionState::default()).await;
    test.send(&neighbour, SessionMsg::Enqueue { body: "other".into(), reply_to: producer() }).await;
    test.run_until_idle().await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(real.reads(), reads);
    test.send(&session(), SessionMsg::Ack { stream: "s-1".into(), through: 1 }).await;
    test.run_until_idle().await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(real.reads(), reads + 1, "a hint of its own address");
}

#[tokio::test(start_paused = true)]
async fn a_stalled_write_closes_after_the_timeout_and_keeps_the_output() {
    let test = with_session(SessionState::default()).await;
    emit_frames(&test, 1..=2).await;
    // One message fits; the client reads nothing.
    let mut connection = connect(test.system(), 0);
    let start = Instant::now();
    assert_eq!(connection.closed().await, Closed::TimedOut("write"));
    assert!(start.elapsed() >= IO_TIMEOUT);
    assert_eq!(session_state(&test).await, SessionState { assigned: 2, acked: 0, frames: frames(1..=2) });
    connection.client.expect(&[hello(0)]).await;
    assert_eq!(connection.client.next().await, None);
}

#[tokio::test(start_paused = true)]
async fn a_failed_or_stalled_read_closes_before_hello() {
    let test = with_session(SessionState::default()).await;
    for (fault, why) in
        [(Fault::Fail, Closed::Store("the read failed".into())), (Fault::Stall, Closed::TimedOut("read"))]
    {
        let faulty = Faulty::new(&test, Faults { read: fault, ..Faults::default() });
        let mut connection = connect(&faulty, 8);
        assert_eq!(connection.closed().await, why);
        assert_eq!(connection.client.next().await, None);
    }
}

#[tokio::test(start_paused = true)]
async fn a_missing_or_deleted_session_closes_the_socket() {
    let test = harness().await;
    let mut missing = connect(test.system(), 8);
    assert_eq!(missing.closed().await, Closed::SessionMissing);

    test.create(&session(), &SessionState::default()).await;
    let mut connection = connect(test.system(), 8);
    connection.client.expect(&[hello(0)]).await;
    test.system().delete_actor(&session().address().unwrap()).await.unwrap();
    assert_eq!(connection.closed().await, Closed::SessionMissing);
}

#[tokio::test(start_paused = true)]
async fn a_key_used_for_a_new_stream_closes_the_socket() {
    let test = with_session(SessionState::default()).await;
    emit_frames(&test, 1..=2).await;
    let faulty = Faulty::new(&test, Faults { feed: Feed::Silent, ..Faults::default() });
    let mut connection = connect(&faulty, 8);
    connection.client.expect(&[hello(0), frame(1), frame(2)]).await;
    // Against the convention: deleted and created again at the same key, between two reads.
    test.system().delete_actor(&session().address().unwrap()).await.unwrap();
    test.create(&session(), &SessionState::default()).await;
    assert_eq!(connection.closed().await, Closed::StreamReset);
}

#[tokio::test(start_paused = true)]
async fn the_end_of_the_feed_closes_the_socket() {
    let test = with_session(SessionState::default()).await;
    let faulty = Faulty::new(&test, Faults { feed: Feed::Ended, ..Faults::default() });
    assert_eq!(connect(&faulty, 8).closed().await, Closed::FeedEnded);

    let mut connection = connect(test.system(), 8);
    connection.client.expect(&[hello(0)]).await;
    test.store().close().await;
    let closed = connection.closed().await;
    assert!(matches!(closed, Closed::FeedEnded | Closed::Store(_)), "{closed:?}");
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_both_halves() {
    let test = with_session(SessionState::default()).await;
    emit_frames(&test, 1..=1).await;
    let mut connection = connect(test.system(), 8);
    connection.client.expect(&[hello(0), frame(1)]).await;
    connection.shutdown.cancel();
    assert_eq!(connection.closed().await, Closed::Shutdown);

    emit_frames(&test, 2..=2).await;
    tokio::time::sleep(POLL_INTERVAL * 2).await;
    assert_eq!(connection.client.next().await, None, "nothing writes any more");
    let ack = ClientMessage::Ack { stream: "s-1".into(), through: 1 };
    assert!(!connection.client.send(Inbound::Text(serde_json::to_string(&ack).unwrap())).await, "nothing reads");
    assert!(test.pending(&session()).await.is_empty());
    assert_eq!(session_state(&test).await.acked, 0);
}

#[tokio::test(start_paused = true)]
async fn invalid_input_closes_the_socket() {
    let test = with_session(SessionState::default()).await;
    for input in [
        Inbound::Binary,
        Inbound::Text("not json".into()),
        Inbound::Text(r#"{"type":"Ack","stream":"s-1","through":1.5}"#.into()),
    ] {
        let mut connection = connect(test.system(), 8);
        connection.client.expect(&[hello(0)]).await;
        assert!(connection.client.send(input).await);
        assert!(matches!(connection.closed().await, Closed::InvalidInput(_)));
    }
}

// ---- The axum route, over a real websocket ----

#[tokio::test(start_paused = true)]
async fn the_axum_route_serves_the_session_over_a_websocket() {
    use tokio_tungstenite::tungstenite::Message;

    let test = with_session(SessionState::default()).await;
    emit_frames(&test, 1..=1).await;
    let shutdown = CancellationToken::new();
    let app = adapter::router(test.system().clone(), shutdown.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/sessions/s-1/socket", listener.local_addr().unwrap());
    let server = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { axum::serve(listener, app).with_graceful_shutdown(shutdown.cancelled_owned()).await }
    });

    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut receive = async || match socket.next().await.unwrap().unwrap() {
        Message::Text(text) => serde_json::from_str::<ServerMessage>(&text).unwrap(),
        other => panic!("a text message, not {other:?}"),
    };
    assert_eq!(receive().await, hello(0));
    assert_eq!(receive().await, frame(1));
    let ack = serde_json::to_string(&ClientMessage::Ack { stream: "s-1".into(), through: 1 }).unwrap();
    socket.send(Message::Text(ack.into())).await.unwrap();
    until(&test, |s| s.acked == 1 && s.frames.is_empty()).await;

    socket.send(Message::Binary(vec![1, 2, 3].into())).await.unwrap();
    match socket.next().await.unwrap().unwrap() {
        Message::Close(Some(close)) => assert_eq!(u16::from(close.code), 1003, "unsupported data"),
        other => panic!("a close frame, not {other:?}"),
    }
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

// ---- The demo's emit, from outside an actor ----

const POLL: Duration = Duration::from_millis(200);

/// Starts [`emit_and_wait`] in its own task; the test runs the turns.
fn emit_in_background(test: &TestActors, body: &str) -> JoinHandle<Result<Answer, String>> {
    let (system, body) = (test.system().clone(), body.to_owned());
    tokio::spawn(async move { emit_and_wait(&system, &producer(), body, POLL, Duration::from_secs(30)).await })
}

/// Waits, without moving time, until the background task's Emit is in the producer's mailbox.
async fn emitted(test: &TestActors) {
    while test.pending(&producer()).await.is_empty() {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_demo_emit_waits_for_the_answer_to_its_own_emit_when_turns_run_late() {
    let test = with_session(SessionState::default()).await;
    assert_eq!(emit(&test, "one").await, accepted(1));

    // Nothing pending and an earlier answer in state: neither is the answer to this Emit.
    let waiting = emit_in_background(&test, "two");
    tokio::time::sleep(POLL * 5).await;
    assert!(!waiting.is_finished(), "the Emit has not run");
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the producer's Emit");
    tokio::time::sleep(POLL * 5).await;
    assert!(!waiting.is_finished(), "the session has not answered");
    test.run_until_idle().await;
    assert_eq!(waiting.await.unwrap(), Ok(accepted(2)));

    // A request that finishes between two reads is still seen.
    let fast = emit_in_background(&test, "three");
    emitted(&test).await;
    test.run_until_idle().await;
    assert_eq!(fast.await.unwrap(), Ok(accepted(3)));
}

#[tokio::test(start_paused = true)]
async fn the_demo_emit_reports_a_refused_or_failed_emit_and_the_limit() {
    let test = harness().await;
    let refused = emit_and_wait(test.system(), &producer(), "x", POLL, Duration::from_secs(30)).await;
    assert!(refused.unwrap_err().starts_with("The emit was refused: NoSuchInstance"));

    test.create(&session(), &SessionState::default()).await;
    let busy = Pending { request: MessageId::random(), body: "earlier".into() };
    test.create(&producer(), &ProducerState { pending: Some(busy), ..ProducerState::new(session()) }).await;
    let failed = emit_in_background(&test, "x");
    emitted(&test).await;
    test.run_until_idle().await;
    let failed = failed.await.unwrap().unwrap_err();
    assert!(failed.starts_with("The emit ended Failed") && failed.contains("A request is pending"), "{failed}");

    let unanswered = emit_and_wait(test.system(), &producer(), "x", POLL, Duration::from_secs(2)).await;
    assert_eq!(unanswered, Err("No answer within 2s".into()));
}
