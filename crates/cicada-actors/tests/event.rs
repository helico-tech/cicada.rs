//! The event actor, on virtual time with `TestActors`: bounded explicit subscriptions, general and batch publications,
//! atomic creates and starts keyed by the publication id, dot ids and percent-encoding, the context of the first
//! `Continue`, failures that commit no partial fan-out, input checks on hosting and other nodes, deduplication inside
//! and after the store's retention, fresh keys for retries and redrives, composition with the scheduler, rate limiter,
//! debouncer and batcher, independent runs, and the depth of a batch item.

mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::RetryResult;
use cicada::store::{Message, OutcomeStatus};
use cicada::{FailureCause, SendResult, WatchTarget};
use cicada_actors::{
    Batcher, BatcherActor, BatcherMsg, BatcherState, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState,
    Debouncer, DebouncerActor, DebouncerMsg, DebouncerState, Event, EventActor, EventMsg, EventState, Flow, FlowActor,
    FlowMsg, FlowRun, FlowState, FlowStatus, LetterPayload, RateLimiter, RateLimiterActor, RateLimiterMsg,
    RateLimiterState, Schedule, Scheduler, SchedulerActor, SchedulerMsg, SchedulerState, Step,
};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SECOND: Duration = Duration::from_secs(1);
const HOUR: Duration = Duration::from_secs(3600);

/// The crate's shared retry policy.
fn shared_policy() -> RetryPolicy {
    RetryPolicy::new(7, Backoff::exponential(SECOND, 2.0, Duration::from_secs(32)).unwrap()).unwrap()
}

/// The policy of `shop.fulfil`: three attempts, ten seconds apart.
fn three_attempts() -> RetryPolicy {
    RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(10)).unwrap()).unwrap()
}

/// The keys of every step call, and the failures still planned for `shop.fulfil`.
#[derive(Clone, Default)]
struct Probe {
    keys: Arc<Mutex<Vec<String>>>,
    failing: Arc<AtomicU32>,
}

impl Probe {
    fn call(&self, key: String) -> Result<(), HandlerError> {
        self.keys.lock().unwrap().push(key);
        let planned = self.failing.load(Ordering::SeqCst);
        if planned == 0 {
            return Ok(());
        }
        self.failing.store(planned - 1, Ordering::SeqCst);
        Err(HandlerError::msg("the warehouse failed"))
    }

    fn keys(&self) -> Vec<String> {
        self.keys.lock().unwrap().clone()
    }
}

/// `shop.email`: any input, the crate's policy, one step `send`.
struct Email(Probe);

impl Flow for Email {
    const TYPE: &'static str = "shop.email";
    type Input = Value;

    async fn run(&self, step: &mut Step, _input: Value) -> HandlerResult {
        let probe = self.0.clone();
        step.run("send", |key| async move {
            probe.keys.lock().unwrap().push(key);
            Ok(())
        })
        .await
    }
}

/// `shop.fulfil`: any input, three attempts, one step `ship` that fails as the probe plans.
struct Fulfil(Probe);

impl Flow for Fulfil {
    const TYPE: &'static str = "shop.fulfil";
    type Input = Value;

    fn retry() -> RetryPolicy {
        three_attempts()
    }

    async fn run(&self, step: &mut Step, _input: Value) -> HandlerResult {
        let probe = self.0.clone();
        step.run("ship", |key| async move { probe.call(key) }).await
    }
}

/// The typed input of `shop.typed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Order {
    order: u32,
}

/// `shop.typed`: an [`Order`], one step `bill`.
struct Typed;

impl Flow for Typed {
    const TYPE: &'static str = "shop.typed";
    type Input = Order;

    async fn run(&self, step: &mut Step, order: Order) -> HandlerResult {
        step.run("bill", |_key| async move { Ok(order.order) }).await.map(drop)
    }
}

/// The batch input of `shop.batched`, declared by the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Rows {
    items: Vec<u32>,
}

/// `shop.batched`: [`Rows`], one step `load` that saves their count.
struct Batched;

impl Flow for Batched {
    const TYPE: &'static str = "shop.batched";
    type Input = Rows;

    async fn run(&self, step: &mut Step, rows: Rows) -> HandlerResult {
        step.run("load", |_key| async move { Ok(rows.items.len()) }).await.map(drop)
    }
}

fn ev() -> Event {
    Event::named("order-placed")
}

fn email(key: &str) -> FlowRun<Email> {
    FlowRun::named(key)
}

fn fulfil(key: &str) -> FlowRun<Fulfil> {
    FlowRun::named(key)
}

