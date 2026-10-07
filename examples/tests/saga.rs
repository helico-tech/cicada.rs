//! Example 5: the order saga with compensation, and the charge's retry policy.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::SendOptions;
use cicada::store::{MessageId, OutcomeRecord, OutcomeStatus};
use cicada_examples::saga::{
    ChargeResult, GatewayUnavailable, Inventory, InventoryActor, InventoryState, Order, OrderActor, OrderMsg,
    OrderState, OrderStatus, PAYMENT_ATTEMPTS, PaymentGateway, Payments, PaymentsActor, PaymentsMsg, charge_retry,
};
use cicada_test::TestActors;

/// A gateway that declines above a limit and is unavailable for the first `outages` calls.
#[derive(Clone)]
struct FakeGateway {
    inner: Arc<Mutex<GatewayState>>,
    limit_cents: i64,
}

#[derive(Default)]
struct GatewayState {
    outages: u32,
    keys: Vec<String>,
}

impl FakeGateway {
    fn new(outages: u32) -> Self {
        Self { inner: Arc::new(Mutex::new(GatewayState { outages, keys: Vec::new() })), limit_cents: 10_000 }
    }

    fn keys(&self) -> Vec<String> {
        self.inner.lock().unwrap().keys.clone()
    }
}

impl PaymentGateway for FakeGateway {
    async fn charge(
        &self,
        order_id: &str,
        amount_cents: i64,
        idempotency_key: &str,
    ) -> Result<ChargeResult, GatewayUnavailable> {
        let mut inner = self.inner.lock().unwrap();
        inner.keys.push(idempotency_key.to_owned());
        if inner.outages > 0 {
            inner.outages -= 1;
            return Err(GatewayUnavailable("gateway timeout".into()));
        }
        Ok(if amount_cents > self.limit_cents {
            ChargeResult::Declined { reason: "limit exceeded".into() }
        } else {
            ChargeResult::Approved { transaction_id: format!("tx-{order_id}") }
        })
    }
}

fn order() -> Order {
    Order { id: "o-1".into() }
}

fn apples() -> Inventory {
    Inventory { sku: "apple".into() }
}

fn place(quantity: i32, amount_cents: i64) -> OrderMsg {
    OrderMsg::PlaceOrder { sku: "apple".into(), quantity, amount_cents }
}

async fn harness(gateway: &FakeGateway) -> TestActors {
    let gateway = gateway.clone();
    TestActors::builder()
        .with(move |node| node.host(OrderActor).host(InventoryActor).host(PaymentsActor { gateway }))
        .build()
        .await
        .unwrap()
}

async fn set_up(test: &TestActors, stock: i32) {
    test.create_stateless(&Payments).await;
    test.create(&apples(), &InventoryState { available: stock, ..Default::default() }).await;
    test.create(&order(), &OrderState::default()).await;
}

async fn order_state(test: &TestActors) -> OrderState {
    test.state(&order()).await
}

async fn inventory_state(test: &TestActors) -> InventoryState {
    test.state(&apples()).await
}

async fn outcome_of(test: &TestActors, key: &str) -> OutcomeRecord {
    test.system().outcome(&MessageId::new(key).unwrap()).await.unwrap().expect("an outcome")
}

#[tokio::test(start_paused = true)]
async fn the_happy_path_reserves_charges_and_completes() {
    let gateway = FakeGateway::new(0);
    let test = harness(&gateway).await;
    set_up(&test, 10).await;
    test.send(&order(), place(3, 1_500)).await;
    test.run_until_idle().await;

    let expected = OrderState {
        status: OrderStatus::Completed,
        sku: Some("apple".into()),
        quantity: 3,
        amount_cents: 1_500,
        transaction_id: Some("tx-o-1".into()),
        failure: None,
    };
    assert_eq!(order_state(&test).await, expected);
    let reservations = BTreeMap::from([("o-1".to_owned(), 3)]);
    assert_eq!(inventory_state(&test).await, InventoryState { available: 7, reservations });
}

#[tokio::test(start_paused = true)]
async fn a_declined_payment_compensates_the_reservation() {
    let gateway = FakeGateway::new(0);
    let test = harness(&gateway).await;
    set_up(&test, 10).await;
    test.send(&order(), place(3, 50_000)).await;
    test.run_until_idle().await;

    let state = order_state(&test).await;
    assert_eq!(state.status, OrderStatus::Cancelled);
    assert_eq!(state.failure.as_deref(), Some("payment declined: limit exceeded"));
    assert_eq!(inventory_state(&test).await, InventoryState { available: 10, ..Default::default() });
}

