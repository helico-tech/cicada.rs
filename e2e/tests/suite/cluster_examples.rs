//! The worked examples across nodes (request/reply, saga with a retried charge),
//! per-message retry policies and `/failures`, over HTTP only.

use std::collections::{BTreeMap, HashSet};

use chrono::{TimeDelta, Utc};
use cicada::SendOptions;
use cicada::api::{MessageState, SendResponse};
use cicada::time::format_instant;
use cicada_e2e::cluster::{cluster, eventually, eventually_every, run_id, uri};
use cicada_e2e::{Class, step};
use cicada_examples::reply::{Cart, CartLine, CartMsg, CartState};
use cicada_examples::saga::{Inventory, InventoryMsg, InventoryState, Order, OrderMsg, OrderState, OrderStatus};
use cicada_sample_service::e2e::{Flaky, FlakyMsg, FlakyState, flaky_retry};

use crate::{millis, secs};

pub fn class() -> Class {
    Class::new(
        "ClusterExamplesTest",
        vec![
            step!(request_and_reply_between_actors),
            step!(saga_completes_through_a_retried_payment_and_compensates_a_declined_one),
            step!(a_message_sent_with_a_retry_policy_over_http_is_retried_in_place_until_an_attempt_succeeds),
            step!(a_message_sent_without_a_retry_policy_fails_at_its_first_error),
            step!(failures_list_a_failed_message_with_its_envelope_and_error),
        ],
    )
}

fn add_item(sku: &str, quantity: i32) -> CartMsg {
    CartMsg::AddItem { sku: sku.to_owned(), quantity }
}

async fn request_and_reply_between_actors() {
    let c = cluster();
    let cart = Cart { id: format!("cart-{}", run_id()) };
    c.node1.client.create(&cart, &CartState::default()).await.unwrap();
    c.send_only.client.send(&cart, add_item("apple", 3)).await.unwrap();
    c.node2.client.send(&cart, add_item("pear", 1)).await.unwrap();
    c.node3.client.send(&cart, add_item("durian", 1)).await.unwrap();

    let state = eventually("cart priced", secs(30), async || {
        let state: CartState = c.node3.try_state(&cart).await?;
        Ok((state.awaiting.is_empty() && state.lines.len() == 2).then_some(state))
    })
    .await;
    let mut lines = state.lines.clone();
    lines.sort_by(|a, b| a.sku.cmp(&b.sku));
    let line = |sku: &str, quantity, total_cents| CartLine { sku: sku.to_owned(), quantity, total_cents };
    assert_eq!(lines, [line("apple", 3, 150), line("pear", 1, 80)]);
    assert_eq!(state.rejected, BTreeMap::from([("durian".to_owned(), "unknown sku".to_owned())]));
    assert_eq!(state.total_cents(), 230);
}

async fn saga_completes_through_a_retried_payment_and_compensates_a_declined_one() {
    let c = cluster();
    let sku = format!("sku-{}", run_id());
    let inventory = Inventory { sku: sku.clone() };
    c.node1.client.create(&inventory, &InventoryState::default()).await.unwrap();
    c.node2.client.send(&inventory, InventoryMsg::Restock { quantity: 10 }).await.unwrap();

    // Amounts ending in 13 cents: the gateway is unavailable on the first call; the retry policy the saga attaches to
    // its Charge gets it through.
    let ok = Order { id: format!("ok-{}", run_id()) };
    let declined = Order { id: format!("declined-{}", run_id()) };
    c.send_only.client.create(&ok, &OrderState::default()).await.unwrap();
    c.send_only.client.create(&declined, &OrderState::default()).await.unwrap();
    let place = |quantity, amount_cents| OrderMsg::PlaceOrder { sku: sku.clone(), quantity, amount_cents };
    c.send_only.client.send(&ok, place(2, 1013)).await.unwrap();
    c.node3.client.send(&declined, place(3, 20_000)).await.unwrap();

    let completed = eventually("order completed", secs(60), async || {
        let state: OrderState = c.node1.try_state(&ok).await?;
        Ok((state.status == OrderStatus::Completed).then_some(state))
    })
    .await;
    assert!(completed.transaction_id.is_some());
    let cancelled = eventually("order cancelled", secs(60), async || {
        let state: OrderState = c.node2.try_state(&declined).await?;
        Ok((state.status == OrderStatus::Cancelled).then_some(state))
    })
    .await;
    let failure = cancelled.failure.unwrap_or_default();
    assert!(failure.contains("declined"), "{failure}");

    let settled = eventually("inventory settled", secs(30), async || {
        let state: InventoryState = c.node3.try_state(&inventory).await?;
        Ok((state.reservations.keys().collect::<Vec<_>>() == [&ok.id]).then_some(state))
    })
    .await;
    assert_eq!(settled.available, 8, "2 reserved by the completed order, the declined order's 3 released");
    let failed: Vec<_> = c
        .node1
        .failures("actor://orders/")
        .await
        .into_iter()
        .filter(|f| f.actor.as_str().ends_with(run_id()))
        .collect();
    assert!(failed.is_empty(), "{failed:?}");
}

