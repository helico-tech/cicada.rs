//! The change feed's own parts (ADR 0034): the collector of a transaction, the PostgreSQL payload format, and the hub
//! with its streams, driven by hand as a source would drive it. The writes of each store are in the conformance suite.

use std::sync::Arc;
use std::time::Duration;

use futures::{FutureExt, StreamExt};

use crate::address::ActorAddress;
use crate::change::{
    ActorChange, ActorChanges, ChangeBatch, ChangeCollector, ChangeHub, HUB_CAPACITY, MAX_ADDRESSES, MAX_PAYLOAD_BYTES,
    SourceState,
};

fn addr(text: &str) -> ActorAddress {
    ActorAddress::parse(text).unwrap()
}

fn changed(text: &str) -> ActorChange {
    ActorChange::changed(addr(text))
}

fn batch(texts: &[&str]) -> ChangeBatch {
    ChangeBatch::Addresses(texts.iter().map(|t| addr(t)).collect())
}

/// The next item, or `None` for "nothing within a minute of virtual time".
async fn next(changes: &mut ActorChanges) -> Option<Option<ActorChange>> {
    tokio::time::timeout(Duration::from_secs(60), changes.next()).await.ok()
}

fn hub(state: SourceState) -> Arc<ChangeHub> {
    Arc::new(ChangeHub::new(state))
}

// ---- collector ----

#[test]
fn a_collector_keeps_each_address_once_in_address_order() {
    let mut changes = ChangeCollector::default();
    assert!(changes.is_empty());
    assert_eq!(changes.take(), None);
    for text in ["b/2", "a/1", "b/2", "a/1"] {
        changes.add(&addr(text));
    }
    assert!(!changes.is_empty());
    assert_eq!(changes.take(), Some(batch(&["a/1", "b/2"])));
    assert!(changes.is_empty(), "take leaves it empty");
    assert_eq!(changes.take(), None);
}

#[test]
fn the_101st_address_turns_the_batch_into_one_resync_for_good() {
    let mut changes = ChangeCollector::default();
    let addresses: Vec<ActorAddress> = (0..MAX_ADDRESSES).map(|i| addr(&format!("a/{i}"))).collect();
    changes.add_all(&addresses);
    changes.add(&addresses[0]); // a duplicate costs nothing
    match changes.take() {
        Some(ChangeBatch::Addresses(all)) => assert_eq!(all.len(), 100),
        other => panic!("{other:?}"),
    }
    changes.add_all(&addresses);
    changes.add(&addr("a/one-more"));
    assert_eq!(changes.take(), Some(ChangeBatch::Resync));
    changes.add_all(&addresses);
    changes.add(&addr("a/one-more"));
    changes.add(&addr("b/after"));
    assert_eq!(changes.take(), Some(ChangeBatch::Resync), "sticky after the bound");
}

#[test]
fn a_batch_of_exactly_7998_encoded_bytes_fits_and_one_more_byte_resyncs() {
    // `@`, three addresses of 1998 bytes and two spaces: 5997 bytes; a fourth address of n bytes adds n + 1.
    let of_len = |c: char, n: usize| addr(&format!("t/{}", c.to_string().repeat(n - 2)));
    let three = [of_len('a', 1998), of_len('b', 1998), of_len('c', 1998)];
    let collect = |last: &ActorAddress| {
        let mut changes = ChangeCollector::default();
        changes.add_all(&three);
        changes.add(&three[1]); // a duplicate takes no space
        changes.add(last);
        changes.take()
    };
    let fits = collect(&of_len('d', 2000)).expect("a batch");
    assert_eq!(fits.encode().len(), MAX_PAYLOAD_BYTES);
    assert!(matches!(&fits, ChangeBatch::Addresses(a) if a.len() == 4));
    assert_eq!(ChangeBatch::parse(&fits.encode()), Some(fits));
    assert_eq!(collect(&of_len('d', 2001)), Some(ChangeBatch::Resync), "7999 bytes");
}

#[test]
fn an_overflow_without_addresses_is_a_resync() {
    let mut changes = ChangeCollector::default();
    changes.add(&addr("a/1"));
    changes.overflow();
    changes.add(&addr("a/2"));
    assert!(!changes.is_empty());
    assert_eq!(changes.take(), Some(ChangeBatch::Resync));
}

// ---- payloads ----

#[test]
fn a_batch_encodes_as_an_at_sign_and_its_addresses_separated_by_one_space() {
    assert_eq!(ChangeBatch::Resync.encode(), "@");
    let b = batch(&["a/1", "b/jan%40example.com"]);
    assert_eq!(b.encode(), "@a/1 b/jan%40example.com");
    assert_eq!(ChangeBatch::parse(&b.encode()), Some(b));
}

