//! The topic, on virtual time with `TestActors` (ADR 0028): subscribers, a delivery to each in a turn of its own, the
//! drop of a subscriber that is not there, and a refused delivery that fails for good.

mod common;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::{FailureCause, WatchTarget};
use cicada_actors::{StoredMessage, Topic, TopicActor, TopicMsg, TopicState};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};

fn topic() -> Topic {
    Topic::named("orders")
}

/// A message of a family that the worker does not take.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Shout {
    Shout { text: String },
}

/// A node with the topic `cicada.topic/orders`, the workers `worker/a` and `worker/b`, and the witness, which watches
/// the topic.
async fn node() -> TestActors {
    let test = TestActors::builder()
        .with(|node| node.host(TopicActor).host(WorkerActor).host(WitnessActor))
        .build()
        .await
        .unwrap();
    test.create(&topic(), &TopicState::default()).await;
    for id in ["a", "b"] {
        test.create(&worker(id), &WorkerState::default()).await;
    }
    witness(&test, "cicada.topic").await;
    test
}

async fn subscribe(test: &TestActors, subscriber: &str) {
    let subscriber = ActorAddress::parse(subscriber).unwrap();
    test.send(&topic(), TopicMsg::Subscribe { subscriber }).await;
    test.run_until_idle().await;
}

fn note(text: &str) -> TopicMsg {
    TopicMsg::publish(WorkerMsg::Note { text: text.into() }).unwrap()
}

async fn publish(test: &TestActors, text: &str) {
    test.send(&topic(), note(text)).await;
    test.run_until_idle().await;
}

async fn log(test: &TestActors, id: &str) -> Vec<String> {
    test.state::<_, WorkerState>(&worker(id)).await.log
}

async fn subscribers(test: &TestActors) -> Vec<String> {
    test.state::<_, TopicState>(&topic()).await.subscribers.iter().map(ToString::to_string).collect()
}

/// The `Failed` the witness was told of the deliveries of the topic.
async fn failed_deliveries(test: &TestActors) -> Vec<cicada::Failed> {
    told(test).await.into_iter().filter(|f| f.message_type == "Deliver").collect()
}

/// Scenario: Two subscribers.
#[tokio::test(start_paused = true)]
async fn each_subscriber_gets_a_published_message_once() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    subscribe(&test, "worker/b").await;
    publish(&test, "shipped").await;
    assert_eq!(log(&test, "a").await, ["shipped"]);
    assert_eq!(log(&test, "b").await, ["shipped"]);
}

/// Scenario: The order for one subscriber.
#[tokio::test(start_paused = true)]
async fn a_subscriber_gets_the_messages_in_the_order_they_were_published() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    for text in ["one", "two", "three"] {
        test.send(&topic(), note(text)).await;
    }
    test.run_until_idle().await;
    assert_eq!(log(&test, "a").await, ["one", "two", "three"]);
}

/// Scenario: A subscriber leaves.
#[tokio::test(start_paused = true)]
async fn a_subscriber_that_left_gets_nothing() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    test.send(&topic(), TopicMsg::Unsubscribe { subscriber: addr(&worker("a")) }).await;
    publish(&test, "shipped").await;
    assert!(log(&test, "a").await.is_empty());
    assert!(subscribers(&test).await.is_empty());
}

/// Scenario: A publish before an unsubscribe.
#[tokio::test(start_paused = true)]
async fn a_publish_queued_before_an_unsubscribe_delivers_nothing_after_it() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    test.send(&topic(), note("shipped")).await;
    test.send(&topic(), TopicMsg::Unsubscribe { subscriber: addr(&worker("a")) }).await;
    test.run_until_idle().await;
    assert!(log(&test, "a").await.is_empty());
}

/// Scenario: A delivery sent by a client.
#[tokio::test(start_paused = true)]
async fn a_delivery_sent_by_a_client_ends_done_and_sends_nothing() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    let message = StoredMessage::for_ref(&worker("a"), WorkerMsg::Note { text: "made up".into() }).unwrap();
    let m = test.send(&topic(), TopicMsg::Deliver { subscriber: addr(&worker("a")), message }).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(log(&test, "a").await.is_empty());
}

/// Scenario: Subscribing twice.
#[tokio::test(start_paused = true)]
async fn an_actor_that_subscribes_twice_gets_a_message_once() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    subscribe(&test, "worker/a").await;
    assert_eq!(subscribers(&test).await, ["worker/a"]);
    publish(&test, "shipped").await;
    assert_eq!(log(&test, "a").await, ["shipped"]);
    let watches = test.system().watches(Some(&addr(&topic())), None, None, 10).await.unwrap().items;
    assert_eq!(watches.iter().map(|w| w.target.to_string()).collect::<Vec<_>>(), ["cicada.topic/orders"]);
}

/// Scenario: The correlation id of a publish.
#[tokio::test(start_paused = true)]
async fn each_subscriber_gets_the_correlation_id_of_the_publish() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    subscribe(&test, "worker/b").await;
    for id in ["a", "b"] {
        test.system().pause_mailbox(&addr(&worker(id))).await.unwrap();
    }
    test.send_with(&topic(), note("shipped"), SendOptions::new().correlation_id("c1")).await;
    test.run_until_idle().await;
    for id in ["a", "b"] {
        let pending = test.pending(&worker(id)).await;
        assert_eq!(pending.len(), 1, "{id}");
        assert_eq!(pending[0].correlation_id.as_deref(), Some("c1"), "{id}");
        assert_eq!(pending[0].message_type, "Note");
    }
}

