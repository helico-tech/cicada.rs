//! The change feed of a node (ADR 0034) on the in-memory store: who may subscribe, what a turn and an operator write
//! reports, and how long a stream lives.

use std::time::Duration;

use futures::StreamExt;
use serde_json::json;

use super::common::*;
use crate::change::{ActorChange, ActorChanges};
use crate::testing::TestActors;
use crate::{ActorSystem, SendOptions};

async fn harness() -> TestActors {
    TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap()
}

/// The next item, or `None` for "nothing within a minute of virtual time".
async fn next(changes: &mut ActorChanges) -> Option<Option<ActorChange>> {
    tokio::time::timeout(Duration::from_secs(60), changes.next()).await.ok()
}

fn changed(counter: &Counter) -> Option<Option<ActorChange>> {
    Some(Some(ActorChange::changed(counter.address())))
}

/// Takes items until the change of `marker`, which a test writes last; returns the ones before it.
async fn until(changes: &mut ActorChanges, marker: &Counter) -> Vec<ActorChange> {
    let mut seen = Vec::new();
    loop {
        match next(changes).await {
            Some(Some(change)) if change == ActorChange::changed(marker.address()) => return seen,
            Some(Some(change)) => seen.push(change),
            other => panic!("no marker; got {other:?} after {seen:?}"),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn an_unstarted_system_that_hosts_no_type_receives_hints_and_reads_the_actors() {
    let test = harness().await;
    let reader = ActorSystem::builder(test.store().clone()).build();
    let mut changes = reader.changes("counter/");
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    test.create(&Counter::new("a"), &CounterState::total(1)).await;
    assert_eq!(next(&mut changes).await, changed(&Counter::new("a")));
    let page = reader.list_actors_with_state("counter/", None, 10).await.unwrap();
    assert_eq!(page.items[0].state["total"], 1);
    assert_eq!(reader.get_actor(&Counter::new("a").address()).await.unwrap().unwrap().version, 0);
}

#[tokio::test(start_paused = true)]
async fn turns_report_new_states_creates_and_stops_and_not_their_mailbox_work() {
    let test = harness().await;
    let mut changes = test.system().changes("");
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    let (parent, child) = (Counter::new("p"), Counter::new("p-child"));
    test.create(&parent, &CounterState::default()).await;
    assert_eq!(next(&mut changes).await, changed(&parent));

    // A new state: one hint.
    test.send(&parent, CounterMsg::Add { n: 1 }).await;
    test.run_until_idle().await;
    assert_eq!(next(&mut changes).await, changed(&parent));
    // A turn that keeps its state creates a counter and sends to it: the create, then the new counter's new state.
    test.send(&parent, CounterMsg::Spawn { child: "p-child".into(), n: 2 }).await;
    test.run_until_idle().await;
    let marker = Counter::new("z1");
    test.create(&marker, &CounterState::default()).await;
    assert_eq!(until(&mut changes, &marker).await, vec![ActorChange::changed(child.address()); 2]);
    // Mailbox work alone (a delayed send, a cancel) reports nothing; a stop reports its actor.
    let later = SendOptions::new().delay(Duration::from_secs(600));
    let later = test.send_with(&parent, CounterMsg::Add { n: 1 }, later).await;
    test.system().cancel(&later.id).await.unwrap();
    test.send(&parent, CounterMsg::Halt).await;
    test.run_until_idle().await;
    let marker = Counter::new("z2");
    test.create(&marker, &CounterState::default()).await;
    assert_eq!(until(&mut changes, &marker).await, vec![ActorChange::changed(parent.address())]);
    assert!(!test.exists(&parent).await);
}

#[tokio::test(start_paused = true)]
async fn operators_report_creates_pauses_resumes_and_deletes_and_idempotent_calls_report_nothing() {
    let test = harness().await;
    let system = test.system();
    let mut changes = system.changes("counter/");
    assert_eq!(next(&mut changes).await, Some(Some(ActorChange::Resync)));
    let a = Counter::new("a");
    system.create_actor(&a.address(), json!({"total": 1})).await.unwrap();
    system.create_actor(&a.address(), json!({"total": 2})).await.unwrap(); // exists: nothing
    system.pause_mailbox(&a.address()).await.unwrap();
    system.resume_mailbox(&a.address()).await.unwrap();
    system.pause_mailbox(&Counter::new("absent").address()).await.unwrap();
    system.purge_mailbox(&a.address()).await.unwrap();
    system.delete_actor(&a.address()).await.unwrap();
    system.delete_actor(&a.address()).await.unwrap(); // absent: nothing
    let marker = Counter::new("z");
    test.create(&marker, &CounterState::default()).await;
    assert_eq!(until(&mut changes, &marker).await, vec![ActorChange::changed(a.address()); 4]);
}

#[tokio::test(start_paused = true)]
async fn stopping_the_worker_keeps_the_feed_and_closing_the_store_ends_every_stream() {
    let store = virtual_store();
    let system = node(store.clone(), "n1").host(CounterActor).build();
    system.start().await.unwrap();
    let mut first = system.changes("");
    let mut second = system.changes("");
    assert_eq!(next(&mut first).await, Some(Some(ActorChange::Resync)));
    assert_eq!(next(&mut second).await, Some(Some(ActorChange::Resync)));
    system.stop().await;
    let a = Counter::new("a");
    system.create(&a, &CounterState::default()).await.unwrap();
    assert_eq!(next(&mut first).await, changed(&a), "the store, not the worker, owns the feed");
    drop(first);
    system.create(&Counter::new("b"), &CounterState::default()).await.unwrap();
    store.close().await;
    assert_eq!(next(&mut second).await, Some(None), "close ends it, with its backlog");
    assert_eq!(next(&mut system.changes("")).await, Some(None), "a stream after close ends at once");
}