#[test]
fn payloads_without_an_at_sign_are_not_the_feeds() {
    for payload in ["", "counter", "!cicada-listener-probe", " @a/1"] {
        assert_eq!(ChangeBatch::parse(payload), None, "{payload:?}");
    }
    assert_eq!(ChangeBatch::parse("@"), Some(ChangeBatch::Resync));
}

#[test]
fn a_malformed_feed_payload_is_one_resync_as_a_whole() {
    let malformed = [
        "@a/b bad",
        "@ a/b",
        "@a/b  c/d",
        "@a/b ",
        "@a/%7e",    // not canonical: a/~
        "@a/b@c",    // not an address
        "@a/b\tc/d", // a tab is no separator
        "@@",        // another @ format
        "@v2:a/b",   // another @ format
        "@A/b",      // not a type name
    ];
    for payload in malformed {
        assert_eq!(ChangeBatch::parse(payload), Some(ChangeBatch::Resync), "{payload:?}");
    }
}

#[test]
fn the_hub_publishes_feed_payloads_as_batches_and_other_ones_as_wake_ups() {
    let hub = ChangeHub::new(SourceState::Up);
    let (mut rx, _) = hub.subscribe().unwrap();
    hub.payload("@a/1");
    hub.payload("counter");
    hub.payload("");
    hub.payload("@");
    let items: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok()).map(|i| format!("{i:?}")).collect();
    assert_eq!(items.len(), 3, "{items:?}");
    assert!(items[0].starts_with("Changes"), "{items:?}");
    assert_eq!(items[1], "WakeUp(\"counter\")");
    assert_eq!(items[2], "Resync");
}

// ---- streams ----

#[test]
fn a_stream_is_send_unpin_and_static() {
    fn check<T: Send + Unpin + 'static>() {}
    check::<ActorChanges>();
}

#[tokio::test(start_paused = true)]
async fn a_live_source_starts_with_resync_then_yields_hints_under_the_prefix() {
    let hub = hub(SourceState::Up);
    let mut changes = ActorChanges::new(hub.clone(), "order/");
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    hub.publish(batch(&["order/1", "other/1", "orders/2"]));
    hub.publish(batch(&["order/3"]));
    assert_eq!(next(&mut changes).await, Some(Some(changed("order/1"))));
    assert_eq!(next(&mut changes).await, Some(Some(changed("order/3"))));
    assert_eq!(next(&mut changes).await, None, "nothing more");
}

#[tokio::test(start_paused = true)]
async fn prefixes_are_literal_like_the_listing_and_a_resync_reaches_every_one() {
    let hub = hub(SourceState::Up);
    let prefixes = ["", "order", "order/", "order/a%40", "order/a_"];
    let mut streams: Vec<ActorChanges> = prefixes.iter().map(|p| ActorChanges::new(hub.clone(), p)).collect();
    for s in &mut streams {
        assert_eq!(next(s).await, Some(Some(ActorChange::Resync)));
    }
    hub.publish(batch(&["order/a%40b", "orders/x", "order/aXb", "order/a_b"]));
    hub.publish(ChangeBatch::Resync);
    let expected: [&[&str]; 5] = [
        &["order/a%40b", "orders/x", "order/aXb", "order/a_b"],
        &["order/a%40b", "orders/x", "order/aXb", "order/a_b"],
        &["order/a%40b", "order/aXb", "order/a_b"],
        &["order/a%40b"],
        &["order/a_b"],
    ];
    for (s, want) in streams.iter_mut().zip(expected) {
        for address in want {
            assert_eq!(next(s).await, Some(Some(changed(address))));
        }
        assert_eq!(next(s).await, Some(Some(ActorChange::Resync)));
    }
}

#[tokio::test(start_paused = true)]
async fn a_source_that_is_down_yields_nothing_until_it_is_up_and_then_resync_without_the_older_hints() {
    let hub = hub(SourceState::Down);
    let mut changes = ActorChanges::new(hub.clone(), "");
    assert_eq!(next(&mut changes).await, None, "no first item while down");
    hub.publish(batch(&["a/before"]));
    hub.up();
    hub.publish(batch(&["a/meanwhile"])); // before the fresh receiver of the resync: its reread covers it
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    hub.publish(batch(&["a/after"]));
    assert_eq!(next(&mut changes).await, Some(Some(changed("a/after"))));
}

