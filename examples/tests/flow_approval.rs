//! Example 17: a purchase that waits for a person's approval; the run parks with no continuation, the decision's reply
//! closes the wait, and a decision for a deleted run rejects the approvals actor's commit.

use std::sync::{Arc, Mutex};

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada_actors::{FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, JournalEntry, WaitOutcome};
use cicada_examples::flow_approval::{
    Approvals, ApprovalsActor, ApprovalsMsg, ApprovalsState, Decision, Orders, PurchaseFlow, PurchaseInput, purchase,
};
use cicada_examples::steps::ServiceError;
use cicada_test::TestActors;

/// Notes each call as `<what> <idempotency key>`.
#[derive(Clone, Default)]
struct FakeOrders {
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeOrders {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Orders for FakeOrders {
    async fn place(&self, item: &str, idempotency_key: &str) -> Result<String, ServiceError> {
        self.calls.lock().unwrap().push(format!("place {item} {idempotency_key}"));
        Ok("order-1".into())
    }

    async fn reject(&self, item: &str, by: &str, idempotency_key: &str) -> Result<(), ServiceError> {
        self.calls.lock().unwrap().push(format!("reject {item} by {by} {idempotency_key}"));
        Ok(())
    }
}

type Run = FlowRun<PurchaseFlow<FakeOrders>>;
type State = FlowState<PurchaseInput>;

fn design() -> Approvals {
    Approvals { team: "design".into() }
}

async fn harness(orders: &FakeOrders) -> TestActors {
    let orders = orders.clone();
    TestActors::builder()
        .with(move |node| node.host(ApprovalsActor).host(FlowActor::new(PurchaseFlow { orders })))
        .build()
        .await
        .unwrap()
}

/// Starts the purchase `key` and runs until it is parked at its wait.
async fn parked(test: &TestActors, key: &str) -> Run {
    let run: Run = purchase(key);
    let input = PurchaseInput { team: "design".into(), item: "monitor".into(), cents: 39_900 };
    test.create(&run, &FlowState::new(input)).await;
    test.send(&run, FlowMsg::Continue).await;
    test.run_until_idle().await;
    let state: State = test.state(&run).await;
    assert_eq!((state.status, state.armed.clone()), (FlowStatus::Running, None), "parked, nothing armed");
    assert!(matches!(state.journal.last(), Some(JournalEntry::Wait { outcome: None, .. })), "an open wait");
    assert!(test.pending(&run).await.is_empty(), "nothing polls");
    run
}

/// The pending request of the approvals of design.
async fn pending_request(test: &TestActors) -> MessageId {
    let approvals: ApprovalsState = test.state(&design()).await;
    approvals.pending.last().expect("a pending purchase").reply.request.clone()
}

#[tokio::test(start_paused = true)]
async fn an_approval_closes_the_wait_and_the_flow_places_the_order() {
    let orders = FakeOrders::default();
    let test = harness(&orders).await;
    let run = parked(&test, "p-17").await;
    let approvals: ApprovalsState = test.state(&design()).await;
    assert_eq!((approvals.pending.len(), approvals.pending[0].reply.run.as_str()), (1, "flows.purchase/p-17"));
    assert!(orders.calls().is_empty());

    let request = pending_request(&test).await;
    test.send(&design(), ApprovalsMsg::Decide { request: request.clone(), approved: true, by: "ann".into() }).await;
    test.run_until_idle().await;
    let state: State = test.state(&run).await;
    assert_eq!(state.status, FlowStatus::Done);
    assert_eq!(state.replies[0].value, serde_json::json!({"approved": true, "by": "ann"}));
    let Some(JournalEntry::Wait { outcome, received, .. }) = state.journal.get(1) else { panic!("{state:?}") };
    assert_eq!((*outcome, received.as_slice()), (Some(WaitOutcome::Received), [request].as_slice()));
    assert_eq!(orders.calls(), ["place monitor flows.purchase/p-17/place-order"]);
    assert!(test.state::<_, ApprovalsState>(&design()).await.pending.is_empty(), "decided");
}

#[tokio::test(start_paused = true)]
async fn a_rejection_tells_the_requester() {
    let orders = FakeOrders::default();
    let test = harness(&orders).await;
    let run = parked(&test, "p-18").await;
    let request = pending_request(&test).await;
    test.send(&design(), ApprovalsMsg::Decide { request, approved: false, by: "bo".into() }).await;
    test.run_until_idle().await;
    let state: State = test.state(&run).await;
    assert_eq!(state.status, FlowStatus::Done);
    let decision: Decision = serde_json::from_value(state.replies[0].value.clone()).unwrap();
    assert_eq!(decision, Decision { approved: false, by: "bo".into() });
    assert_eq!(orders.calls(), ["reject monitor by bo flows.purchase/p-18/notify-rejected"]);
}

/// A reply that does not come from the approvals actor, here sent from outside, is ignored: the run stays parked.
#[tokio::test(start_paused = true)]
async fn a_reply_from_anyone_else_is_ignored() {
    let orders = FakeOrders::default();
    let test = harness(&orders).await;
    let run = parked(&test, "p-19").await;
    let request = pending_request(&test).await;
    let forged = FlowMsg::Reply { request, value: serde_json::json!({"approved": true, "by": "mallory"}) };
    let sent = test.send(&run, forged).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&sent).await.unwrap().status, OutcomeStatus::Done);
    let state: State = test.state(&run).await;
    assert!(state.replies.is_empty() && state.armed.is_none(), "still parked");
    assert!(orders.calls().is_empty());
}

/// A decision for a run that was deleted: the approvals actor's whole commit is rejected, so the purchase stays
/// pending and the run is not recreated.
#[tokio::test(start_paused = true)]
async fn a_decision_for_a_deleted_run_rejects_the_commit_of_the_approvals() {
    let orders = FakeOrders::default();
    let test = harness(&orders).await;
    let run = parked(&test, "p-20").await;
    let request = pending_request(&test).await;
    test.system().delete_actor(&run.address().unwrap()).await.unwrap();
    let decide = test.send(&design(), ApprovalsMsg::Decide { request, approved: true, by: "ann".into() }).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&decide).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed, "the commit is rejected");
    assert_eq!(test.state::<_, ApprovalsState>(&design()).await.pending.len(), 1, "still pending");
    assert!(!test.exists(&run).await, "not recreated");
    assert!(orders.calls().is_empty());
}