fn typed(key: &str) -> FlowRun<Typed> {
    FlowRun::named(key)
}

fn batched(key: &str) -> FlowRun<Batched> {
    FlowRun::named(key)
}

fn id(text: &str) -> MessageId {
    MessageId::new(text).unwrap()
}

/// A node with the event, the four flows on `probe`, the worker, the witness (watching the events), the dead letters
/// and the actors that compose with an event; the event `order-placed` exists with no subscription.
async fn node_with(probe: &Probe, retention: Duration) -> TestActors {
    let (a, b) = (probe.clone(), probe.clone());
    let test = TestActors::builder()
        .retention(retention)
        .with(move |node| {
            node.host(EventActor)
                .host(FlowActor::new(Email(a)))
                .host(FlowActor::new(Fulfil(b)))
                .host(FlowActor::new(Typed))
                .host(FlowActor::new(Batched))
                .host(WorkerActor)
                .host(WitnessActor)
                .host(DeadLettersActor::default())
                .host(SchedulerActor)
                .host(RateLimiterActor)
                .host(DebouncerActor)
                .host(BatcherActor)
        })
        .build()
        .await
        .unwrap();
    test.create(&ev(), &EventState::default()).await;
    witness(&test, "cicada.event").await;
    test
}

async fn node(probe: &Probe) -> TestActors {
    node_with(probe, cicada::time::DEFAULT_RETENTION).await
}

/// Subscribes each of `flows` in order and runs what is due.
async fn subscribe(test: &TestActors, flows: &[&str]) {
    for flow in flows {
        test.send(&ev(), EventMsg::Subscribe { flow: (*flow).to_owned() }).await;
    }
    test.run_until_idle().await;
}

async fn flows(test: &TestActors) -> Vec<String> {
    test.state::<_, EventState>(&ev()).await.flows
}

/// Sends `message` to the event with the id `key` and no correlation id; runs nothing.
async fn publish_as(test: &TestActors, key: &str, message: EventMsg) -> Message {
    test.send_with(&ev(), message, SendOptions::new().id(id(key))).await
}

async fn outcome_of(test: &TestActors, message: &Message) -> (OutcomeStatus, String) {
    let outcome = test.outcome(message).await.expect("an outcome");
    (outcome.status, outcome.error.unwrap_or_default())
}

/// The `Failed` the witness was told of `message`.
async fn failure_of(test: &TestActors, message: &Message) -> cicada::Failed {
    told(test).await.into_iter().find(|f| f.message_id == message.id).expect("a Failed of the message")
}

/// Every flow run that exists, in address order.
async fn runs(test: &TestActors) -> Vec<String> {
    test.list("shop.").await.iter().map(|a| a.as_str().to_owned()).collect()
}

fn nested(depth: usize) -> Value {
    (0..depth).fold(Value::Null, |inner, _| Value::Array(vec![inner]))
}

// ---- subscriptions ----

/// Scenario: Subscription limits.
#[tokio::test(start_paused = true)]
async fn an_existing_subscription_at_the_limit_is_kept_once_and_a_101st_is_refused() {
    let test = node(&Probe::default()).await;
    let full: Vec<String> = (0..100).map(|i| format!("shop.flow{i}")).collect();
    let event = Event::named("full");
    test.create(&event, &EventState { flows: full.clone() }).await;
    let again = test.send(&event, EventMsg::Subscribe { flow: "shop.flow7".into() }).await;
    let extra = test.send(&event, EventMsg::Subscribe { flow: "shop.extra".into() }).await;
    test.run_until_idle().await;
    assert_eq!(outcome_of(&test, &again).await.0, OutcomeStatus::Done);
    let (status, error) = outcome_of(&test, &extra).await;
    assert_eq!(status, OutcomeStatus::Failed);
    assert!(error.contains("at most 100 flows"), "{error}");
    assert_eq!(failure_of(&test, &extra).await.cause, FailureCause::Refused);
    assert_eq!(test.state::<_, EventState>(&event).await.flows, full, "unchanged, without a duplicate");
}

