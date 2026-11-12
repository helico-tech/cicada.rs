//! The cause of a `Failed` (ADR 0028): the node decides it where it decides that a message fails, and hands it to the
//! store with the commit. A witness that watches every type records each `Failed` it is told. On virtual time, but for
//! the one test whose handler blocks its thread past its maximum turn, which does not move tokio's paused clock.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::json;

use super::common::*;
use crate::prelude::*;
use crate::store::{Backend, Message, OutcomeStatus, OutgoingMessage, StoreError, StoreResult, TurnOutcome, TypeDef};
use crate::testing::TestActors;
use crate::time::{self, MAX_DELAY, Timestamp};
use crate::{ActorSystem, Failed, FailureCause, Stopped, WatchTarget};

const MAX_TURN: Duration = Duration::from_millis(200);

/// `subject/{key}`; a key that starts with `bad` is no address of the type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
struct Subject(String);

impl From<Subject> for String {
    fn from(subject: Subject) -> Self {
        subject.0
    }
}

impl TryFrom<String> for Subject {
    type Error = String;

    fn try_from(key: String) -> Result<Self, String> {
        if key.starts_with("bad") { Err(format!("{key} is a bad key")) } else { Ok(Self(key)) }
    }
}

impl ActorRef for Subject {
    const TYPE: &'static str = "subject";
    type Message = SubjectMsg;
}

fn subject(key: &str) -> Subject {
    Subject(key.to_owned())
}

/// A type no node registers.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ghost {
    id: String,
}

impl ActorRef for Ghost {
    const TYPE: &'static str = "ghost";
    type Message = SubjectMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SubjectMsg {
    Work {
        n: i32,
    },
    Fail,
    Panic,
    Refuse,
    RefuseWithStoreError,
    /// Sleeps past the maximum turn: cut off.
    Hang,
    /// Blocks its thread past the maximum turn, then refuses.
    RefuseLate,
    /// Sends to an actor that does not exist: the commit is rejected.
    Orphan,
    /// Creates an actor of a type that is not registered: the node refuses the effect after the handler returned.
    CreateUnregistered,
    /// Creates `subject/child`: the node reads the registry after the handler returned.
    Spawn,
    /// Sends `Work` to `to` by address and passes an error on with `?`.
    Forward {
        to: String,
    },
    /// The same, wrapped with a context of its own.
    ForwardWrapped {
        to: String,
    },
    /// Returns a new state that cannot be encoded.
    Unencodable,
    /// Creates `subject/made` by address and passes an error on with `?`.
    CreateByAddress,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct SubjectState {
    #[serde(default)]
    n: i32,
    /// True: the state cannot be encoded.
    #[serde(default, serialize_with = "refuse_when_set")]
    unencodable: bool,
    /// True: decoding the state panics, outside the handler.
    #[serde(default, deserialize_with = "panic_when_set")]
    explosive: bool,
}

fn refuse_when_set<S: Serializer>(set: &bool, s: S) -> Result<S::Ok, S::Error> {
    if *set { Err(serde::ser::Error::custom("this state cannot be encoded")) } else { s.serialize_bool(false) }
}

fn panic_when_set<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let set = bool::deserialize(d)?;
    assert!(!set, "decoding the state exploded");
    Ok(false)
}

struct SubjectActor;

impl Actor for SubjectActor {
    type Ref = Subject;
    type State = SubjectState;
    const MAX_TURN: Option<Duration> = Some(MAX_TURN);

