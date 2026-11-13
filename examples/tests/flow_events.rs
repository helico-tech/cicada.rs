//! Example 14: flows started by an event.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::SendOptions;
use cicada::store::{MessageId, OutcomeStatus};
use cicada_actors::{
    BatcherActor, BatcherMsg, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, EventActor, EventMsg,
    EventState, FlowActor, FlowMsg, FlowState, FlowStatus,
};
use cicada_examples::flow_events::{
    Digest, DigestFlow, FulfilFlow, PlacedOrder, ReceiptFlow, ShopServices, digest, fulfilment, order_placed,
    orders_batcher, receipt, set_up,
};
use cicada_examples::steps::ServiceError;
use cicada_test::TestActors;

/// Notes each call as `<service> <idempotency key>`; shipping fails while `ship_failures` is above zero.
#[derive(Clone, Default)]
struct FakeShop {
    calls: Arc<Mutex<Vec<String>>>,
    ship_failures: Arc<AtomicU32>,
}

impl FakeShop {
    fn calls(&self) -> Vec<String> {
        let mut calls = self.calls.lock().unwrap().clone();
        calls.sort();
        calls
    }

    fn note(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl ShopServices for FakeShop {
    async fn send_receipt(&self, order: &PlacedOrder, key: &str) -> Result<(), ServiceError> {
        self.note(format!("receipt {} {key}", order.order));
        Ok(())
    }

    async fn ship(&self, order: &PlacedOrder, key: &str) -> Result<String, ServiceError> {
        self.note(format!("ship {} {key}", order.order));
        let failing = self.ship_failures.load(Ordering::SeqCst);
        if failing > 0 {
            self.ship_failures.store(failing - 1, Ordering::SeqCst);
            return Err(ServiceError("the warehouse is down".into()));
        }
        Ok("t-1".into())
    }

    async fn write_digest(&self, orders: &[PlacedOrder], key: &str) -> Result<(), ServiceError> {
        let ids: Vec<&str> = orders.iter().map(|o| o.order.as_str()).collect();
        self.note(format!("digest {} {key}", ids.join(",")));
        Ok(())
    }
}

fn order(id: &str) -> PlacedOrder {
    PlacedOrder { order: id.into(), email: "ann@example.test".into(), cents: 1250 }
}

/// A node with the event, the batcher, the dead letters and the three flows on `shop`, set up.
async fn node(shop: &FakeShop) -> TestActors {
    let shop = shop.clone();
    let test = TestActors::builder()
        .with(move |node| {
            node.host(EventActor)
                .host(BatcherActor)
                .host(DeadLettersActor::default())
                .host(FlowActor::new(ReceiptFlow { services: shop.clone() }))
                .host(FlowActor::new(FulfilFlow { services: shop.clone() }))
                .host(FlowActor::new(DigestFlow { services: shop }))
        })
        .build()
        .await
        .unwrap();
    set_up(test.system()).await.unwrap();
    test.run_until_idle().await;
    test
}

/// Publishes `order` as the publication `id`.
async fn place(test: &TestActors, id: &str, order: PlacedOrder) -> cicada::store::Message {
    let options = SendOptions::new().id(MessageId::new(id).unwrap());
    test.send_with(&order_placed(), EventMsg::publish(order).unwrap(), options).await
}

async fn letters(test: &TestActors) -> DeadLettersState {
    test.state(&DeadLetters::default()).await
}

#[tokio::test(start_paused = true)]
async fn a_placed_order_starts_a_receipt_and_a_fulfilment_keyed_by_the_publication() {
    let shop = FakeShop::default();
    let test = node(&shop).await;
    place(&test, "p-1", order("o-1")).await;
    test.run_until_idle().await;
    let receipt: FlowState<PlacedOrder> = test.state(&receipt::<FakeShop>("p-1")).await;
    let fulfilment: FlowState<PlacedOrder> = test.state(&fulfilment::<FakeShop>("p-1")).await;
    assert_eq!((receipt.input, receipt.status), (order("o-1"), FlowStatus::Done));
    assert_eq!(fulfilment.status, FlowStatus::Done);
    assert_eq!(shop.calls(), ["receipt o-1 orders.receipt/p-1/send-receipt", "ship o-1 orders.fulfil/p-1/ship"]);
}

#[tokio::test(start_paused = true)]
async fn set_up_again_keeps_one_subscription_of_each_flow() {
    let test = node(&FakeShop::default()).await;
    set_up(test.system()).await.unwrap();
    test.run_until_idle().await;
    let held: EventState = test.state(&order_placed()).await;
    assert_eq!(held.flows, ["orders.receipt", "orders.fulfil"]);
}

#[tokio::test(start_paused = true)]
async fn a_batch_of_orders_starts_one_digest_with_all_of_them() {
    let shop = FakeShop::default();
    let test = node(&shop).await;
    for id in ["o-1", "o-2"] {
        test.send(&orders_batcher(), BatcherMsg::add(order(id)).unwrap()).await;
    }
    test.run_until_idle().await;
    assert!(shop.calls().is_empty(), "the batch waits");
    test.advance_time_by(Duration::from_secs(3600)).await;
    let runs = test.list("orders.digest/").await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    let key = runs[0].name();
    let state: FlowState<Digest> = test.state(&digest::<FakeShop>(&key)).await;
    assert_eq!((state.input, state.status), (Digest { items: vec![order("o-1"), order("o-2")] }, FlowStatus::Done));
    assert_eq!(shop.calls(), [format!("digest o-1,o-2 orders.digest/{key}/write-digest")]);
}

#[tokio::test(start_paused = true)]
async fn a_fulfilment_that_runs_out_of_attempts_is_kept_by_dead_letters_and_recovered_by_continue() {
    let shop = FakeShop::default();
    shop.ship_failures.store(3, Ordering::SeqCst);
    let test = node(&shop).await;
    let publication = place(&test, "p-1", order("o-1")).await;
    test.advance_time_by(Duration::from_secs(60)).await;
    assert_eq!(test.outcome(&publication).await.unwrap().status, OutcomeStatus::Done, "the publication stands");
    let held = letters(&test).await;
    assert_eq!(held.letters.len(), 1, "{held:?}");
    assert!(held.letters.values().all(|l| l.actor.as_str() == "orders.fulfil/p-1"), "{held:?}");
    test.send(&fulfilment::<FakeShop>("p-1"), FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, FlowState<PlacedOrder>>(&fulfilment::<FakeShop>("p-1")).await.status, FlowStatus::Done);
    let ships = shop.calls().into_iter().filter(|c| c.starts_with("ship")).count();
    assert_eq!(ships, 4, "three attempts and the recovery, with one key");
}

#[tokio::test(start_paused = true)]
async fn a_publication_refused_for_a_stale_subscription_is_redriven_under_a_new_id_once_it_is_removed() {
    let shop = FakeShop::default();
    let test = node(&shop).await;
    test.send(&order_placed(), EventMsg::Subscribe { flow: "orders.gone".into() }).await;
    let failed = place(&test, "p-1", order("o-1")).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&failed).await.unwrap().status, OutcomeStatus::Failed);
    assert!(test.list("orders.").await.is_empty(), "no run started");
    assert!(letters(&test).await.letters.contains_key(&failed.id), "the dead letters keep the publication");
    test.send(&order_placed(), EventMsg::Unsubscribe { flow: "orders.gone".into() }).await;
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    test.run_until_idle().await;
    let runs: Vec<String> = test.list("orders.").await.iter().map(|a| a.as_str().to_owned()).collect();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(runs.iter().all(|r| !r.ends_with("/p-1")), "the redrive's new id: {runs:?}");
}