/// Scenario: Invalid subscription state or command.
#[tokio::test(start_paused = true)]
async fn an_invalid_name_is_refused_and_an_absent_unsubscribe_changes_nothing() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email"]).await;
    let bad = [
        test.send(&ev(), EventMsg::Subscribe { flow: "Shop Email".into() }).await,
        test.send(&ev(), EventMsg::Subscribe { flow: "".into() }).await,
        test.send(&ev(), EventMsg::Unsubscribe { flow: "shop/email".into() }).await,
    ];
    let absent = test.send(&ev(), EventMsg::Unsubscribe { flow: "shop.other".into() }).await;
    test.run_until_idle().await;
    for message in &bad {
        let (status, error) = outcome_of(&test, message).await;
        assert_eq!(status, OutcomeStatus::Failed);
        assert!(error.contains("is no flow type"), "{error}");
        assert_eq!(failure_of(&test, message).await.cause, FailureCause::Refused);
    }
    assert_eq!(outcome_of(&test, &absent).await.0, OutcomeStatus::Done);
    assert_eq!(flows(&test).await, ["shop.email"]);
}

/// Scenario: Explicit startup subscription.
#[tokio::test(start_paused = true)]
async fn a_type_not_yet_registered_is_kept_and_a_repeated_startup_does_not_duplicate_it() {
    let test = node(&Probe::default()).await;
    for _ in 0..2 {
        test.create(&ev(), &EventState::default()).await;
        subscribe(&test, &["shop.later", "shop.email"]).await;
    }
    assert_eq!(flows(&test).await, ["shop.later", "shop.email"], "in order, each once");
    test.send(&ev(), EventMsg::Unsubscribe { flow: "shop.later".into() }).await;
    test.run_until_idle().await;
    assert_eq!(flows(&test).await, ["shop.email"]);
}

// ---- publications ----

/// Scenario: Two subscribed flows.
#[tokio::test(start_paused = true)]
async fn one_commit_creates_and_starts_a_run_of_each_subscribed_flow_keyed_by_the_publication_id() {
    let probe = Probe::default();
    let test = node(&probe).await;
    subscribe(&test, &["shop.email", "shop.fulfil"]).await;
    let opts = SendOptions::new().id(id("event-17")).correlation_id("c-1").header("x-tenant", "acme");
    let publish = test.send_with(&ev(), EventMsg::publish(json!({"order": 7})).unwrap(), opts).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the publication alone");
    assert_eq!(outcome_of(&test, &publish).await.0, OutcomeStatus::Done);
    assert_eq!(runs(&test).await, ["shop.email/event-17", "shop.fulfil/event-17"]);
    let initial = json!({"input": {"order": 7}, "journal": [], "status": "running", "armed": null});
    for run in [addr(&email("event-17")), addr(&fulfil("event-17"))] {
        assert_eq!(test.system().get_actor(&run).await.unwrap().unwrap().state, initial, "{run}");
        let pending = test.pending_at(&run).await;
        assert_eq!(pending.len(), 1, "one Continue for {run}");
        let start = &pending[0];
        assert_eq!((start.message_type.as_str(), &start.payload), ("Continue", &json!({})));
        assert_ne!(start.id, publish.id, "a new id");
        assert_eq!(start.from, Some(addr(&ev())), "sent by the event");
        assert_eq!(start.correlation_id.as_deref(), Some("c-1"));
        assert_eq!(start.retry, Some(shared_policy()));
        assert_eq!(start.retention, None);
        assert!(start.headers.is_empty(), "no header of the publication: {:?}", start.headers);
    }
    test.run_until_idle().await;
    assert_eq!(test.state::<_, FlowState<Value>>(&email("event-17")).await.status, FlowStatus::Done);
    assert_eq!(test.state::<_, FlowState<Value>>(&fulfil("event-17")).await.status, FlowStatus::Done);
    let mut keys = probe.keys();
    keys.sort();
    assert_eq!(keys, ["shop.email/event-17/send", "shop.fulfil/event-17/ship"]);
}

/// Scenario: General JSON input.
#[tokio::test(start_paused = true)]
async fn any_json_value_becomes_the_input_of_each_run() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email"]).await;
    for (key, input) in [("o", json!({"a": 1})), ("a", json!([1, 2])), ("s", json!("text")), ("n", Value::Null)] {
        publish_as(&test, key, EventMsg::Publish { input: input.clone() }).await;
        test.run_until_idle().await;
        let state: FlowState<Value> = test.state(&email(key)).await;
        assert_eq!((state.input, state.status), (input, FlowStatus::Done), "{key}");
    }
}

