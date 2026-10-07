//! Example 2: the checkout timer.

use std::time::Duration;

use cicada_examples::timer::{Checkout, CheckoutActor, CheckoutMsg, CheckoutState, CheckoutStatus};
use cicada_test::TestActors;

fn checkout() -> Checkout {
    Checkout { id: "c1".into() }
}

async fn harness() -> TestActors {
    let test = TestActors::builder().with(|node| node.host(CheckoutActor)).build().await.unwrap();
    test.create(&checkout(), &CheckoutState::default()).await;
    test
}

async fn state(test: &TestActors) -> CheckoutState {
    test.state(&checkout()).await
}

fn begin() -> CheckoutMsg {
    CheckoutMsg::Begin { timeout_seconds: 60 }
}

#[tokio::test(start_paused = true)]
async fn an_unpaid_checkout_expires_when_its_timer_fires() {
    let test = harness().await;
    test.send(&checkout(), begin()).await;
    test.run_until_idle().await;
    assert_eq!(state(&test).await.status, CheckoutStatus::AwaitingPayment);

    test.advance_time_by(Duration::from_secs(59)).await;
    assert_eq!(state(&test).await.status, CheckoutStatus::AwaitingPayment);
    test.advance_time_by(Duration::from_secs(1)).await;
    assert_eq!(state(&test).await.status, CheckoutStatus::Expired);
}

#[tokio::test(start_paused = true)]
async fn extending_re_arms_the_timer_and_the_stale_timeout_is_ignored() {
    let test = harness().await;
    test.send(&checkout(), begin()).await;
    test.run_until_idle().await;
    test.send(&checkout(), CheckoutMsg::Extend { seconds: 60 }).await;
    test.run_until_idle().await;
    assert_eq!(test.pending(&checkout()).await.len(), 2); // the old timer is still there; it is stale, not cancelled

    test.advance_time_by(Duration::from_secs(61)).await;
    let s = state(&test).await;
    assert_eq!(s.status, CheckoutStatus::AwaitingPayment);
    assert_eq!(s.stale_timeouts, 1);
    test.advance_time_by(Duration::from_secs(60)).await;
    assert_eq!(state(&test).await.status, CheckoutStatus::Expired);
}

#[tokio::test(start_paused = true)]
async fn paying_disarms_the_timer() {
    let test = harness().await;
    test.send(&checkout(), begin()).await;
    test.send(&checkout(), begin()).await; // idempotent: no second timer
    test.run_until_idle().await;
    assert_eq!(test.pending(&checkout()).await.len(), 1);
    test.send(&checkout(), CheckoutMsg::Pay { amount_cents: 4_200 }).await;
    test.advance_time_by(Duration::from_secs(600)).await;

    let s = state(&test).await;
    assert_eq!(s.status, CheckoutStatus::Paid);
    assert_eq!(s.paid_cents, Some(4_200));
    assert_eq!(s.stale_timeouts, 1);
    assert!(test.pending(&checkout()).await.is_empty());
}
