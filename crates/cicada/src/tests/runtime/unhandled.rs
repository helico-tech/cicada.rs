//! A message that cannot be decoded at delivery ends `failed` with the reason, without a retry; no handler runs.

use std::time::Duration;

use super::common::*;
use crate::plugin::{HandlerErrorContext, HookResult, Plugin};
use crate::prelude::*;
use crate::store::{Message, OutcomeStatus, OutgoingMessage};
use crate::testing::TestActors;
use async_trait::async_trait;
use serde_json::{Value, json};

fn a() -> Counter {
    Counter::new("a")
}

async fn raw_enqueue(test: &TestActors, message_type: &str, payload: Value) -> Message {
    enqueue(test.store(), OutgoingMessage::new(a().address(), message_type, payload)).await
}

#[tokio::test(start_paused = true)]
async fn an_undecodable_message_ends_failed_with_the_decode_reason() {
    let test = TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    let bad = raw_enqueue(&test, "Add", json!({"n": "x"})).await;
    let unknown = raw_enqueue(&test, "Nope", json!({})).await;
    let not_object = raw_enqueue(&test, "Add", json!([])).await;
    test.run_until_idle().await;
    let bad = test.outcome(&bad).await.unwrap();
    assert_eq!(bad.status, OutcomeStatus::Failed);
    assert!(bad.error.as_deref().unwrap().starts_with("Payload of 'Add' cannot be decoded"), "{:?}", bad.error);
    assert_eq!(test.outcome(&unknown).await.unwrap().error.as_deref(), Some("Unknown message type 'Nope'"));
    assert_eq!(
        test.outcome(&not_object).await.unwrap().error.as_deref(),
        Some("Payload of 'Add' must be a JSON object")
    );
}

#[tokio::test(start_paused = true)]
async fn an_unknown_type_name_fails_and_leaves_the_state_as_it_was() {
    let test = TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap();
    test.create(&a(), &CounterState::total(7)).await;
    let unknown = raw_enqueue(&test, "Nope", json!({"n": 1})).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&unknown).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert!(outcome.error.as_deref().unwrap().contains("Nope"), "{:?}", outcome.error);
    assert_eq!(test.state::<_, CounterState>(&a()).await, CounterState::total(7));
    assert_eq!(test.version(&a()).await, 0);
}

#[tokio::test(start_paused = true)]
async fn the_message_after_an_undecodable_one_is_handled() {
    let test = TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    let bad = raw_enqueue(&test, "Add", json!({})).await;
    let good = test.send(&a(), CounterMsg::Add { n: 2 }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&bad).await.unwrap().status, OutcomeStatus::Failed);
    assert_eq!(test.outcome(&good).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, CounterState>(&a()).await.total, 2);
}

struct Observer(Events);

#[async_trait]
impl Plugin for Observer {
    fn name(&self) -> &str {
        "Observer"
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
        self.0.push(ctx.info().message().message_type.clone());
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn an_undecodable_message_is_no_handler_failure_and_is_not_retried() {
    let hook_runs = Events::default();
    let observer = hook_runs.clone();
    let retry = RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    let test =
        TestActors::builder().with(move |n| n.host(CounterActor).install(Observer(observer))).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    let mut ids = Vec::new();
    for (t, payload) in [("Plain", json!({})), ("Add", json!({"n": "x"})), ("Add", json!([]))] {
        let mut m = OutgoingMessage::new(a().address(), t, payload);
        m.retry = Some(retry.clone());
        ids.push(enqueue(test.store(), m).await.id);
    }
    test.run_until_idle().await;
    let mut outcomes = Vec::new();
    for id in &ids {
        outcomes.push(test.system().outcome(id).await.unwrap().unwrap());
    }
    assert!(outcomes.iter().all(|o| o.status == OutcomeStatus::Failed));
    let errors: Vec<String> = outcomes.iter().map(|o| o.error.clone().unwrap()).collect();
    assert_eq!(errors[0], "Unknown message type 'Plain'");
    assert!(errors[1].starts_with("Payload of 'Add' cannot be decoded"), "{}", errors[1]);
    assert_eq!(errors[2], "Payload of 'Add' must be a JSON object");
    assert!(outcomes.iter().all(|o| o.envelope.attempt == 1), "one attempt each");
    assert!(outcomes.iter().all(|o| o.envelope.failures == 0), "never retried");
    assert!(hook_runs.all().is_empty(), "no handler ran: no on_handler_error");
    assert_eq!(test.state::<_, CounterState>(&a()).await, CounterState::default());
}
