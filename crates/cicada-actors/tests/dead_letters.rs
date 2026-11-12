//! Dead letters, on virtual time with `TestActors` (ADR 0028): told of failed messages by a watch, with no timer; a
//! capacity; a redrive, once; and a discard.

mod common;

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::{Failed, FailureCause, WatchTarget};
use cicada_actors::{DeadLetter, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

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

/// The default name of the dead-letters actor is `_`.
#[test]
fn the_dead_letters_actor_is_at_cicada_dead_letters_underscore_by_convention() {
    assert_eq!(addr(&DeadLetters::default()).as_str(), "cicada.dead-letters/_");
    assert_eq!(addr(&DeadLetters::named("ops")).as_str(), "cicada.dead-letters/ops");
    assert_eq!(DeadLettersActor::with_capacity(0).unwrap_err().to_string(), "capacity must be positive");
    assert_eq!(DeadLettersActor::default().capacity(), 1_000);
}
