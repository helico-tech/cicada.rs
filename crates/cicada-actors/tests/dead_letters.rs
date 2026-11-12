//! Dead letters, on virtual time with `TestActors` (ADR 0028): told of failed messages by a watch, with no timer; a
//! capacity; a redrive, once; and a discard.

mod common;

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::{Message, MessageStatus, OutcomeStatus, SqliteOptions};
use cicada::{Failed, FailureCause, Store, WatchTarget};
use cicada_actors::{DeadLetter, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// `billing.invoices/{id}` and `shipping/{id}`: actors whose messages fail.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Invoice {
    id: String,
}

impl ActorRef for Invoice {
    const TYPE: &'static str = "billing.invoices";
    type Message = Failing;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Shipment {
    id: String,
}

impl ActorRef for Shipment {
    const TYPE: &'static str = "shipping";
    type Message = Failing;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Failing {
    Fail,
    /// Fails too, with a payload `{"v": …}` of any depth.
    Deep {
        v: Value,
    },
}

struct InvoiceActor;

impl Actor for InvoiceActor {
    type Ref = Invoice;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: Failing) -> HandlerResult {
        Err(HandlerError::msg("declined"))
    }
}

struct ShipmentActor;

impl Actor for ShipmentActor {
    type Ref = Shipment;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: Failing) -> HandlerResult {
        Err(HandlerError::msg("lost"))
    }
}

fn letters_actor() -> DeadLetters {
    DeadLetters::default()
}

/// A node with dead letters of `capacity` watching `target`, the worker `worker/w`, the invoice and the shipment types,
/// and the witness, which watches the dead letters.
async fn node(capacity: usize, target: &str) -> TestActors {
    let test = TestActors::builder()
        .with(move |node| {
            node.host(DeadLettersActor::with_capacity(capacity).unwrap())
                .host(WorkerActor)
                .host(InvoiceActor)
                .host(ShipmentActor)
                .host(WitnessActor)
        })
        .build()
        .await
        .unwrap();
    test.create(&letters_actor(), &DeadLettersState::default()).await;
    test.system().watch(&WatchTarget::parse(target).unwrap(), &addr(&letters_actor())).await.unwrap();
    test.create(&worker("w"), &WorkerState::default()).await;
    witness(&test, "cicada.dead-letters").await;
    test
}

async fn letters(test: &TestActors) -> std::collections::BTreeMap<MessageId, DeadLetter> {
    test.state::<_, DeadLettersState>(&letters_actor()).await.letters
}

async fn letter(test: &TestActors, id: &MessageId) -> DeadLetter {
    letters(test).await.get(id).cloned().unwrap_or_else(|| panic!("a letter for {id}"))
}

async fn fail_work(test: &TestActors) -> cicada::store::Message {
    let m = test.send_with(&worker("w"), WorkerMsg::Fail, SendOptions::new().correlation_id("c-1")).await;
    test.run_until_idle().await;
    m
}

async fn to_letters(test: &TestActors, message: DeadLettersMsg) -> cicada::store::Message {
    let m = test.send(&letters_actor(), message).await;
    test.run_until_idle().await;
    m
}

/// Scenario: Every failure.
#[tokio::test(start_paused = true)]
async fn a_watch_on_every_type_records_a_letter_for_every_failed_message() {
    let test = node(1_000, "*").await;
    test.system().pause_mailbox(&addr(&letters_actor())).await.unwrap();
    let failed = fail_work(&test).await;
    let notification = test.pending(&letters_actor()).await[0].clone();
    test.system().resume_mailbox(&addr(&letters_actor())).await.unwrap();
    test.run_until_idle().await;
    let letter = letter(&test, &failed.id).await;
    assert_eq!(
        letter,
        DeadLetter {
            actor: addr(&worker("w")),
            message_type: "Fail".into(),
            payload: json!({}),
            correlation_id: Some("c-1".into()),
            from: None,
            error: "boom".into(),
            cause: FailureCause::Handler,
            attempts: 1,
            recorded_at: test.now(),
            seq: notification.seq,
            redriven_as: None,
            redrive_error: None,
        }
    );
    let refused = test.send(&worker("w"), WorkerMsg::Refuse).await;
    test.run_until_idle().await;
    assert_eq!(self::letter(&test, &refused.id).await.cause, FailureCause::Refused);
}