    async fn handle(&self, turn: &mut Turn<Self>, message: SubjectMsg) -> HandlerResult {
        match message {
            SubjectMsg::Work { n } => turn.state_mut().n += n,
            SubjectMsg::Fail => return Err(HandlerError::msg("boom")),
            SubjectMsg::Panic => panic!("boom"),
            SubjectMsg::Refuse => return Err(turn.fail("no")),
            SubjectMsg::RefuseWithStoreError => {
                return Err(HandlerError::permanent_from(StoreError::unavailable("down")));
            }
            SubjectMsg::Hang => tokio::time::sleep(Duration::from_secs(10)).await,
            SubjectMsg::RefuseLate => {
                std::thread::sleep(MAX_TURN * 2);
                return Err(turn.fail("no, and late"));
            }
            SubjectMsg::Orphan => {
                turn.send(&subject("absent"), SubjectMsg::Work { n: 1 })?;
            }
            SubjectMsg::CreateUnregistered => turn.create(&Ghost { id: "g".into() }, &SubjectState::default())?,
            SubjectMsg::Spawn => turn.create(&subject("child"), &SubjectState::default())?,
            SubjectMsg::Forward { to } => {
                let to = ActorAddress::parse(&to)?;
                turn.send_raw(&to, "Work", json!({"n": 1}), SendOptions::new()).await?;
            }
            SubjectMsg::ForwardWrapped { to } => {
                let to = ActorAddress::parse(&to)?;
                turn.send_raw(&to, "Work", json!({"n": 1}), SendOptions::new())
                    .await
                    .context("forwarding the work")
                    .map_err(HandlerError::new)?;
            }
            SubjectMsg::Unencodable => turn.state_mut().unencodable = true,
            SubjectMsg::CreateByAddress => {
                turn.create_raw(&ActorAddress::parse("subject/made")?, json!({})).await?;
            }
        }
        Ok(())
    }
}

/// `witness/_`: watches every type and keeps the `Failed` it is told.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Witness;

impl ActorRef for Witness {
    const TYPE: &'static str = "witness";
    type Message = WitnessMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
enum WitnessMsg {
    Failed(Failed),
    Stopped(Stopped),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Told {
    failed: Vec<Failed>,
}

struct WitnessActor;

impl Actor for WitnessActor {
    type Ref = Witness;
    type State = Told;