async fn a_message_sent_with_a_retry_policy_over_http_is_retried_in_place_until_an_attempt_succeeds() {
    let c = cluster();
    let flaky = Flaky { id: format!("retry-{}", run_id()) };
    c.node1.client.create(&flaky, &FlakyState::default()).await.unwrap();
    let retry = r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT1S"}}"#;
    let sent = c
        .send_only
        .post(
            &format!("/actors/e2e/flaky/retry-{}/messages", run_id()),
            &format!(r#"{{"type": "Attempt", "payload": {{"times": 2}}, "retry": {retry}}}"#),
        )
        .await;
    assert_eq!(sent.status, 202, "{sent}");
    let receipt: SendResponse = serde_json::from_str(&sent.body).unwrap();
    // Poll the single id: pending with failures 1, then 2 (the policy waits 1 s between attempts), then done.
    let mut polled = Vec::new();
    let last = eventually_every(&format!("message {} finished", receipt.id), secs(60), millis(100), async || {
        let status = c.send_only.try_status(&receipt.id).await?;
        polled.push(status.clone());
        Ok(status.status.is_finished().then_some(status))
    })
    .await;
    for status in &polled {
        assert_eq!(status.id, receipt.id);
        assert_eq!(status.failures, status.message.failures);
        assert_eq!(status.message.retry, Some(flaky_retry()), "the policy is stored with the message");
        if status.failures > 0 {
            let error = status.last_error.clone().unwrap_or_default();
            assert!(error.contains("failing on purpose"), "{error}");
        }
    }
    let seen: HashSet<(MessageState, u32)> = polled.iter().map(|s| (s.status, s.failures)).collect();
    assert!(seen.contains(&(MessageState::Pending, 1)), "observed {seen:?}");
    assert!(seen.contains(&(MessageState::Pending, 2)), "observed {seen:?}");
    assert_eq!(last.status, MessageState::Done);
    assert_eq!(last.failures, 2);
    assert_eq!(last.message.attempt, 3);
    let state = eventually("flaky succeeded", secs(30), async || {
        let state: FlakyState = c.node2.try_state(&flaky).await?;
        Ok((state.succeeded == 1).then_some(state))
    })
    .await;
    assert_eq!(state.last_attempt, 3);
    assert!(c.node3.failures(&uri(&flaky)).await.is_empty(), "a retried message is no failure");
}

async fn a_message_sent_without_a_retry_policy_fails_at_its_first_error() {
    let c = cluster();
    let flaky = Flaky { id: format!("once-{}", run_id()) };
    c.node1.client.create(&flaky, &FlakyState::default()).await.unwrap();
    let sent = c.node3.client.send(&flaky, FlakyMsg::Attempt { times: 1 }).await.unwrap();
    let last = c.send_only.await_finished_within(&sent.id, secs(30)).await;
    assert_eq!(last.status, MessageState::Failed);
    assert_eq!(last.failures, 0);
    assert_eq!(last.message.attempt, 1);
    assert_eq!(last.message.retry, None);
    let listed = eventually("failure listed", secs(30), async || {
        let failures = c.node2.try_failures(&uri(&flaky)).await?;
        Ok((failures.len() == 1).then(|| failures[0].clone()))
    })
    .await;
    assert_eq!(listed.id, sent.id);
    assert_eq!(c.node1.state::<FlakyState, _>(&flaky).await.succeeded, 0);
}

async fn failures_list_a_failed_message_with_its_envelope_and_error() {
    let c = cluster();
    let flaky = Flaky { id: format!("fail-{}", run_id()) };
    c.node1.client.create(&flaky, &FlakyState::default()).await.unwrap();
    let before = Utc::now() - TimeDelta::seconds(5);
    let correlation = format!("corr-{}", run_id());
    let options = SendOptions::new().correlation_id(&correlation).retry(flaky_retry());
    let sent = c.node2.client.send_with(&flaky, FlakyMsg::Attempt { times: 5 }, options).await.unwrap();

    let failure = eventually("failure listed", secs(30), async || {
        let failures = c.send_only.try_failures(&uri(&flaky)).await?;
        Ok((failures.len() == 1).then(|| failures[0].clone()))
    })
    .await;
    assert_eq!(failure.status, MessageState::Failed);
    assert_eq!(failure.id, sent.id, "the message itself failed, once, after its retries");
    assert_eq!(failure.message.failures, 2);
    assert_eq!(failure.message.retry, Some(flaky_retry()));
    assert_eq!(failure.actor.to_string(), uri(&flaky));
    let error = failure.error.clone().unwrap_or_default();
    assert!(error.contains("failing on purpose"), "{error}");
    assert_eq!(failure.message.message_type, "Attempt");
    assert_eq!(failure.message.payload["times"], 5);
    assert_eq!(failure.message.correlation_id, Some(correlation), "the retried message keeps its correlation id");
    assert_eq!(failure.message.to.to_string(), uri(&flaky));

    // Raw form and the `since` filter.
    let raw = c.node3.get(&format!("/failures?prefix={}&since={}", uri(&flaky), format_instant(before))).await;
    assert_eq!(raw.status, 200, "{raw}");
    assert!(raw.body.contains("failing on purpose"), "{raw}");
    let later = Utc::now() + TimeDelta::hours(1);
    let none = c.node3.client.failures(&uri(&flaky), Some(later), None, None).await.unwrap();
    assert!(none.items.is_empty(), "{:?}", none.items);
    assert_eq!(c.node1.state::<FlakyState, _>(&flaky).await.succeeded, 0);
}
