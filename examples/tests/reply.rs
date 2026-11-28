//! Example 4: request and reply between the cart and the pricing service.

use std::collections::BTreeMap;

use cicada::message::{decode, encode};
use cicada::store::OutcomeStatus;
use cicada::{Actor, NoState, SendOptions};
use cicada_examples::reply::{
    Cart, CartActor, CartLine, CartMsg, CartState, PriceList, Pricing, PricingActor, PricingMsg, PricingReply,
};
use cicada_test::TestActors;

// The cart depends only on the pricing port, so either fake can stand in for the real pricing actor.
mod fake_pricing {
    use cicada::{Actor, HandlerResult, NoState, SendOptions, Turn};
    use cicada_examples::reply::{Pricing, PricingMsg, PricingReply};

    pub(super) struct FixedPricing;

    impl Actor for FixedPricing {
        type Ref = Pricing;
        type State = NoState;

        async fn handle(&self, turn: &mut Turn<Self>, message: PricingMsg) -> HandlerResult {
            let PricingMsg::QuoteRequest { sku, quantity } = message;
            let Some(requester) = turn.message().from.clone() else { return Ok(()) };
            let unit_price_cents = 125;
            let reply = PricingReply::Quote {
                sku,
                quantity,
                unit_price_cents,
                total_cents: unit_price_cents * i64::from(quantity),
            };
            let options = SendOptions { correlation_id: turn.message().correlation_id.clone(), ..SendOptions::new() };
            turn.send_to(&requester, &reply, options).await?;
            Ok(())
        }
    }

    pub(super) struct RejectingPricing;

    impl Actor for RejectingPricing {
        type Ref = Pricing;
        type State = NoState;

        async fn handle(&self, turn: &mut Turn<Self>, message: PricingMsg) -> HandlerResult {
            let PricingMsg::QuoteRequest { sku, .. } = message;
            let Some(requester) = turn.message().from.clone() else { return Ok(()) };
            let reply = PricingReply::QuoteRejected { sku, reason: "not for sale".into() };
            let options = SendOptions { correlation_id: turn.message().correlation_id.clone(), ..SendOptions::new() };
            turn.send_to(&requester, &reply, options).await?;
            Ok(())
        }
    }
}

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

async fn harness(pricing: impl Actor<Ref = Pricing, State = NoState>) -> TestActors {
    TestActors::builder().with(move |node| node.host(pricing).host(CartActor)).build().await.unwrap()
}

fn add_item(sku: &str, quantity: i32) -> CartMsg {
    CartMsg::AddItem { sku: sku.into(), quantity }
}

fn apple_quote() -> CartMsg {
    CartMsg::Quote { sku: "apple".into(), quantity: 1, unit_price_cents: 50, total_cents: 50 }
}

#[tokio::test(start_paused = true)]
async fn the_cart_prices_its_lines_through_request_and_reply() {
    let test = harness(PricingActor { prices: Prices }).await;
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
async fn the_cart_prices_lines_with_a_fixed_pricing_fake() {
    let test = harness(fake_pricing::FixedPricing).await;
    test.create(&Pricing, &NoState {}).await;
    test.create(&cart(), &CartState::default()).await;
    test.send(&cart(), add_item("apple", 3)).await;
    test.send(&cart(), add_item("pear", 2)).await;
    test.run_until_idle().await;

    let state: CartState = test.state(&cart()).await;
    assert_eq!(
        state.lines,
        vec![
            CartLine { sku: "apple".into(), quantity: 3, total_cents: 375 },
            CartLine { sku: "pear".into(), quantity: 2, total_cents: 250 },
        ]
    );
    assert!(state.awaiting.is_empty());
    assert!(state.rejected.is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_cart_records_a_rejection_from_a_pricing_fake() {
    let test = harness(fake_pricing::RejectingPricing).await;
    test.create(&Pricing, &NoState {}).await;
    test.create(&cart(), &CartState::default()).await;
    test.send(&cart(), add_item("apple", 3)).await;
    test.run_until_idle().await;

    let state: CartState = test.state(&cart()).await;
    assert_eq!(state.rejected, BTreeMap::from([("apple".into(), "not for sale".into())]));
    assert!(state.awaiting.is_empty());
    assert!(state.lines.is_empty());
}

#[test]
fn pricing_replies_decode_as_cart_messages() {
    // A port's reply family and the caller's message family are matched only at runtime.
    let cases = [
        (
            PricingReply::Quote { sku: "apple".into(), quantity: 3, unit_price_cents: 125, total_cents: 375 },
            CartMsg::Quote { sku: "apple".into(), quantity: 3, unit_price_cents: 125, total_cents: 375 },
        ),
        (
            PricingReply::QuoteRejected { sku: "pear".into(), reason: "not for sale".into() },
            CartMsg::QuoteRejected { sku: "pear".into(), reason: "not for sale".into() },
        ),
    ];
    for (reply, expected) in cases {
        let encoded = encode(&reply).unwrap();
        let decoded = decode::<CartMsg>(&encoded.message_type, &encoded.payload).unwrap();
        assert_eq!(decoded, expected);
    }
}

#[tokio::test(start_paused = true)]
async fn an_unsolicited_or_duplicate_reply_is_ignored() {
    let test = harness(PricingActor { prices: Prices }).await;
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
    let test = harness(PricingActor { prices: Prices }).await;
    test.create(&Pricing, &NoState {}).await;
    let request = test.send(&Pricing, PricingMsg::QuoteRequest { sku: "apple".into(), quantity: 1 }).await;
    assert_eq!(test.run_until_idle().await, 1);
    assert_eq!(test.outcome(&request).await.unwrap().status, OutcomeStatus::Done);
}