    async fn handle(&self, turn: &mut Turn<Self>, message: WitnessMsg) -> HandlerResult {
        if let WitnessMsg::Failed(failed) = message {
            turn.state_mut().failed.push(failed);
        }
        Ok(())
    }
}

/// A test with the subject `subject/s` and the witness, on virtual time from `start`.
async fn test_from(start: Option<Timestamp>) -> TestActors {
    let mut builder = TestActors::builder().with(|node| node.host(SubjectActor).host(WitnessActor));
    if let Some(start) = start {
        builder = builder.start_at(start);
    }
    let test = builder.build().await.unwrap();
    test.create(&Witness, &Told::default()).await;
    test.system().watch(&WatchTarget::Every, &addr(&Witness)).await.unwrap();
    test.create(&subject("s"), &SubjectState::default()).await;
    test
}

async fn test() -> TestActors {
    test_from(None).await
}

/// Runs every due turn, and returns the cause of the `Failed` the witness was told of `message`, with its error.
async fn told(test: &TestActors, message: &Message) -> (FailureCause, String) {
    test.run_until_idle().await;
    let told: Told = test.state(&Witness).await;
    let failed = told.failed.iter().find(|f| f.message_id == message.id).expect("the witness was told");
    (failed.cause, failed.error.clone())
}

async fn cause_of(test: &TestActors, message: SubjectMsg) -> FailureCause {
    let m = test.send(&subject("s"), message).await;
    told(test, &m).await.0
}

/// Scenario: A handler fails.
#[tokio::test(start_paused = true)]
async fn a_handler_that_fails_is_the_cause_handler() {
    let test = test().await;
    assert_eq!(cause_of(&test, SubjectMsg::Fail).await, FailureCause::Handler);
}

/// Scenario: A handler panics.
#[tokio::test(start_paused = true)]
async fn a_handler_that_panics_is_the_cause_handler() {
    let test = test().await;
    assert_eq!(cause_of(&test, SubjectMsg::Panic).await, FailureCause::Handler);
}

/// Scenario: A handler refuses a message.
#[tokio::test(start_paused = true)]
async fn a_handler_that_refuses_is_the_cause_refused() {
    let test = test().await;
    assert_eq!(cause_of(&test, SubjectMsg::Refuse).await, FailureCause::Refused);
}

/// Scenario: A handler refuses with a store error inside.
#[tokio::test(start_paused = true)]
async fn a_refusal_that_holds_a_store_error_is_the_cause_refused() {
    let test = test().await;
    assert_eq!(cause_of(&test, SubjectMsg::RefuseWithStoreError).await, FailureCause::Refused);
}

/// Scenario: A turn runs too long.
#[tokio::test(start_paused = true)]
async fn a_turn_cut_off_at_its_maximum_duration_is_the_cause_timeout() {
    let test = test().await;
    assert_eq!(cause_of(&test, SubjectMsg::Hang).await, FailureCause::Timeout);
}

/// Scenario: A turn that ran too long after a refusal. Real time: the handler blocks its thread.
#[tokio::test]
async fn a_refusal_after_the_turn_ran_past_its_maximum_duration_is_the_cause_timeout() {
    let test = test().await;
    assert_eq!(cause_of(&test, SubjectMsg::RefuseLate).await, FailureCause::Timeout);
}

/// Scenario: A message that cannot be decoded.
#[tokio::test(start_paused = true)]
async fn a_message_that_cannot_be_decoded_is_the_cause_message() {
    let test = test().await;
    let m = enqueue(test.store(), OutgoingMessage::new(addr(&subject("s")), "Work", json!({"n": "seven"}))).await;
    assert_eq!(told(&test, &m).await.0, FailureCause::Message);
    let m = enqueue(test.store(), OutgoingMessage::new(addr(&subject("s")), "Nope", json!({}))).await;
    assert_eq!(told(&test, &m).await.0, FailureCause::Message, "a type name the family does not have");
}

/// Scenario: A state that cannot be decoded.
#[tokio::test(start_paused = true)]
async fn a_state_that_cannot_be_decoded_is_the_cause_state() {
    let test = test().await;
    test.store().create(&addr(&subject("x")), json!({"n": "seven"})).await.unwrap();
    let m = test.send(&subject("x"), SubjectMsg::Work { n: 1 }).await;
    assert_eq!(told(&test, &m).await.0, FailureCause::State);
}

/// Scenario: An address the type cannot read.
#[tokio::test(start_paused = true)]
async fn an_address_the_type_cannot_read_is_the_cause_state() {
    let test = test().await;
    let bad = ActorAddress::parse("subject/bad1").unwrap();
    test.store().create(&bad, json!({})).await.unwrap();
    let m = enqueue(test.store(), OutgoingMessage::new(bad, "Work", json!({"n": 1}))).await;
    let (cause, error) = told(&test, &m).await;
    assert_eq!(cause, FailureCause::State, "{error}");
    assert!(error.contains("bad1 is a bad key"), "{error}");
}

/// Scenario: A commit that is rejected.
#[tokio::test(start_paused = true)]
async fn a_send_to_an_actor_that_does_not_exist_is_the_cause_commit() {
    let test = test().await;
    assert_eq!(cause_of(&test, SubjectMsg::Orphan).await, FailureCause::Commit);
}

/// Scenario: An effect that is refused after the handler returned.
#[tokio::test(start_paused = true)]
async fn an_effect_the_node_refuses_after_the_handler_is_the_cause_commit() {
    let test = test().await;
    assert_eq!(cause_of(&test, SubjectMsg::CreateUnregistered).await, FailureCause::Commit);
}

/// Scenario: A new state that cannot be stored.
#[tokio::test(start_paused = true)]
async fn a_new_state_that_cannot_be_encoded_is_the_cause_handler() {
    let test = test().await;
    let m = test.send(&subject("s"), SubjectMsg::Unencodable).await;
    let (cause, error) = told(&test, &m).await;
    assert_eq!(cause, FailureCause::Handler);
    assert!(error.starts_with("State cannot be encoded"), "{error}");
}

/// Scenario: A panic outside the handler: decoding the state panics.
#[tokio::test(start_paused = true)]
async fn a_panic_outside_the_handler_is_the_cause_node() {
    let test = test().await;
    test.store().create(&addr(&subject("x")), json!({"explosive": true})).await.unwrap();
    let m = test.send(&subject("x"), SubjectMsg::Work { n: 1 }).await;
    let (cause, error) = told(&test, &m).await;
    assert_eq!(cause, FailureCause::Node);
    assert!(error.contains("before its handler ran: panic: decoding the state exploded"), "{error}");
}

/// Scenario: A retry the store cannot schedule. Near the largest instant the store refuses the time of the retry.
#[tokio::test(start_paused = true)]
async fn a_retry_the_store_cannot_schedule_is_the_cause_handler() {
    let test = test_from(Some(time::parse_instant("9500-01-01T00:00:00Z").unwrap())).await;
    let policy = RetryPolicy::new(5, Backoff::fixed(MAX_DELAY).unwrap()).unwrap();
    let m = test.send_with(&subject("s"), SubjectMsg::Fail, SendOptions::new().retry(policy)).await;
    let (cause, error) = told(&test, &m).await;
    assert_eq!(cause, FailureCause::Handler);
    assert!(error.starts_with("boom (commit refused: "), "{error}");
}

/// Scenario: An observer is not touched.
#[tokio::test(start_paused = true)]
async fn an_observer_is_told_the_failure_as_before() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    let test = TestActors::builder()
        .with(move |node| {
            node.host(SubjectActor)
                .observe(OnEnd::new(move |_, end| record.lock().unwrap().push(end.outcome().clone())))
        })
        .build()
        .await
        .unwrap();
    test.create(&subject("s"), &SubjectState::default()).await;
    let m = test.send(&subject("s"), SubjectMsg::Refuse).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Failed);
    assert_eq!(*seen.lock().unwrap(), [TurnOutcome::Failed("no".into())]);
}

// ---- a registry that cannot be read ----

/// Reading the TypeRegistry fails while `failures` is positive (counting down).
#[derive(Default)]
struct FlakyRegistry {
    failures: AtomicI32,
}

#[async_trait]
impl Faults for FlakyRegistry {
    async fn list_types(&self, inner: &dyn Backend) -> StoreResult<Vec<TypeDef>> {
        if self.failures.fetch_sub(1, Ordering::SeqCst) > 0 {
            return Err(StoreError::unavailable("registry unreachable"));
        }
        self.failures.store(0, Ordering::SeqCst);
        inner.list_types().await
    }
}

/// A node on a store whose registry fails to be read when the test says so, with the subject `subject/s` and the
/// witness.
struct Flaky {
    memory: crate::Store,
    store: Arc<Faulty<FlakyRegistry>>,
    system: ActorSystem,
}

impl Flaky {
    async fn new() -> Self {
        let memory = virtual_store();
        let store = Faulty::new(memory.clone(), FlakyRegistry::default());
        let system = node(store.clone(), "n1").host(SubjectActor).host(WitnessActor).build();
        system.initialize().await.unwrap();
        system.create(&Witness, &Told::default()).await.unwrap();
        system.watch(&WatchTarget::Every, &addr(&Witness)).await.unwrap();
        system.create(&subject("s"), &SubjectState::default()).await.unwrap();
        system.create(&subject("t"), &SubjectState::default()).await.unwrap();
        Self { memory, store, system }
    }

