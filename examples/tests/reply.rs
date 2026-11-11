//! Example 4: request and reply between the cart and the pricing service.

use std::collections::BTreeMap;

use cicada::store::OutcomeStatus;
use cicada::{NoState, SendOptions};
use cicada_examples::reply::{
    Cart, CartActor, CartLine, CartMsg, CartState, PriceList, Pricing, PricingActor, PricingMsg,
};
use cicada_test::TestActors;

struct Prices;

impl PriceList for Prices {
    fn unit_price_cents(&self, sku: &str) -> Option<i64> {
        match sku {
            "apple" => Some(50),
            "pear" => Some(80),
            _ => None,
        }
    }
}

fn cart() -> Cart {
    Cart { id: "alice".into() }
}

async fn harness() -> TestActors {
    TestActors::builder().with(|node| node.host(PricingActor { prices: Prices }).host(CartActor)).build().await.unwrap()
}

fn add_item(sku: &str, quantity: i32) -> CartMsg {
    CartMsg::AddItem { sku: sku.into(), quantity }
}

fn apple_quote() -> CartMsg {
    CartMsg::Quote { sku: "apple".into(), quantity: 1, unit_price_cents: 50, total_cents: 50 }
}

#[tokio::test(start_paused = true)]
async fn the_cart_prices_its_lines_through_request_and_reply() {
    let test = harness().await;
    test.create(&Pricing, &NoState {}).await;
    test.create(&cart(), &CartState::default()).await;
    let apple = test.send(&cart(), add_item("apple", 3)).await;
    test.send(&cart(), add_item("pear", 2)).await;
    test.send(&cart(), add_item("durian", 1)).await;
    test.run_until_idle().await;

    let state: CartState = test.state(&cart()).await;
    let line = |sku: &str, quantity, total_cents| CartLine { sku: sku.into(), quantity, total_cents };
    assert_eq!(state.lines, vec![line("apple", 3, 150), line("pear", 2, 160)]);
    assert_eq!(state.total_cents(), 310);
    assert_eq!(state.rejected, BTreeMap::from([("durian".to_owned(), "unknown sku".to_owned())]));
    assert!(state.awaiting.is_empty());

    assert_eq!(test.outcome(&apple).await.unwrap().status, OutcomeStatus::Done);
    assert!(test.failures().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_unsolicited_or_duplicate_reply_is_ignored() {
    let test = harness().await;
    test.create(&Pricing, &NoState {}).await;
    test.create(&cart(), &CartState::default()).await;
    test.send(&cart(), apple_quote()).await;
    // With a correlation id that matches no request.
    test.send_with(&cart(), apple_quote(), SendOptions::new().correlation_id("unknown")).await;
    test.run_until_idle().await;
    assert!(test.state::<_, CartState>(&cart()).await.lines.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_request_from_outside_an_actor_gets_no_reply() {
    let test = harness().await;
    test.create(&Pricing, &NoState {}).await;
    let request = test.send(&Pricing, PricingMsg::QuoteRequest { sku: "apple".into(), quantity: 1 }).await;
    assert_eq!(test.run_until_idle().await, 1);
    assert_eq!(test.outcome(&request).await.unwrap().status, OutcomeStatus::Done);
}
