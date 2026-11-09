//! Messages that cannot be decoded at delivery go to the type's unhandled policy.

mod common;

use std::time::Duration;

use async_trait::async_trait;
use cicada::plugin::{HandlerErrorContext, HookResult, Plugin};
use cicada::prelude::*;
use cicada::store::{Message, OutcomeStatus, OutgoingMessage};
use cicada_test::TestActors;
use common::*;
use serde_json::{Value, json};

fn a() -> Counter {
    Counter::new("a")
}

async fn raw_enqueue(test: &TestActors, message_type: &str, payload: Value) -> Message {
    enqueue(&**test.store(), OutgoingMessage::new(a().address(), message_type, payload)).await
}

#[tokio::test(start_paused = true)]
async fn fail_outcome_failed_with_the_decode_reason() {
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

/// A custom policy that changes state, then drops.
struct DroppingCounter;

impl Actor for DroppingCounter {
    type Ref = Counter;
    type State = CounterState;
    type Message = CounterMsg;
    const UNHANDLED: Unhandled = Unhandled::Custom;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        handle_counter(turn, message).await
    }

    async fn on_unhandled(&self, turn: &mut Turn<Self>, _reason: String) -> Result<UnhandledDecision, HandlerError> {
        turn.state_mut().log("x");
        Ok(UnhandledDecision::Drop)
    }
}

#[tokio::test(start_paused = true)]
async fn drop_outcome_done_and_no_effects() {
    let test = TestActors::builder().with(|n| n.host(DroppingCounter)).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    let bad = raw_enqueue(&test, "Add", json!({})).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&bad).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, CounterState>(&a()).await, CounterState::default());
    assert_eq!(test.version(&a()).await, 0);
}

#[tokio::test(start_paused = true)]
async fn drop_policy_outcome_done() {
    counter_type!(DropCounter, Unhandled::Drop);
    let test = TestActors::builder().with(|n| n.host(DropCounter)).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    let bad = raw_enqueue(&test, "Add", json!({})).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&bad).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.version(&a()).await, 0);
}

struct LoggingCounter;

impl Actor for LoggingCounter {
    type Ref = Counter;
    type State = CounterState;
    type Message = CounterMsg;
    const UNHANDLED: Unhandled = Unhandled::Custom;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        handle_counter(turn, message).await
    }

    async fn on_unhandled(&self, turn: &mut Turn<Self>, reason: String) -> Result<UnhandledDecision, HandlerError> {
        let entry = format!("unhandled:{}:{reason}", turn.message().message_type);
        turn.state_mut().log(entry);
        Ok(UnhandledDecision::Handled)
    }
}

#[tokio::test(start_paused = true)]
async fn custom_receives_the_raw_envelope_and_its_effects_are_committed() {
    let test = TestActors::builder().with(|n| n.host(LoggingCounter)).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    let bad = raw_enqueue(&test, "Mystery", json!({})).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&bad).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, CounterState>(&a()).await.log, vec!["unhandled:Mystery:Unknown message type 'Mystery'"]);
}

struct DecidingCounter;

impl Actor for DecidingCounter {
    type Ref = Counter;
    type State = CounterState;
    type Message = CounterMsg;
    const UNHANDLED: Unhandled = Unhandled::Custom;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        handle_counter(turn, message).await
    }

    async fn on_unhandled(&self, turn: &mut Turn<Self>, _reason: String) -> Result<UnhandledDecision, HandlerError> {
        if turn.message().message_type == "A" {
            Ok(UnhandledDecision::Fail("custom reason".into()))
        } else {
            Err(HandlerError::msg("policy broke"))
        }
    }
}

#[tokio::test(start_paused = true)]
async fn custom_fail_with_own_reason_and_a_failing_custom_policy_fails_the_turn() {
    let test = TestActors::builder().with(|n| n.host(DecidingCounter)).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    let first = raw_enqueue(&test, "A", json!({})).await;
    let second = raw_enqueue(&test, "B", json!({})).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&first).await.unwrap().error.as_deref(), Some("custom reason"));
    assert_eq!(test.outcome(&second).await.unwrap().error.as_deref(), Some("policy broke"));
}

struct Observer(Events);

#[async_trait]
impl Plugin for Observer {
    fn name(&self) -> &str {
        "Observer"
    }

    async fn on_handler_error(&self, ctx: &mut HandlerErrorContext<'_>) -> HookResult {
        self.0.push(ctx.info().message().message_type.clone());
        Ok(())
    }
}

struct FailingPolicyCounter;

impl Actor for FailingPolicyCounter {
    type Ref = Counter;
    type State = CounterState;
    type Message = CounterMsg;
    const UNHANDLED: Unhandled = Unhandled::Custom;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        handle_counter(turn, message).await
    }

    async fn on_unhandled(&self, turn: &mut Turn<Self>, reason: String) -> Result<UnhandledDecision, HandlerError> {
        let message_type = turn.message().message_type.clone();
        turn.state_mut().log(format!("unhandled:{message_type}"));
        Ok(UnhandledDecision::Fail(match message_type.as_str() {
            "Plain" => reason,
            "Reason" => "own reason".into(),
            _ => "own reason with cause".into(),
        }))
    }
}

#[tokio::test(start_paused = true)]
async fn fail_decisions_are_no_failure_no_hooks_no_retry_effects_discarded() {
    let hook_runs = Events::default();
    let observer = hook_runs.clone();
    let retry = RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    let test = TestActors::builder()
        .with(move |n| n.host(ActorType::new(FailingPolicyCounter).install(Observer(observer))))
        .build()
        .await
        .unwrap();
    test.create(&a(), &CounterState::default()).await;
    let mut ids = Vec::new();
    for t in ["Plain", "Reason", "Caused"] {
        let mut m = OutgoingMessage::new(a().address(), t, json!({}));
        m.retry = Some(retry.clone());
        ids.push(enqueue(&**test.store(), m).await.id);
    }
    test.run_until_idle().await;
    let mut outcomes = Vec::new();
    for id in &ids {
        outcomes.push(test.system().outcome(id).await.unwrap().unwrap());
    }
    assert!(outcomes.iter().all(|o| o.status == OutcomeStatus::Failed));
    assert_eq!(
        outcomes.iter().map(|o| o.error.clone().unwrap()).collect::<Vec<_>>(),
        vec!["Unknown message type 'Plain'", "own reason", "own reason with cause"]
    );
    assert!(outcomes.iter().all(|o| o.envelope.failures == 0), "never retried");
    assert!(hook_runs.all().is_empty(), "a decision, not a failure: no on_handler_error");
    assert_eq!(test.state::<_, CounterState>(&a()).await, CounterState::default(), "effects discarded");
}