/// Scenario: Empty batch; and a batch is one run per flow, not one per item.
#[tokio::test(start_paused = true)]
async fn a_batch_starts_one_run_per_flow_with_its_items_also_an_empty_one() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email", "shop.batched"]).await;
    publish_as(&test, "b1", EventMsg::publish_batch([1, 2]).unwrap()).await;
    publish_as(&test, "b0", EventMsg::PublishBatch { items: vec![] }).await;
    test.run_until_idle().await;
    assert_eq!(runs(&test).await, ["shop.batched/b0", "shop.batched/b1", "shop.email/b0", "shop.email/b1"]);
    assert_eq!(test.state::<_, FlowState<Value>>(&email("b1")).await.input, json!({"items": [1, 2]}));
    assert_eq!(test.state::<_, FlowState<Value>>(&email("b0")).await.input, json!({"items": []}));
    let rows: FlowState<Rows> = test.state(&batched("b1")).await;
    assert_eq!((rows.result::<usize>("load").unwrap().unwrap(), rows.status), (2, FlowStatus::Done));
    assert_eq!(test.state::<_, FlowState<Rows>>(&batched("b0")).await.result::<usize>("load").unwrap().unwrap(), 0);
}

/// Scenario: No subscribers.
#[tokio::test(start_paused = true)]
async fn a_publication_without_subscriptions_does_nothing_and_a_later_subscription_does_not_replay_it() {
    let test = node(&Probe::default()).await;
    let version = test.version(&ev()).await;
    let first = test.send(&ev(), EventMsg::publish(json!({"order": 1})).unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(outcome_of(&test, &first).await.0, OutcomeStatus::Done);
    assert_eq!(test.version(&ev()).await, version, "nothing changes");
    subscribe(&test, &["shop.email"]).await;
    test.advance_time_by(HOUR).await;
    assert!(runs(&test).await.is_empty(), "nothing replayed");
}

// ---- keys ----

/// Scenario: Reserved actor keys. The ids are used again after the store's retention, as they are unique within it.
#[tokio::test(start_paused = true)]
async fn a_dot_id_is_refused_with_subscriptions_and_a_no_op_without() {
    let test = node_with(&Probe::default(), HOUR).await;
    let mut empty = Vec::new();
    for (key, message) in [(".", EventMsg::publish(1).unwrap()), ("..", EventMsg::publish_batch([1]).unwrap())] {
        empty.push(publish_as(&test, key, message).await);
    }
    test.run_until_idle().await;
    for message in &empty {
        assert_eq!(outcome_of(&test, message).await.0, OutcomeStatus::Done, "{}", message.id);
    }
    subscribe(&test, &["shop.email", "shop.fulfil"]).await;
    let version = test.version(&ev()).await;
    test.advance_time_by(2 * HOUR).await;
    for (key, message) in [(".", EventMsg::publish_batch([1]).unwrap()), ("..", EventMsg::publish(1).unwrap())] {
        let m = publish_as(&test, key, message).await;
        test.run_until_idle().await;
        let (status, error) = outcome_of(&test, &m).await;
        assert_eq!(status, OutcomeStatus::Failed);
        assert!(error.contains(&format!("the publication id `{key}` cannot key a run")), "{error}");
        assert_eq!(failure_of(&test, &m).await.cause, FailureCause::Refused);
    }
    assert!(runs(&test).await.is_empty(), "no run");
    assert_eq!(test.version(&ev()).await, version);
}

/// Scenario: Recovery with a new key.
#[tokio::test(start_paused = true)]
async fn a_refused_dot_id_recovers_under_the_new_id_of_a_retry_or_a_new_publication() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email"]).await;
    let refused = publish_as(&test, ".", EventMsg::publish(1).unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(outcome_of(&test, &refused).await.0, OutcomeStatus::Failed);
    let RetryResult::Accepted(retried) = test.system().retry_message(&refused.id).await.unwrap() else {
        panic!("not retried")
    };
    assert_ne!(retried.id, refused.id);
    publish_as(&test, "fresh", EventMsg::publish(1).unwrap()).await;
    test.run_until_idle().await;
    let mut expected = vec![format!("shop.email/{}", retried.id), "shop.email/fresh".to_owned()];
    expected.sort();
    assert_eq!(runs(&test).await, expected);
    assert_eq!(test.state::<_, FlowState<Value>>(&email(retried.id.as_str())).await.status, FlowStatus::Done);
}

/// Scenario: Percent-encoding is applied once.
#[tokio::test(start_paused = true)]
async fn raw_and_encoded_looking_ids_key_distinct_runs() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email"]).await;
    publish_as(&test, "a/b", EventMsg::publish(1).unwrap()).await;
    publish_as(&test, "a%2Fb", EventMsg::publish(2).unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(runs(&test).await, ["shop.email/a%252Fb", "shop.email/a%2Fb"]);
    assert_eq!(test.state::<_, FlowState<Value>>(&email("a/b")).await.input, json!(1));
    assert_eq!(test.state::<_, FlowState<Value>>(&email("a%2Fb")).await.input, json!(2));
}

// ---- context ----

/// Scenario: A flow with its own policy.
#[tokio::test(start_paused = true)]
async fn the_first_continue_has_the_crate_policy_and_the_armed_one_the_flows_with_the_correlation_id() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.fulfil"]).await;
    let opts = SendOptions::new().id(id("p")).correlation_id("c-9").retry(three_attempts());
    test.send_with(&ev(), EventMsg::publish(1).unwrap(), opts).await;
    test.system().run_due_turns(1).await.unwrap();
    let start = test.pending(&fulfil("p")).await.pop().unwrap();
    assert_eq!(start.retry, Some(shared_policy()), "not the publication's policy, nor the flow's");
    test.system().run_due_turns(1).await.unwrap();
    let armed_id = test.state::<_, FlowState<Value>>(&fulfil("p")).await.armed.expect("armed");
    let armed = test.pending(&fulfil("p")).await.into_iter().find(|m| m.id == armed_id).unwrap();
    assert_eq!(armed.retry, Some(three_attempts()), "the flow's own policy");
    assert_eq!(armed.correlation_id.as_deref(), Some("c-9"), "the correlation id follows the chain");
}

