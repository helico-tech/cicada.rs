//! Example 17: a purchase that waits for a person's approval for at most a week; the run waits with only its deadline's
//! continuation, the decision's reply closes the wait, an undecided purchase expires and ignores a later decision, and
//! a decision for a deleted run rejects the approvals actor's commit.

use std::sync::{Arc, Mutex};

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada_actors::{FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, JournalEntry, WaitOutcome};
use cicada_examples::flow_approval::{
    Approvals, ApprovalsActor, ApprovalsMsg, ApprovalsState, DECISION_TIMEOUT, Decision, Orders, PurchaseFlow,
    PurchaseInput, purchase,
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

    async fn expire(&self, item: &str, idempotency_key: &str) -> Result<(), ServiceError> {
        self.calls.lock().unwrap().push(format!("expire {item} {idempotency_key}"));
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

/// Starts the purchase `key` and runs until it waits at its open wait, whose deadline is a week after the claim of
/// its turn: its one pending message is the armed continuation, due then.
async fn waiting(test: &TestActors, key: &str) -> Run {
    let run: Run = purchase(key);
    let input = PurchaseInput { team: "design".into(), item: "monitor".into(), cents: 39_900 };
    test.create(&run, &FlowState::new(input)).await;
    test.send(&run, FlowMsg::Continue).await;
    let deadline = cicada::time::add(test.now(), DECISION_TIMEOUT).unwrap();
    test.run_until_idle().await;
    let state: State = test.state(&run).await;
    assert_eq!(state.status, FlowStatus::Running);
    let Some(JournalEntry::Wait { until, outcome: None, .. }) = state.journal.last() else { panic!("{state:?}") };
    assert_eq!(*until, Some(deadline), "an open wait with its deadline");
    let pending = test.pending(&run).await;
    assert_eq!(pending.len(), 1, "nothing polls");
    assert_eq!((Some(&pending[0].id), pending[0].deliver_at), (state.armed.as_ref(), deadline), "due at the deadline");
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
    let run = waiting(&test, "p-17").await;
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
    let run = waiting(&test, "p-18").await;
    let request = pending_request(&test).await;
    test.send(&design(), ApprovalsMsg::Decide { request, approved: false, by: "bo".into() }).await;
    test.run_until_idle().await;
    let state: State = test.state(&run).await;
    assert_eq!(state.status, FlowStatus::Done);
    let decision: Decision = serde_json::from_value(state.replies[0].value.clone()).unwrap();
    assert_eq!(decision, Decision { approved: false, by: "bo".into() });
    assert_eq!(orders.calls(), ["reject monitor by bo flows.purchase/p-18/notify-rejected"]);
}

/// A reply that does not come from the approvals actor, here sent from outside, is ignored: the run still waits.
#[tokio::test(start_paused = true)]
async fn a_reply_from_anyone_else_is_ignored() {
    let orders = FakeOrders::default();
    let test = harness(&orders).await;
    let run = waiting(&test, "p-19").await;
    let before: State = test.state(&run).await;
    let request = pending_request(&test).await;
    let forged = FlowMsg::Reply { request, value: serde_json::json!({"approved": true, "by": "mallory"}) };
    let sent = test.send(&run, forged).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&sent).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, State>(&run).await, before, "still waiting");
    assert!(orders.calls().is_empty());
}

/// Nobody decides within the week: the deadline closes the wait as timed out and the flow tells the requester. The
/// purchase stays pending; a decision made after the timeout is ignored by the run.
#[tokio::test(start_paused = true)]
async fn an_undecided_purchase_expires_and_ignores_a_later_decision() {
    let orders = FakeOrders::default();
    let test = harness(&orders).await;
    let run = waiting(&test, "p-21").await;
    let ms = std::time::Duration::from_millis(1);
    assert_eq!(test.advance_time_by(DECISION_TIMEOUT - ms).await, 0, "nothing runs before the deadline");
    test.advance_time_by(ms).await;
    let state: State = test.state(&run).await;
    assert_eq!(state.status, FlowStatus::Done);
    let Some(JournalEntry::Wait { outcome, received, .. }) = state.journal.get(1) else { panic!("{state:?}") };
    assert_eq!((*outcome, received.as_slice()), (Some(WaitOutcome::TimedOut), [].as_slice()));
    assert_eq!(orders.calls(), ["expire monitor flows.purchase/p-21/notify-expired"]);

    let request = pending_request(&test).await;
    test.send(&design(), ApprovalsMsg::Decide { request, approved: true, by: "ann".into() }).await;
    test.run_until_idle().await;
    assert!(test.state::<_, ApprovalsState>(&design()).await.pending.is_empty(), "decided, too late");
    assert_eq!(test.state::<_, State>(&run).await, state, "the run ignores the late decision");
    assert_eq!(orders.calls().len(), 1, "no order is placed");
}

/// A decision for a run that was deleted: the approvals actor's whole commit is rejected, so the purchase stays
/// pending and the run is not recreated.
#[tokio::test(start_paused = true)]
async fn a_decision_for_a_deleted_run_rejects_the_commit_of_the_approvals() {
    let orders = FakeOrders::default();
    let test = harness(&orders).await;
    let run = waiting(&test, "p-20").await;
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