#[tokio::test(start_paused = true)]
async fn out_of_stock_cancels_without_compensation() {
    let gateway = FakeGateway::new(0);
    let test = harness(&gateway).await;
    set_up(&test, 2).await;
    test.send(&order(), place(3, 1_500)).await;
    test.run_until_idle().await;

    assert_eq!(order_state(&test).await.status, OrderStatus::Cancelled);
    assert_eq!(inventory_state(&test).await, InventoryState { available: 2, ..Default::default() });
    assert!(gateway.keys().is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_charges_retry_policy_rides_out_a_transient_gateway_outage_with_one_idempotency_key() {
    let gateway = FakeGateway::new(2);
    let test = harness(&gateway).await;
    set_up(&test, 10).await;
    test.send(&order(), place(1, 500)).await;
    test.run_until_idle().await;
    assert_eq!(order_state(&test).await.status, OrderStatus::Charging);

    test.advance_time_by(Duration::from_secs(1)).await; // first retry after 1 s, fails again
    test.advance_time_by(Duration::from_secs(2)).await; // second retry after 2 s more, succeeds
    assert_eq!(order_state(&test).await.status, OrderStatus::Completed);
    let keys = gateway.keys();
    assert_eq!(keys.len(), 3);
    assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 1);
    assert!(test.failures().await.is_empty()); // rescheduled attempts are no failures; nothing for supervision to see
    let charge = outcome_of(&test, &keys[0]).await;
    assert_eq!(charge.status, OutcomeStatus::Done, "the key is the Charge message's own id");
    assert_eq!(charge.envelope.failures, 2);
    assert_eq!(charge.envelope.retry, Some(charge_retry()), "the policy the saga attached travels with the message");
}

#[tokio::test(start_paused = true)]
async fn a_charge_sent_without_a_retry_policy_is_not_retried_its_first_outage_is_answered_with_a_decline() {
    let gateway = FakeGateway::new(1);
    let test = harness(&gateway).await;
    test.create_stateless(&Payments).await;
    test.create(&order(), &OrderState::default()).await;
    let charge = test.send(&Payments, PaymentsMsg::Charge { reply_to: order(), amount_cents: 500 }).await;
    test.advance_time_by(Duration::from_secs(60)).await;

    let outcome = test.outcome(&charge).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done, "the only attempt is the last: Payments answers");
    assert_eq!(outcome.envelope.failures, 0);
    assert_eq!(gateway.keys().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_invalid_charge_is_vetoed_with_fail_and_not_retried_despite_the_policy() {
    let gateway = FakeGateway::new(0);
    let test = harness(&gateway).await;
    test.create_stateless(&Payments).await;
    test.create(&order(), &OrderState::default()).await;
    let charge = PaymentsMsg::Charge { reply_to: order(), amount_cents: 0 };
    let charge = test.send_with(&Payments, charge, SendOptions::new().retry(charge_retry())).await;
    test.run_until_idle().await;

    let outcome = test.outcome(&charge).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert_eq!(outcome.error.as_deref(), Some("invalid amount 0"));
    assert_eq!(outcome.envelope.failures, 0);
    assert!(gateway.keys().is_empty());
}

#[tokio::test(start_paused = true)]
async fn when_the_gateway_stays_down_the_last_attempt_declines_and_the_saga_compensates() {
    let gateway = FakeGateway::new(u32::MAX);
    let test = harness(&gateway).await;
    set_up(&test, 10).await;
    test.send(&order(), place(1, 500)).await;
    test.run_until_idle().await;
    test.advance_time_by(Duration::from_secs(60)).await;

    let keys = gateway.keys();
    assert_eq!(keys.len(), PAYMENT_ATTEMPTS as usize);
    let state = order_state(&test).await;
    assert_eq!(state.status, OrderStatus::Cancelled);
    assert_eq!(state.failure.as_deref(), Some("payment declined: payment gateway unavailable"));
    assert_eq!(inventory_state(&test).await, InventoryState { available: 10, ..Default::default() });
    // The key is the Charge message's id: one message, run PAYMENT_ATTEMPTS times; the last run answered.
    assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 1);
    let charge = outcome_of(&test, &keys[0]).await;
    assert_eq!(charge.status, OutcomeStatus::Done);
    assert_eq!(charge.envelope.failures, PAYMENT_ATTEMPTS - 1);
}

#[tokio::test(start_paused = true)]
async fn late_and_duplicate_replies_are_ignored() {
    let gateway = FakeGateway::new(0);
    let test = harness(&gateway).await;
    set_up(&test, 10).await;
    test.send(&order(), place(3, 1_500)).await;
    test.run_until_idle().await;
    test.send(&order(), OrderMsg::Reserved).await;
    test.send(&order(), OrderMsg::ChargeDeclined { reason: "late".into() }).await;
    test.send(&order(), place(3, 1_500)).await;
    test.run_until_idle().await;

    assert_eq!(order_state(&test).await.status, OrderStatus::Completed);
    assert_eq!(inventory_state(&test).await.available, 7);
}
