//! The failure watcher and the dead letters, ported from `SupervisionTest.kt`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada_extras::{DeadLetter, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, FailureWatcher};
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Mailer {
    id: String,
}

impl ActorRef for Mailer {
    const TEMPLATE: &'static str = "actor://mailers/{id}";
    type Message = MailerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum MailerMsg {
    SendMail { to: String },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct MailerState {
    sent: Vec<String>,
}

struct MailerActor {
    smtp_up: Arc<AtomicBool>,
}

impl Actor for MailerActor {
    type Ref = Mailer;
    type State = MailerState;
    type Message = MailerMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: MailerMsg) -> HandlerResult {
        let MailerMsg::SendMail { to } = message;
        if !self.smtp_up.load(Ordering::SeqCst) {
            return Err(HandlerError::msg("smtp down"));
        }
        turn.state_mut().sent.push(to);
        Ok(())
    }
}

fn mailer(id: &str) -> Mailer {
    Mailer { id: id.into() }
}

fn redrive(id: &MessageId) -> DeadLettersMsg {
    DeadLettersMsg::Redrive { failed_message_id: id.to_string() }
}

async fn harness(capacity: usize) -> (TestActors, Arc<AtomicBool>) {
    let smtp_up = Arc::new(AtomicBool::new(false));
    let actor = MailerActor { smtp_up: smtp_up.clone() };
    let test = TestActors::builder()
        .with(move |node| node.host(actor).host(DeadLettersActor::with_capacity(capacity).unwrap()))
        .build()
        .await
        .unwrap();
    (test, smtp_up)
}

async fn letters(test: &TestActors) -> BTreeMap<String, DeadLetter> {
    test.state::<_, DeadLettersState>(&DeadLetters).await.letters
}

#[tokio::test(start_paused = true)]
async fn the_watcher_copies_each_failure_once_into_the_dead_letters() {
    let (test, _) = harness(1_000).await;
    let mut watcher = FailureWatcher::new(test.system().clone());
    let mut other_node = FailureWatcher::new(test.system().clone()); // a second node's watcher: same ids, so no duplicates
    test.create(&mailer("m"), &MailerState::default()).await;
    let failed = test
        .send_with(&mailer("m"), MailerMsg::SendMail { to: "a@example.test".into() }, SendOptions::new().correlation_id("c-1"))
        .await;
    test.send(&mailer("m"), MailerMsg::SendMail { to: "b@example.test".into() }).await;
    test.run_until_idle().await;
    assert_eq!(test.failures().await.len(), 2);

    assert_eq!(watcher.poll().await.unwrap(), 2);
    assert_eq!(other_node.poll().await.unwrap(), 0);
    assert_eq!(watcher.poll().await.unwrap(), 0);
    test.run_until_idle().await;

    let letters = letters(&test).await;
    assert_eq!(letters.len(), 2);
    let letter = &letters[failed.id.as_str()];
    assert_eq!(letter.actor, "actor://mailers/m");
    assert_eq!(letter.message_type, "SendMail");
    assert_eq!(letter.payload, json!({ "to": "a@example.test" }));
    assert_eq!(letter.correlation_id.as_deref(), Some("c-1"));
    assert!(letter.error.as_deref().unwrap().contains("smtp down"));
}

#[tokio::test(start_paused = true)]
async fn redrive_sends_the_failed_message_again_once_the_cause_is_fixed() {
    let (test, smtp_up) = harness(1_000).await;
    let mut watcher = FailureWatcher::new(test.system().clone()).prefix("actor://mailers/");
    test.create(&mailer("m"), &MailerState::default()).await;
    let failed = test.send(&mailer("m"), MailerMsg::SendMail { to: "a@example.test".into() }).await;
    test.run_until_idle().await;
    watcher.poll().await.unwrap();
    test.run_until_idle().await;

    smtp_up.store(true, Ordering::SeqCst);
    test.send(&DeadLetters, redrive(&failed.id)).await;
    test.send(&DeadLetters, redrive(&failed.id)).await; // redriven once only
    test.run_until_idle().await;

    assert_eq!(test.state::<_, MailerState>(&mailer("m")).await.sent, vec!["a@example.test"]);
    let letter = letters(&test).await.remove(failed.id.as_str()).unwrap();
    assert!(letter.redriven_as.is_some());
    assert_eq!(letter.redrive_error, None);

    test.send(&DeadLetters, DeadLettersMsg::Discard { failed_message_id: failed.id.to_string() }).await;
    test.run_until_idle().await;
    assert!(letters(&test).await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_redrive_to_a_deleted_actor_fails_without_becoming_a_dead_letter_itself() {
    let (test, _) = harness(1_000).await;
    let mut watcher = FailureWatcher::new(test.system().clone());
    test.create(&mailer("gone"), &MailerState::default()).await;
    let failed = test.send(&mailer("gone"), MailerMsg::SendMail { to: "x@example.test".into() }).await;
    test.run_until_idle().await;
    watcher.poll().await.unwrap();
    test.run_until_idle().await;
    test.system().delete_actor(&mailer("gone").address().unwrap()).await.unwrap();

    let redrive = test.send(&DeadLetters, redrive(&failed.id)).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&redrive).await.map(|o| o.status), Some(OutcomeStatus::Failed));
    assert_eq!(watcher.poll().await.unwrap(), 0);
    assert_eq!(letters(&test).await[failed.id.as_str()].redriven_as, None);
}

#[tokio::test(start_paused = true)]
async fn a_redrive_of_a_type_the_target_does_not_accept_records_the_refusal() {
    let (test, _) = harness(1_000).await;
    test.create(&mailer("m"), &MailerState::default()).await;
    let letter = DeadLetter {
        failed_message_id: "f-1".into(),
        actor: "actor://mailers/m".into(),
        message_type: "Unknown".into(),
        payload: json!({}),
        correlation_id: None,
        error: None,
        failed_at: "2026-01-01T00:00:00Z".parse().unwrap(),
        redriven_as: None,
        redrive_error: None,
    };
    test.create(&DeadLetters, &DeadLettersState { letters: BTreeMap::from([("f-1".to_owned(), letter)]) }).await;
    test.send(&DeadLetters, DeadLettersMsg::Redrive { failed_message_id: "f-1".into() }).await;
    test.run_until_idle().await;
    let letter = letters(&test).await.remove("f-1").unwrap();
    assert_eq!(letter.redriven_as, None);
    assert!(letter.redrive_error.as_deref().unwrap().contains("Unknown"), "{:?}", letter.redrive_error);
}

#[tokio::test(start_paused = true)]
async fn the_dead_letters_keep_at_most_their_capacity() {
    let (test, _) = harness(2).await;
    test.create(&mailer("m"), &MailerState::default()).await;
    for i in 0..3 {
        test.send(&mailer("m"), MailerMsg::SendMail { to: format!("{i}@example.test") }).await;
    }
    test.run_until_idle().await;
    FailureWatcher::new(test.system().clone()).poll().await.unwrap();
    test.run_until_idle().await;
    assert_eq!(letters(&test).await.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_oldest_letter_is_evicted_by_failure_time_whatever_the_order_of_the_stored_keys() {
    // PostgreSQL's jsonb does not keep object key order, so the key order says nothing about age.
    let (test, _) = harness(2).await;
    let letter = |id: &str, second: i64| DeadLetter {
        failed_message_id: id.into(),
        actor: "actor://mailers/m".into(),
        message_type: "SendMail".into(),
        payload: json!({}),
        correlation_id: None,
        error: None,
        failed_at: cicada::time::Timestamp::from_timestamp(second, 0).unwrap(),
        redriven_as: None,
        redrive_error: None,
    };
    // "a" sorts first by key but is the newest.
    let state = DeadLettersState { letters: BTreeMap::from([("a".to_owned(), letter("a", 2)), ("b".to_owned(), letter("b", 1))]) };
    test.create(&DeadLetters, &state).await;
    test.send(&DeadLetters, DeadLettersMsg::RecordFailure { letter: letter("c", 3) }).await;
    test.run_until_idle().await;
    let keys: BTreeSet<String> = letters(&test).await.into_keys().collect();
    assert_eq!(keys, BTreeSet::from(["a".to_owned(), "c".to_owned()]));
}

#[test]
fn messages_and_state_keep_the_kotlin_wire_form() {
    let msg = cicada::message::encode(&redrive(&MessageId::new("m-1").unwrap())).unwrap();
    assert_eq!(msg.message_type, "Redrive");
    assert_eq!(msg.payload, json!({ "failedMessageId": "m-1" }));
    let letter: DeadLetter = serde_json::from_value(json!({
        "failedMessageId": "m-1", "actor": "actor://mailers/m", "type": "SendMail", "payload": { "to": "a" },
        "failedAt": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&letter).unwrap(),
        json!({
            "failedMessageId": "m-1", "actor": "actor://mailers/m", "type": "SendMail", "payload": { "to": "a" },
            "correlationId": null, "error": null, "failedAt": "2026-01-01T00:00:00Z", "redrivenAs": null,
            "redriveError": null
        })
    );
    assert_eq!(DeadLettersActor::with_capacity(0).unwrap_err(), "capacity must be positive");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_watcher_in_another_process_works_through_the_http_client() {
    // The node serves its API; the watcher reaches it only over HTTP, as a watcher in a process of its own would.
    let store = Arc::new(cicada_test::InMemoryStore::default());
    let smtp_up = Arc::new(AtomicBool::new(false));
    let system = ActorSystem::builder(store)
        .host(MailerActor { smtp_up })
        .host(DeadLettersActor::with_capacity(1_000).unwrap())
        .poll_interval(std::time::Duration::from_millis(20))
        .build();
    system.start().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = cicada_axum::actor_api(system.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = cicada_client::ActorClient::new(&base);

    client.create(&mailer("m"), &MailerState::default()).await.unwrap();
    let failed = client.send(&mailer("m"), MailerMsg::SendMail { to: "a@example.test".into() }).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while client.failures("", None, None, None).await.unwrap().items.is_empty() {
        assert!(std::time::Instant::now() < deadline, "the send did not fail in time");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let mut watcher = FailureWatcher::new(client.clone());
    assert_eq!(watcher.poll().await.unwrap(), 1);
    assert_eq!(FailureWatcher::new(client.clone()).poll().await.unwrap(), 0);
    let letters = loop {
        let state: Option<DeadLettersState> = client.state(&DeadLetters).await.unwrap();
        if let Some(state) = state.filter(|s| !s.letters.is_empty()) {
            break state.letters;
        }
        assert!(std::time::Instant::now() < deadline, "no dead letter in time");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    assert!(letters.contains_key(failed.id.as_str()));
    server.abort();
    system.stop().await;
}