/// Scenario: Batch correlation; and Direct batcher composition.
#[tokio::test(start_paused = true)]
async fn a_batcher_publishes_its_items_as_one_run_per_flow_without_a_correlation_id() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.batched", "shop.email"]).await;
    let bat = Batcher::named("rows");
    test.create(&bat, &BatcherState::new(&ev(), "PublishBatch", 2, HOUR).unwrap()).await;
    test.send(&bat, BatcherMsg::add(1).unwrap()).await;
    test.send(&bat, BatcherMsg::add(2).unwrap()).await;
    for _ in 0..3 {
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "two Adds and the publication");
    }
    let created = runs(&test).await;
    assert_eq!(created.len(), 2, "one run per flow for the batch: {created:?}");
    let key = created[0].rsplit_once('/').unwrap().1.to_owned();
    for start in [test.pending(&batched(&key)).await, test.pending(&email(&key)).await] {
        assert_eq!(start[0].correlation_id, None);
    }
    test.run_until_idle().await;
    assert_eq!(test.state::<_, FlowState<Rows>>(&batched(&key)).await.input, Rows { items: vec![1, 2] });
    assert_eq!(test.state::<_, FlowState<Value>>(&email(&key)).await.input, json!({"items": [1, 2]}));
}

// ---- failures ----

/// Scenario: Stale subscription after a valid one; and Redrive a failed publication.
#[tokio::test(start_paused = true)]
async fn a_stale_subscription_blocks_every_run_and_a_redrive_after_its_removal_uses_a_new_id() {
    let test = node(&Probe::default()).await;
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    test.system().watch(&WatchTarget::parse("cicada.event").unwrap(), &addr(&DeadLetters::default())).await.unwrap();
    subscribe(&test, &["shop.email", "shop.gone"]).await;
    let failed = publish_as(&test, "e1", EventMsg::publish(1).unwrap()).await;
    test.run_until_idle().await;
    let (status, error) = outcome_of(&test, &failed).await;
    assert_eq!(status, OutcomeStatus::Failed);
    assert!(error.contains("no registered actor type shop.gone"), "{error}");
    assert_eq!(failure_of(&test, &failed).await.cause, FailureCause::Refused);
    assert!(runs(&test).await.is_empty(), "neither run");
    test.send(&ev(), EventMsg::Unsubscribe { flow: "shop.gone".into() }).await;
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    test.run_until_idle().await;
    let created = runs(&test).await;
    assert_eq!(created.len(), 1, "{created:?}");
    assert!(created[0].starts_with("shop.email/") && created[0] != "shop.email/e1", "a new id: {created:?}");
}

/// Scenario: An ordinary actor is subscribed.
#[tokio::test(start_paused = true)]
async fn an_actor_without_continue_is_refused_without_partial_creates() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email", "worker"]).await;
    let m = publish_as(&test, "e1", EventMsg::publish(json!({})).unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(failure_of(&test, &m).await.cause, FailureCause::Refused);
    assert!(test.list("").await.iter().all(|a| !a.as_str().ends_with("/e1")), "no run and no worker");
}

