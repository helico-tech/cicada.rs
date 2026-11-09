//! Example 2: the checkout timer.

use std::time::Duration;

use cicada::time;
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
    let begun_at = test.now();
    test.send(&checkout(), begin()).await;
    test.run_until_idle().await;
    let s = state(&test).await;
    assert_eq!(s.status, CheckoutStatus::AwaitingPayment);
    assert_eq!(s.deadline, Some(time::add(begun_at, Duration::from_secs(60)).unwrap()));
    // The state names the timeout that waits in the mailbox.
    let [timeout] = test.pending(&checkout()).await.try_into().expect("one pending timeout");
    assert_eq!(timeout.message_type, "PaymentTimeout");
    assert_eq!(s.timeout, Some(timeout.id));
    assert_eq!(Some(timeout.deliver_at), s.deadline);

    test.advance_time_by(Duration::from_secs(59)).await;
    assert_eq!(state(&test).await.status, CheckoutStatus::AwaitingPayment);
    test.advance_time_by(Duration::from_secs(1)).await;
    let s = state(&test).await;
    assert_eq!(s.status, CheckoutStatus::Expired);
    assert_eq!((s.deadline, s.timeout), (None, None));
    assert!(test.pending(&checkout()).await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn extending_moves_the_deadline_and_leaves_one_pending_timeout() {
    let test = harness().await;
    test.send(&checkout(), begin()).await;
    test.run_until_idle().await;
    let before = state(&test).await;
    test.send(&checkout(), CheckoutMsg::Extend { seconds: 60 }).await;
    test.run_until_idle().await;

    let after = state(&test).await;
    assert_eq!(after.status, CheckoutStatus::AwaitingPayment);
    assert_eq!(after.deadline, Some(time::add(before.deadline.unwrap(), Duration::from_secs(60)).unwrap()));
    assert_ne!(after.timeout, before.timeout);
    // The old timeout was cancelled, not left behind: only the new one is pending.
    let [timeout] = test.pending(&checkout()).await.try_into().expect("one pending timeout");
    assert_eq!(after.timeout, Some(timeout.id));
    assert_eq!(Some(timeout.deliver_at), after.deadline);

    test.advance_time_by(Duration::from_secs(61)).await; // past the first deadline
    assert_eq!(state(&test).await, after);
    test.advance_time_by(Duration::from_secs(60)).await;
    assert_eq!(state(&test).await.status, CheckoutStatus::Expired);
}

#[tokio::test(start_paused = true)]
async fn paying_cancels_the_pending_timeout() {
    let test = harness().await;
    test.send(&checkout(), begin()).await;
    test.run_until_idle().await;
    assert_eq!(test.pending(&checkout()).await.len(), 1);
    test.send(&checkout(), CheckoutMsg::Pay { amount_cents: 4_200 }).await;
    test.run_until_idle().await;
    assert!(test.pending(&checkout()).await.is_empty());

    test.advance_time_by(Duration::from_secs(600)).await;
    let s = state(&test).await;
    assert_eq!(s.status, CheckoutStatus::Paid);
    assert_eq!(s.paid_cents, Some(4_200));
    assert_eq!((s.deadline, s.timeout), (None, None));
}

#[tokio::test(start_paused = true)]
async fn a_second_begin_is_idempotent() {
    let test = harness().await;
    test.send(&checkout(), begin()).await;
    test.run_until_idle().await;
    let first = state(&test).await;

    test.advance_time_by(Duration::from_secs(10)).await;
    test.send(&checkout(), CheckoutMsg::Begin { timeout_seconds: 600 }).await;
    test.run_until_idle().await;
    assert_eq!(state(&test).await, first); // the same deadline and the same timeout: no second timer
    assert_eq!(test.pending(&checkout()).await.len(), 1);

    test.advance_time_by(Duration::from_secs(50)).await;
    assert_eq!(state(&test).await.status, CheckoutStatus::Expired);
}