/// Scenario: A subscriber was deleted.
#[tokio::test(start_paused = true)]
async fn a_deleted_subscriber_fails_its_own_delivery_and_the_other_gets_the_message() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    subscribe(&test, "worker/b").await;
    test.system().delete_actor(&addr(&worker("a"))).await.unwrap();
    publish(&test, "shipped").await;
    assert_eq!(log(&test, "b").await, ["shipped"]);
    let failed = failed_deliveries(&test).await;
    assert_eq!(failed.len(), 1, "the delivery to the deleted one fails alone: {failed:?}");
    assert_eq!(failed[0].cause, FailureCause::Commit);
    assert_eq!(failed[0].payload["subscriber"], "worker/a");
}

/// Scenario: A subscriber that went away is dropped.
#[tokio::test(start_paused = true)]
async fn a_subscriber_that_went_away_is_dropped_after_its_first_rejected_delivery() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    subscribe(&test, "worker/b").await;
    test.system().delete_actor(&addr(&worker("a"))).await.unwrap();
    publish(&test, "first").await;
    assert_eq!(subscribers(&test).await, ["worker/b"]);
    publish(&test, "second").await;
    assert_eq!(failed_deliveries(&test).await.len(), 1, "a later publish makes no message for it");
    assert_eq!(log(&test, "b").await, ["first", "second"]);
}

/// Scenario: The failures of a subscriber are not the topic's business.
#[tokio::test(start_paused = true)]
async fn a_failure_of_a_subscriber_tells_the_topic_nothing() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    let version = test.version(&topic()).await;
    test.send(&worker("a"), WorkerMsg::Fail).await;
    test.system().pause_mailbox(&addr(&topic())).await.unwrap();
    test.run_until_idle().await;
    assert!(test.pending(&topic()).await.is_empty(), "the topic gets no message about it");
    assert_eq!(test.version(&topic()).await, version);
}

/// Scenario: A subscriber whose type is not registered.
#[tokio::test(start_paused = true)]
async fn a_delivery_to_an_address_without_a_registered_type_is_refused_and_the_address_stays() {
    let test = node().await;
    subscribe(&test, "nobody/x").await;
    publish(&test, "shipped").await;
    let failed = failed_deliveries(&test).await;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].cause, FailureCause::Refused);
    assert_eq!(failed[0].attempts, 1, "a refused send is not tried again");
    assert_eq!(subscribers(&test).await, ["nobody/x"], "it stays subscribed");
}

/// Scenario: A message the subscriber's type does not take.
#[tokio::test(start_paused = true)]
async fn a_delivery_the_type_of_the_subscriber_does_not_take_is_refused_and_the_subscriber_stays() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    test.send(&topic(), TopicMsg::publish(Shout::Shout { text: "hey".into() }).unwrap()).await;
    test.run_until_idle().await;
    let failed = failed_deliveries(&test).await;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].cause, FailureCause::Refused);
    assert!(failed[0].error.contains("Shout"), "{}", failed[0].error);
    assert_eq!(subscribers(&test).await, ["worker/a"]);
    assert!(log(&test, "a").await.is_empty(), "nothing is sent");
}

/// Scenario: A subscriber that never existed.
#[tokio::test(start_paused = true)]
async fn a_subscriber_that_never_existed_is_dropped_after_its_delivery_is_rejected() {
    let test = node().await;
    subscribe(&test, "worker/ghost").await;
    publish(&test, "first").await;
    let failed = failed_deliveries(&test).await;
    assert_eq!(failed.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Commit]);
    assert!(subscribers(&test).await.is_empty());
    publish(&test, "second").await;
    assert_eq!(failed_deliveries(&test).await.len(), 1, "a second publish makes no message for it");
}

/// Scenario: A `Failed` sent by a client. It names a rejected delivery to a subscriber that exists.
#[tokio::test(start_paused = true)]
async fn a_failed_sent_by_a_client_drops_no_subscriber() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    let message = StoredMessage::for_ref(&worker("a"), WorkerMsg::Note { text: "x".into() }).unwrap();
    let deliver = cicada::message::encode(&TopicMsg::Deliver { subscriber: addr(&worker("a")), message }).unwrap();
    let planted = cicada::Failed {
        actor: addr(&topic()),
        message_id: MessageId::new("m-planted").unwrap(),
        message_type: deliver.message_type,
        payload: deliver.payload,
        correlation_id: None,
        from: Some(addr(&topic())),
        error: "made up".into(),
        attempts: 1,
        cause: FailureCause::Commit,
    };
    let version = test.version(&topic()).await;
    let m = test.send(&topic(), TopicMsg::Failed(planted)).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(subscribers(&test).await, ["worker/a"], "the subscriber stays");
    assert_eq!(test.version(&topic()).await, version, "nothing changes");
}

/// The topic watches only itself.
#[tokio::test(start_paused = true)]
async fn a_topic_watches_itself_and_not_its_subscribers() {
    let test = node().await;
    subscribe(&test, "worker/a").await;
    let watches = test.system().watches(None, Some(&WatchTarget::from(addr(&worker("a")))), None, 10).await.unwrap();
    assert!(watches.items.is_empty());
}