/// Scenario: Failure after publication.
#[tokio::test(start_paused = true)]
async fn a_run_that_fails_after_the_publication_leaves_it_done_and_recovers_by_continue() {
    let probe = Probe::default();
    probe.failing.store(3, Ordering::SeqCst);
    let test = node(&probe).await;
    test.system().watch(&WatchTarget::parse("shop.fulfil").unwrap(), &addr(&Witness)).await.unwrap();
    subscribe(&test, &["shop.email", "shop.fulfil"]).await;
    let publish = publish_as(&test, "e1", EventMsg::publish(1).unwrap()).await;
    test.advance_time_by(HOUR).await;
    assert_eq!(outcome_of(&test, &publish).await.0, OutcomeStatus::Done);
    let fault = told(&test).await.into_iter().find(|f| f.actor == addr(&fulfil("e1"))).expect("the run failed");
    assert_eq!(fault.attempts, 3, "the flow's policy");
    assert_eq!(test.state::<_, FlowState<Value>>(&email("e1")).await.status, FlowStatus::Done, "unaffected");
    test.send(&fulfil("e1"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, FlowState<Value>>(&fulfil("e1")).await.status, FlowStatus::Done, "recovered");
}

/// Scenario: Invalid hosted input.
#[tokio::test(start_paused = true)]
async fn input_that_a_hosted_flow_cannot_decode_refuses_the_whole_publication() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email", "shop.typed"]).await;
    let m = publish_as(&test, "e1", EventMsg::publish(json!({"order": "seven"})).unwrap()).await;
    test.run_until_idle().await;
    let (status, error) = outcome_of(&test, &m).await;
    assert_eq!(status, OutcomeStatus::Failed);
    assert!(error.contains("the initial state cannot be decoded"), "{error}");
    assert_eq!(failure_of(&test, &m).await.cause, FailureCause::Refused);
    assert!(runs(&test).await.is_empty());
    publish_as(&test, "e2", EventMsg::publish(Order { order: 7 }).unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, FlowState<Order>>(&typed("e2")).await.status, FlowStatus::Done);
}

/// Scenario: Invalid unhosted input. The event's node does not host `shop.typed`; another node on the store does.
#[tokio::test(start_paused = true)]
async fn input_that_an_unhosted_flow_cannot_decode_commits_and_fails_its_run_with_the_cause_state() {
    let probe = Probe::default();
    let p = probe.clone();
    let test = TestActors::builder()
        .with(move |node| node.host(EventActor).host(FlowActor::new(Email(p))).host(WitnessActor))
        .build()
        .await
        .unwrap();
    let billing = ActorSystem::builder(test.store().clone()).host(FlowActor::new(Typed)).build();
    billing.initialize().await.unwrap();
    test.create(&ev(), &EventState::default()).await;
    witness(&test, "shop.typed").await;
    subscribe(&test, &["shop.typed", "shop.email"]).await;
    let m = publish_as(&test, "e1", EventMsg::publish(json!({"order": "seven"})).unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(outcome_of(&test, &m).await.0, OutcomeStatus::Done, "committed");
    assert_eq!(runs(&test).await, ["shop.email/e1", "shop.typed/e1"]);
    while billing.run_due_turns(10).await.unwrap() > 0 {}
    test.run_until_idle().await;
    let fault = told(&test).await.into_iter().find(|f| f.actor == addr(&typed("e1"))).expect("the run failed");
    assert_eq!(fault.cause, FailureCause::State);
    assert_eq!(test.state::<_, FlowState<Value>>(&email("e1")).await.status, FlowStatus::Done, "unaffected");
}

// ---- deduplication ----

/// Scenario: Same id inside the window.
#[tokio::test(start_paused = true)]
async fn a_repeated_id_inside_the_retention_adds_nothing() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email"]).await;
    publish_as(&test, "e1", EventMsg::publish(1).unwrap()).await;
    test.run_until_idle().await;
    let version = test.version(&email("e1")).await;
    let again = test.system().send_with(&ev(), EventMsg::publish(2).unwrap(), SendOptions::new().id(id("e1"))).await;
    assert!(matches!(again.unwrap(), SendResult::Duplicate(_)));
    assert_eq!(test.run_until_idle().await, 0, "no turn, so no create and no Continue");
    assert_eq!(test.version(&email("e1")).await, version);
}