#[tokio::test(start_paused = true)]
async fn hints_queued_before_a_loss_may_come_first_and_the_reconnect_resyncs() {
    let hub = hub(SourceState::Up);
    let mut changes = ActorChanges::new(hub.clone(), "");
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    hub.publish(batch(&["a/1"]));
    hub.down();
    hub.publish(batch(&["a/lost"])); // a source cannot send while down; were it to, the resync covers it
    assert_eq!(next(&mut changes).await, Some(Some(changed("a/1"))));
    assert_eq!(next(&mut changes).await, None, "waits for the source");
    hub.up();
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    hub.publish(batch(&["a/2"]));
    assert_eq!(next(&mut changes).await, Some(Some(changed("a/2"))));
}

#[tokio::test(start_paused = true)]
async fn an_up_seen_while_live_resyncs_again_as_after_a_stale_up() {
    let hub = hub(SourceState::Up);
    let mut changes = ActorChanges::new(hub.clone(), "");
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    // The connection was lost before the listener noticed: the stale up yielded Resync; the loss and reconnect follow.
    hub.down();
    hub.up();
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    assert_eq!(next(&mut changes).await, None);
}

#[tokio::test(start_paused = true)]
async fn a_slow_reader_resyncs_against_a_fresh_receiver_and_writers_never_wait() {
    let hub = hub(SourceState::Up);
    let mut slow = ActorChanges::new(hub.clone(), "");
    let mut fast = ActorChanges::new(hub.clone(), "");
    assert_eq!(next(&mut slow).await, Some(Some(ActorChange::Resync)));
    assert_eq!(next(&mut fast).await, Some(Some(ActorChange::Resync)));
    for i in 0..HUB_CAPACITY + 10 {
        hub.publish(batch(&[&format!("a/{i}")]));
        assert_eq!(next(&mut fast).await, Some(Some(changed(&format!("a/{i}")))));
    }
    assert_eq!(next(&mut slow).await, Some(Some(ActorChange::Resync)), "lagged");
    assert_eq!(next(&mut slow).await, None, "the lagged backlog is dropped");
    hub.publish(batch(&["a/live"]));
    assert_eq!(next(&mut slow).await, Some(Some(changed("a/live"))));
    assert_eq!(next(&mut fast).await, Some(Some(changed("a/live"))));
}

#[tokio::test(start_paused = true)]
async fn a_close_before_the_first_poll_or_while_down_ends_the_stream_without_resync() {
    let up = hub(SourceState::Up);
    let mut first = ActorChanges::new(up.clone(), "");
    up.close();
    assert_eq!(next(&mut first).await, Some(None));
    assert_eq!(next(&mut ActorChanges::new(up.clone(), "")).await, Some(None), "subscribed after close");
    up.up();
    assert_eq!(up.state(), SourceState::Closed, "a closed hub cannot be revived");

    let down = hub(SourceState::Down);
    let mut waiting = ActorChanges::new(down.clone(), "");
    assert_eq!(next(&mut waiting).await, None);
    down.close();
    assert_eq!(next(&mut waiting).await, Some(None));
}

#[tokio::test(start_paused = true)]
async fn a_close_wins_over_queued_hints() {
    let hub = hub(SourceState::Up);
    let mut changes = ActorChanges::new(hub.clone(), "");
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    hub.publish(batch(&["a/1", "a/2"]));
    assert_eq!(next(&mut changes).await, Some(Some(changed("a/1"))));
    hub.publish(batch(&["a/3"]));
    hub.close();
    assert_eq!(next(&mut changes).await, Some(None), "a/2 and a/3 are dropped");
    assert_eq!(next(&mut changes).await, Some(None));
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_next_loses_nothing_and_dropping_one_stream_leaves_the_other() {
    let hub = hub(SourceState::Up);
    let mut kept = ActorChanges::new(hub.clone(), "");
    let mut dropped = ActorChanges::new(hub.clone(), "");
    assert_eq!(next(&mut kept).await, Some(Some(ActorChange::Resync)));
    assert_eq!(next(&mut dropped).await, Some(Some(ActorChange::Resync)));
    assert_eq!(kept.next().now_or_never(), None, "pending, then cancelled");
    drop(dropped);
    hub.publish(batch(&["a/1"]));
    assert_eq!(next(&mut kept).await, Some(Some(changed("a/1"))));
}

#[tokio::test(start_paused = true)]
async fn a_busy_source_of_other_prefixes_yields_to_the_runtime_within_one_poll() {
    let hub = hub(SourceState::Up);
    let mut changes = ActorChanges::new(hub.clone(), "mine/");
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    for i in 0..500 {
        hub.publish(batch(&[&format!("other/{i}")]));
        hub.payload("counter");
    }
    hub.publish(batch(&["mine/1"]));
    assert_eq!(changes.next().now_or_never(), None, "one poll does bounded work and yields");
    assert_eq!(next(&mut changes).await, Some(Some(changed("mine/1"))));
}