/// Scenario: One type.
#[tokio::test(start_paused = true)]
async fn a_watch_on_one_type_records_the_failures_of_that_type_only() {
    let test = node(1_000, "billing").await;
    test.create(&Invoice { id: "a".into() }, &NoState {}).await;
    test.create(&Shipment { id: "b".into() }, &NoState {}).await;
    let invoice = test.send(&Invoice { id: "a".into() }, Failing::Fail).await;
    let shipment = test.send(&Shipment { id: "b".into() }, Failing::Fail).await;
    test.run_until_idle().await;
    let letters = letters(&test).await;
    assert_eq!(letters.keys().collect::<Vec<_>>(), [&invoice.id]);
    assert_eq!(letters[&invoice.id].error, "declined");
    assert_eq!(status(&test, &shipment).await, Some(OutcomeStatus::Failed));
}

/// Scenario: A failure of its own: a redrive to an actor that is gone.
#[tokio::test(start_paused = true)]
async fn a_failure_of_its_own_records_no_letter() {
    let test = node(1_000, "*").await;
    let failed = fail_work(&test).await;
    test.system().delete_actor(&addr(&worker("w"))).await.unwrap();
    let redrive = to_letters(&test, DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    assert_eq!(status(&test, &redrive).await, Some(OutcomeStatus::Failed));
    assert_eq!(letters(&test).await.keys().collect::<Vec<_>>(), [&failed.id], "no letter for its own failure");
}

/// Scenario: No delay.
#[tokio::test(start_paused = true)]
async fn the_letter_is_recorded_by_the_next_turn_of_the_dead_letters_actor() {
    let test = node(1_000, "*").await;
    let at = test.now();
    let m = test.send(&worker("w"), WorkerMsg::Fail).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the failure");
    assert_eq!(test.pending(&letters_actor()).await.len(), 1, "the Failed is in the mailbox at once");
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the next turn of the dead-letters actor");
    assert!(letters(&test).await.contains_key(&m.id));
    assert_eq!(test.now(), at, "no time passed");
}

/// Scenario: More letters than the capacity.
#[tokio::test(start_paused = true)]
async fn the_letters_recorded_first_go_beyond_the_capacity() {
    let test = node(2, "*").await;
    let mut failed = Vec::new();
    for _ in 0..3 {
        failed.push(fail_work(&test).await.id);
        test.advance_time_by(Duration::from_secs(1)).await;
    }
    let held: Vec<MessageId> = letters(&test).await.into_keys().collect();
    let mut expected = vec![failed[1].clone(), failed[2].clone()];
    expected.sort();
    assert_eq!(held, expected, "the second and the third");
}

/// Scenario: More letters than the capacity, recorded at one store time: the letter recorded first goes, whatever the
/// ids of the failed messages.
#[tokio::test(start_paused = true)]
async fn of_letters_recorded_at_one_store_time_the_first_goes_beyond_the_capacity() {
    let test = node(1, "*").await;
    let at = test.now();
    let mut failed = Vec::new();
    for id in ["m-b-first", "m-a-second"] {
        let options = SendOptions::new().id(MessageId::new(id).unwrap());
        failed.push(test.send_with(&worker("w"), WorkerMsg::Fail, options).await.id);
        test.run_until_idle().await;
    }
    assert_eq!(test.now(), at, "one store time");
    let held: Vec<MessageId> = letters(&test).await.into_keys().collect();
    assert_eq!(held, [failed[1].clone()], "the second is kept, although its id sorts first");
}

/// Scenario: A redrive, and Scenario: A second redrive.
#[tokio::test(start_paused = true)]
async fn a_redrive_sends_the_failed_message_again_once() {
    let test = node(1_000, "*").await;
    let failed = fail_work(&test).await;
    let pending_before = test.pending(&worker("w")).await.len();
    test.system().pause_mailbox(&addr(&worker("w"))).await.unwrap();
    to_letters(&test, DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    let letter = letter(&test, &failed.id).await;
    let sent = letter.redriven_as.clone().expect("the letter holds the id of the new message");
    let pending = test.pending(&worker("w")).await;
    assert_eq!(pending.len(), pending_before + 1);
    let again = pending.iter().find(|m| m.id == sent).expect("the new message");
    assert_eq!((again.message_type.as_str(), &again.payload), ("Fail", &json!({})));
    assert_eq!(again.correlation_id.as_deref(), Some("c-1"));
    assert_eq!(again.from, Some(addr(&letters_actor())));

    to_letters(&test, DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    assert_eq!(test.pending(&worker("w")).await.len(), pending_before + 1, "nothing is sent a second time");
    assert_eq!(self::letter(&test, &failed.id).await.redriven_as, Some(sent));
}

/// Scenario: A redrive that fails again.
#[tokio::test(start_paused = true)]
async fn a_redriven_message_that_fails_again_is_a_letter_of_its_own() {
    let test = node(1_000, "*").await;
    let failed = fail_work(&test).await;
    to_letters(&test, DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    let sent = letter(&test, &failed.id).await.redriven_as.unwrap();
    let letters = letters(&test).await;
    assert_eq!(letters.len(), 2);
    assert_eq!(letters[&sent].error, "boom");
}

/// `legacy/{id}` before and after a deploy that drops its message type `Retired`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyV1 {
    id: String,
}

impl ActorRef for LegacyV1 {
    const TYPE: &'static str = "legacy";
    type Message = LegacyV1Msg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum LegacyV1Msg {
    Retired,
    Work,
}

struct LegacyV1Actor;

impl Actor for LegacyV1Actor {
    type Ref = LegacyV1;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, message: LegacyV1Msg) -> HandlerResult {
        match message {
            LegacyV1Msg::Retired => Err(HandlerError::msg("retired")),
            LegacyV1Msg::Work => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyV2 {
    id: String,
}

impl ActorRef for LegacyV2 {
    const TYPE: &'static str = "legacy";
    type Message = LegacyV2Msg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum LegacyV2Msg {
    Work,
}

struct LegacyV2Actor;

impl Actor for LegacyV2Actor {
    type Ref = LegacyV2;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: LegacyV2Msg) -> HandlerResult {
        Ok(())
    }
}

/// Scenario: A type that no longer takes the message. A message of `legacy` fails; a deploy changes the contract of
/// `legacy` so that it no longer has that message type ("Changing a contract", stop the world); then the letter is
/// redriven.
#[tokio::test(start_paused = true)]
async fn a_redrive_the_type_no_longer_takes_sends_nothing_and_records_why() {
    let test = TestActors::builder()
        .with(|node| node.host(DeadLettersActor::default()).host(LegacyV1Actor))
        .build()
        .await
        .unwrap();
    test.create(&letters_actor(), &DeadLettersState::default()).await;
    test.system().watch(&WatchTarget::Every, &addr(&letters_actor())).await.unwrap();
    let old = LegacyV1 { id: "l".into() };
    test.create(&old, &NoState {}).await;
    let failed = test.send(&old, LegacyV1Msg::Retired).await;
    test.run_until_idle().await;
    assert!(letters(&test).await.contains_key(&failed.id));

    test.system().delete_actor(&addr(&old)).await.unwrap();
    test.system().remove_type(&cicada::TypeName::parse("legacy").unwrap()).await.unwrap();
    let later = ActorSystem::builder(test.store().clone()).host(LegacyV2Actor).build();
    later.initialize().await.unwrap();
    let new = LegacyV2 { id: "l".into() };
    later.create(&new, &NoState {}).await.unwrap();

    let redrive = to_letters(&test, DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    assert_eq!(status(&test, &redrive).await, Some(OutcomeStatus::Done));
    let letter = letter(&test, &failed.id).await;
    assert_eq!(letter.redriven_as, None, "nothing is sent");
    assert!(letter.redrive_error.as_deref().unwrap().contains("Retired"), "{letter:?}");
    let pending = later.pending(&addr(&new), None, 10).await.unwrap().unwrap();
    assert!(pending.items.is_empty());
}

/// Scenario: A `Failed` sent by a client.
#[tokio::test(start_paused = true)]
async fn a_failed_sent_by_a_client_records_no_letter() {
    let test = node(1_000, "*").await;
    let planted = Failed {
        actor: addr(&worker("w")),
        message_id: MessageId::new("m-planted").unwrap(),
        message_type: "Work".into(),
        payload: json!({}),
        correlation_id: None,
        from: None,
        error: "made up".into(),
        attempts: 1,
        cause: FailureCause::Handler,
    };
    let version = test.version(&letters_actor()).await;
    let m = to_letters(&test, DeadLettersMsg::Failed(planted)).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(letters(&test).await.is_empty(), "no letter: a later redrive cannot send what a client chose");
    assert_eq!(test.version(&letters_actor()).await, version, "nothing changes");
}

/// Scenario: The actor is gone.
#[tokio::test(start_paused = true)]
async fn a_redrive_to_an_actor_that_is_gone_fails_with_the_cause_commit_and_leaves_the_letter() {
    let test = node(1_000, "*").await;
    let failed = fail_work(&test).await;
    let before = letter(&test, &failed.id).await;
    test.system().delete_actor(&addr(&worker("w"))).await.unwrap();
    let redrive = to_letters(&test, DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    let told = told(&test).await;
    let failure = told.iter().find(|f| f.message_id == redrive.id).expect("the witness is told");
    assert_eq!((failure.message_type.as_str(), failure.cause), ("Redrive", FailureCause::Commit));
    assert_eq!(letter(&test, &failed.id).await, before, "the letter is as it was");
}

/// Scenario: A discard.
#[tokio::test(start_paused = true)]
async fn a_discard_forgets_a_letter() {
    let test = node(1_000, "*").await;
    let failed = fail_work(&test).await;
    to_letters(&test, DeadLettersMsg::Discard { message_id: failed.id.clone() }).await;
    assert!(letters(&test).await.is_empty());
}

/// Scenario: A letter that is not there.
#[tokio::test(start_paused = true)]
async fn a_redrive_or_a_discard_of_an_id_that_is_no_letter_changes_nothing() {
    let test = node(1_000, "*").await;
    let failed = fail_work(&test).await;
    let version = test.version(&letters_actor()).await;
    let unknown = MessageId::new("no-such-letter").unwrap();
    for message in
        [DeadLettersMsg::Redrive { message_id: unknown.clone() }, DeadLettersMsg::Discard { message_id: unknown }]
    {
        let m = to_letters(&test, message).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    }
    assert_eq!(test.version(&letters_actor()).await, version, "nothing changes");
    assert_eq!(letters(&test).await.keys().collect::<Vec<_>>(), [&failed.id]);
}

/// A value `levels` deep: a chain of arrays around a number, built without recursion.
fn nested(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, _| Value::Array(vec![inner]))
}

/// The error of a turn of the dead-letters actor whose letter would make its state too deep.
const TOO_DEEP: &str = "State cannot be stored: The state is nested deeper than 64 levels";

/// Sends `billing.invoices/a` a message whose payload is `levels` deep, which fails; runs the invoice's turn and
/// returns the message and the `Failed` that the dead-letters actor gets, which waits in its paused mailbox.
async fn fail_deep(test: &TestActors, levels: usize) -> (Message, Message) {
    test.create(&Invoice { id: "a".into() }, &NoState {}).await;
    test.system().pause_mailbox(&addr(&letters_actor())).await.unwrap();
    let failed = test.send(&Invoice { id: "a".into() }, Failing::Deep { v: nested(levels - 1) }).await;
    test.run_until_idle().await;
    let notification = test.pending(&letters_actor()).await.pop().expect("the Failed");
    test.system().resume_mailbox(&addr(&letters_actor())).await.unwrap();
    (failed, notification)
}

/// Scenario: The deepest payload a letter keeps. A letter holds its payload three levels into the state, so a payload
/// of 61 levels makes a state of 64.
#[tokio::test(start_paused = true)]
async fn a_letter_keeps_a_payload_of_61_levels() {
    let test = node(1_000, "*").await;
    let (failed, notification) = fail_deep(&test, 61).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &notification).await, Some(OutcomeStatus::Done));
    assert_eq!(letter(&test, &failed.id).await.payload, json!({"v": nested(60)}));
}

/// Scenario: A payload too deep for a letter. With a payload of 62 levels the state would be 65 deep: the turn that
/// would record it fails and records nothing, and the next failure is recorded as before. The `Failed` that fails is a
/// notification, which tells nobody, so its cause shows only in what the node writes for a new state that cannot be
/// stored, with the cause `handler`: its error.
#[tokio::test(start_paused = true)]
async fn a_payload_too_deep_for_a_letter_fails_the_turn_and_records_nothing() {
    let test = node(1_000, "*").await;
    let (failed, notification) = fail_deep(&test, 62).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&notification).await.expect("an outcome");
    assert_eq!((outcome.status, outcome.error.as_deref()), (OutcomeStatus::Failed, Some(TOO_DEEP)));
    assert!(letters(&test).await.is_empty(), "no letter for {}", failed.id);
    assert!(told(&test).await.is_empty(), "a notification that fails tells nobody");

    let next = fail_work(&test).await;
    assert_eq!(letters(&test).await.keys().collect::<Vec<_>>(), [&next.id]);
}

/// A node on a SQLite store in `dir` with dead letters watching every type and the worker `worker/w`, and a second
/// connection to the database, to write a letter as an earlier release could.
async fn sqlite_node(dir: &std::path::Path) -> (ActorSystem, Store, sqlx::SqlitePool) {
    let path = dir.join("cicada.db");
    let store = Store::sqlite(&path, SqliteOptions::default()).await.unwrap();
    let system = ActorSystem::builder(store.clone()).host(DeadLettersActor::default()).host(WorkerActor).build();
    system.initialize().await.unwrap();
    system.create(&letters_actor(), &DeadLettersState::default()).await.unwrap();
    system.watch(&WatchTarget::Every, &addr(&letters_actor())).await.unwrap();
    system.create(&worker("w"), &WorkerState::default()).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display())).await.unwrap();
    (system, store, pool)
}

/// Sends `message`, runs every due turn and returns how it ended, with its error.
async fn handled<R: ActorRef>(
    system: &ActorSystem,
    to: &R,
    message: impl Into<R::Message>,
) -> (OutcomeStatus, Option<String>) {
    let cicada::SendResult::Accepted(m) = system.send(to, message).await.unwrap() else { panic!("not accepted") };
    while system.run_due_turns(10).await.unwrap() > 0 {}
    match system.status(&m.id).await.unwrap() {
        Some(MessageStatus::Finished(o)) => (o.status, o.error),
        other => panic!("{} has no outcome: {other:?}", m.id),
    }
}

/// The letters the dead-letters actor holds.
async fn held(system: &ActorSystem) -> Vec<MessageId> {
    let state = system.get_actor(&addr(&letters_actor())).await.unwrap().expect("the actor").state;
    serde_json::from_value::<DeadLettersState>(state).unwrap().letters.into_keys().collect()
}

/// Scenario: A deep letter from before. A letter whose payload is 70 levels deep, written past the checks into a SQLite
/// store, makes every turn that writes the state fail, so a failure is not recorded; once the operator discards that
/// letter, the next failure is. Real time: SQLite works on threads of its own, which tokio's paused clock does not wait
/// for.
#[tokio::test]
async fn a_deep_letter_from_before_stops_the_letters_until_it_is_discarded() {
    let dir = tempfile::TempDir::new().unwrap();
    let (system, store, pool) = sqlite_node(dir.path()).await;
    let old = MessageId::new("m-old").unwrap();
    let letter = DeadLetter {
        actor: addr(&worker("w")),
        message_type: "Fail".into(),
        payload: nested(70),
        correlation_id: None,
        from: None,
        error: "boom".into(),
        cause: FailureCause::Handler,
        attempts: 1,
        recorded_at: system.now().await.unwrap(),
        seq: 1,
        redriven_as: None,
        redrive_error: None,
    };
    let state = DeadLettersState { letters: [(old.clone(), letter)].into() };
    sqlx::query("UPDATE actors SET state = ? WHERE address = ?")
        .bind(serde_json::to_string(&state).unwrap())
        .bind(addr(&letters_actor()).as_str())
        .execute(&pool)
        .await
        .unwrap();

    let (status, _) = handled(&system, &worker("w"), WorkerMsg::Fail).await;
    assert_eq!(status, OutcomeStatus::Failed);
    let notification = system.outcomes(Some(OutcomeStatus::Failed), "cicada.dead-letters/", None, None, 10).await;
    let notification = notification.unwrap().items.pop().expect("the Failed of the dead-letters actor failed");
    assert_eq!(notification.error.as_deref(), Some(TOO_DEEP));
    assert_eq!(held(&system).await, std::slice::from_ref(&old), "no letter is recorded");

    let discard = DeadLettersMsg::Discard { message_id: old };
    assert_eq!(handled(&system, &letters_actor(), discard).await, (OutcomeStatus::Done, None));
    assert!(held(&system).await.is_empty());
    let (status, _) = handled(&system, &worker("w"), WorkerMsg::Fail).await;
    assert_eq!(status, OutcomeStatus::Failed);
    assert_eq!(held(&system).await.len(), 1, "the next failure is recorded");
    pool.close().await;
    store.close().await;
}

/// The default name of the dead-letters actor is `_`.
#[test]
fn the_dead_letters_actor_is_at_cicada_dead_letters_underscore_by_convention() {
    assert_eq!(addr(&DeadLetters::default()).as_str(), "cicada.dead-letters/_");
    assert_eq!(addr(&DeadLetters::named("ops")).as_str(), "cicada.dead-letters/ops");
    assert_eq!(DeadLettersActor::with_capacity(0).unwrap_err().to_string(), "capacity must be positive");
    assert_eq!(DeadLettersActor::default().capacity(), 1_000);
}