    /// Sends `message` to `subject/s`, runs its turn while every read of the registry fails, then runs the rest, and
    /// returns the cause and the error the witness was told.
    async fn fail(&self, message: SubjectMsg) -> (FailureCause, String) {
        let m = accepted(self.system.send(&subject("s"), message).await.unwrap());
        self.store.faults.failures.store(i32::MAX, Ordering::SeqCst);
        assert_eq!(self.system.run_due_turns(1).await.unwrap(), 1);
        self.store.faults.failures.store(0, Ordering::SeqCst);
        while self.system.run_due_turns(10).await.unwrap() > 0 {}
        assert_eq!(outcome_of(&self.memory, &m.id).await.unwrap().status, OutcomeStatus::Failed);
        let told: Told = state_of(&self.memory, &addr(&Witness)).await;
        let failed = told.failed.iter().find(|f| f.message_id == m.id).expect("the witness was told");
        (failed.cause, failed.error.clone())
    }
}

/// Scenario: A read of the store that fails after the handler returned.
#[tokio::test(start_paused = true)]
async fn a_read_of_the_registry_that_fails_after_the_handler_returned_is_the_cause_node() {
    let flaky = Flaky::new().await;
    let (cause, error) = flaky.fail(SubjectMsg::Spawn).await;
    assert_eq!(cause, FailureCause::Node);
    assert!(error.contains("after its handler ended") && error.contains("registry unreachable"), "{error}");
}

/// Scenario: A handler passes a failed read of the store on, with `?` and wrapped with a context of its own.
#[tokio::test(start_paused = true)]
async fn a_failed_read_of_the_store_that_the_handler_passes_on_is_the_cause_node() {
    let flaky = Flaky::new().await;
    let (cause, error) = flaky.fail(SubjectMsg::Forward { to: "subject/t".into() }).await;
    assert_eq!(cause, FailureCause::Node, "passed on with ?");
    assert!(error.contains("registry unreachable"), "{error}");
    let (cause, error) = flaky.fail(SubjectMsg::ForwardWrapped { to: "subject/t".into() }).await;
    assert_eq!(cause, FailureCause::Node, "wrapped");
    assert!(error.starts_with("forwarding the work: "), "{error}");
    let (cause, error) = flaky.fail(SubjectMsg::CreateByAddress).await;
    assert_eq!(cause, FailureCause::Node, "a create by address whose read of the registry fails: TurnError::Store");
    assert!(error.contains("registry unreachable"), "{error}");
    // With a store that works, a send the registry refuses is the fault of the handler that passes it on.
    let m = accepted(flaky.system.send(&subject("s"), SubjectMsg::Forward { to: "ghost/g".into() }).await.unwrap());
    while flaky.system.run_due_turns(10).await.unwrap() > 0 {}
    let told: Told = state_of(&flaky.memory, &addr(&Witness)).await;
    let failed = told.failed.iter().find(|f| f.message_id == m.id).expect("told");
    assert_eq!(failed.cause, FailureCause::Handler, "a send the registry refuses: {}", failed.error);
}