/// Scenario: Same id after the window; and New subscription after expiry.
#[tokio::test(start_paused = true)]
async fn a_repeated_id_after_the_retention_keeps_existing_runs_and_starts_a_new_subscription() {
    let probe = Probe::default();
    let test = node_with(&probe, HOUR).await;
    subscribe(&test, &["shop.email"]).await;
    publish_as(&test, "e1", EventMsg::publish(1).unwrap()).await;
    test.run_until_idle().await;
    let first: FlowState<Value> = test.state(&email("e1")).await;
    test.advance_time_by(2 * HOUR).await;
    subscribe(&test, &["shop.fulfil"]).await;
    let again = publish_as(&test, "e1", EventMsg::publish(2).unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(outcome_of(&test, &again).await.0, OutcomeStatus::Done);
    assert_eq!(test.state::<_, FlowState<Value>>(&email("e1")).await, first, "the first input and journal stay");
    assert_eq!(runs(&test).await, ["shop.email/e1", "shop.fulfil/e1"], "no extra address");
    assert_eq!(test.state::<_, FlowState<Value>>(&fulfil("e1")).await.input, json!(2), "the new subscription's run");
    assert_eq!(probe.keys().iter().filter(|k| k.starts_with("shop.email")).count(), 1, "the done run ignored it");
}

/// Scenario: Prospective input is still checked.
#[tokio::test(start_paused = true)]
async fn a_repeated_id_after_the_retention_with_invalid_input_is_refused_although_the_run_exists() {
    let test = node_with(&Probe::default(), HOUR).await;
    subscribe(&test, &["shop.typed"]).await;
    publish_as(&test, "e1", EventMsg::publish(Order { order: 7 }).unwrap()).await;
    test.run_until_idle().await;
    test.advance_time_by(2 * HOUR).await;
    let again = publish_as(&test, "e1", EventMsg::publish(json!({"order": "x"})).unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(failure_of(&test, &again).await.cause, FailureCause::Refused);
    assert_eq!(test.state::<_, FlowState<Order>>(&typed("e1")).await.input, Order { order: 7 });
}

// ---- composition and lifecycle ----

/// Scenario: Two scheduler slots; and a rate limiter's and a debouncer's forwards, each under a fresh id.
#[tokio::test(start_paused = true)]
async fn each_scheduler_slot_and_each_forward_starts_a_fresh_run() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email"]).await;
    let nightly = Scheduler::named("nightly");
    let publish = EventMsg::publish(json!({"report": "daily"})).unwrap();
    test.create(&nightly, &SchedulerState::new(Schedule::daily(2, 0).unwrap(), &ev(), publish.clone()).unwrap()).await;
    test.send(&nightly, SchedulerMsg::Start).await;
    test.advance_time_by(48 * HOUR).await;
    assert_eq!(runs(&test).await.len(), 2, "two slots, two runs");
    let api = RateLimiter::named("api");
    test.create(&api, &RateLimiterState::new(1, SECOND, 1, HOUR).unwrap()).await;
    test.send(&api, RateLimiterMsg::submit(&ev(), publish.clone(), None).unwrap()).await;
    let quiet = Debouncer::named("quiet");
    test.create(&quiet, &DebouncerState::new(SECOND, HOUR).unwrap()).await;
    test.send(&quiet, DebouncerMsg::submit(&ev(), publish, None).unwrap()).await;
    test.advance_time_by(HOUR).await;
    let created = runs(&test).await;
    assert_eq!(created.len(), 4, "{created:?}");
    for run in &created {
        let state: FlowState<Value> = test.state(&email(run.rsplit_once('/').unwrap().1)).await;
        assert_eq!((state.input, state.status), (json!({"report": "daily"}), FlowStatus::Done));
    }
}

/// Scenario: Removing an event. The `Unsubscribe` runs while the runs of a publication exist and leaves them; then the
/// event is deleted, and the runs still finish.
#[tokio::test(start_paused = true)]
async fn unsubscribing_or_deleting_the_event_leaves_its_runs() {
    let probe = Probe::default();
    probe.failing.store(1, Ordering::SeqCst);
    let test = node(&probe).await;
    subscribe(&test, &["shop.email", "shop.fulfil"]).await;
    publish_as(&test, "e1", EventMsg::publish(1).unwrap()).await;
    test.system().run_due_turns(1).await.unwrap();
    assert_eq!(runs(&test).await, ["shop.email/e1", "shop.fulfil/e1"]);
    for run in [addr(&email("e1")), addr(&fulfil("e1"))] {
        test.system().pause_mailbox(&run).await.unwrap();
    }
    let unsubscribe = test.send(&ev(), EventMsg::Unsubscribe { flow: "shop.fulfil".into() }).await;
    test.run_until_idle().await;
    assert_eq!(outcome_of(&test, &unsubscribe).await.0, OutcomeStatus::Done);
    assert_eq!(flows(&test).await, ["shop.email"]);
    assert_eq!(runs(&test).await, ["shop.email/e1", "shop.fulfil/e1"], "the runs are kept");
    assert_eq!(test.state::<_, FlowState<Value>>(&fulfil("e1")).await.status, FlowStatus::Running);

    test.system().delete_actor(&addr(&ev())).await.unwrap();
    for run in [addr(&email("e1")), addr(&fulfil("e1"))] {
        test.system().resume_mailbox(&run).await.unwrap();
    }
    test.advance_time_by(HOUR).await;
    assert!(!test.exists(&ev()).await);
    assert_eq!(test.state::<_, FlowState<Value>>(&fulfil("e1")).await.status, FlowStatus::Done, "ran on");
    assert_eq!(test.state::<_, FlowState<Value>>(&email("e1")).await.status, FlowStatus::Done);
}

// ---- depth ----

/// Scenario: Batch depth boundary; and an input 63 levels deep.
#[tokio::test(start_paused = true)]
async fn a_batch_item_may_be_61_deep_and_one_62_deep_refuses_the_publication() {
    let test = node(&Probe::default()).await;
    subscribe(&test, &["shop.email"]).await;
    publish_as(&test, "i61", EventMsg::PublishBatch { items: vec![nested(61)] }).await;
    publish_as(&test, "p63", EventMsg::Publish { input: nested(63) }).await;
    let deep = publish_as(&test, "i62", EventMsg::PublishBatch { items: vec![nested(62)] }).await;
    test.run_until_idle().await;
    assert_eq!(runs(&test).await, ["shop.email/i61", "shop.email/p63"]);
    let (status, error) = outcome_of(&test, &deep).await;
    assert_eq!(status, OutcomeStatus::Failed);
    assert!(error.contains("The initial state is nested deeper than 64 levels"), "{error}");
    let empty = Event::named("empty");
    test.create(&empty, &EventState::default()).await;
    let batch = EventMsg::PublishBatch { items: vec![nested(62)] };
    let none = test.send_with(&empty, batch, SendOptions::new().id(id("j62"))).await;
    test.run_until_idle().await;
    assert_eq!(outcome_of(&test, &none).await.0, OutcomeStatus::Done, "no run, no check");
}

/// Scenario: A refused batch publication (ADR 0043). The publication of an item 62 levels deep, a payload of 64, fails
/// for the state of its run; the dead letters keep it as JSON text, and its redrive sends the same payload, which fails
/// again for the same reason and becomes a letter of its own: keeping the payload does not make the run's state valid.
#[tokio::test(start_paused = true)]
async fn a_refused_batch_publication_is_kept_and_redriven_unchanged_and_fails_again_for_the_state_of_its_run() {
    let test = node(&Probe::default()).await;
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    test.system().watch(&WatchTarget::parse("cicada.event").unwrap(), &addr(&DeadLetters::default())).await.unwrap();
    subscribe(&test, &["shop.email"]).await;
    let refused = publish_as(&test, "i62", EventMsg::PublishBatch { items: vec![nested(62)] }).await;
    test.run_until_idle().await;
    assert_eq!(outcome_of(&test, &refused).await.0, OutcomeStatus::Failed);
    let letters: DeadLettersState = test.state(&DeadLetters::default()).await;
    let kept = &letters.letters[&refused.id];
    assert!(matches!(kept.payload, LetterPayload::Json(_)), "{:?}", kept.payload);
    assert_eq!(kept.payload.value().unwrap(), refused.payload);

    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: refused.id.clone() }).await;
    test.run_until_idle().await;
    let letters: DeadLettersState = test.state(&DeadLetters::default()).await;
    let sent = letters.letters[&refused.id].redriven_as.clone().expect("the redrive sent it");
    let Some(cicada::store::MessageStatus::Finished(again)) = test.system().status(&sent).await.unwrap() else {
        panic!("the redriven publication has no outcome")
    };
    assert_eq!(again.status, OutcomeStatus::Failed);
    assert!(again.error.unwrap_or_default().contains("The initial state is nested deeper than 64 levels"));
    assert_eq!(again.envelope.payload, refused.payload, "the same payload");
    assert_eq!(letters.letters[&sent].payload.value().unwrap(), refused.payload, "a letter of its own");
    assert!(runs(&test).await.is_empty(), "no run");
}
