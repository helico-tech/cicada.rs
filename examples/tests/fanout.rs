//! Example 7: fan-out and fan-in.

mod common;

use std::collections::BTreeMap;

use cicada_examples::fanout::{
    Batch, BatchActor, BatchItem, BatchItemActor, BatchMsg, BatchState, BatchStatus, ItemProcessor,
};
use cicada_test::TestActors;

/// The result of an item is the length of its input.
struct Length;

impl ItemProcessor for Length {
    async fn process(&self, input: &str) -> i64 {
        input.len() as i64
    }
}

fn batch() -> Batch {
    Batch { id: "b-1".into() }
}

fn start(items: &[&str]) -> BatchMsg {
    BatchMsg::StartBatch { items: items.iter().map(|s| (*s).to_owned()).collect() }
}

async fn harness() -> TestActors {
    TestActors::builder()
        .with(|node| node.host(BatchActor).host(BatchItemActor { processor: Length }))
        .build()
        .await
        .unwrap()
}

async fn batch_state(test: &TestActors, batch: &Batch) -> BatchState {
    test.state(batch).await
}

#[tokio::test(start_paused = true)]
async fn the_parent_fans_out_to_children_and_counts_their_replies() {
    let test = harness().await;
    test.create(&batch(), &BatchState::default()).await;
    test.send(&batch(), start(&["a", "bb", "ccc", "dddd", "eeeee"])).await;
    test.run_until_idle().await;

    let state = batch_state(&test, &batch()).await;
    assert_eq!(state.status, BatchStatus::Completed);
    assert_eq!(state.total, 5);
    assert_eq!(state.results, BTreeMap::from([(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)]));
    assert_eq!(state.sum(), 15);
    // The children stopped after reporting; only the parent is left in the namespace.
    assert_eq!(common::list(&test, "actor://batches/").await, vec!["actor://batches/b-1"]);
}

#[tokio::test(start_paused = true)]
async fn while_running_the_children_are_instances_of_their_own() {
    let test = harness().await;
    test.create(&batch(), &BatchState::default()).await;
    test.send(&batch(), start(&["x", "y"])).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1); // only the parent's turn
    assert!(test.exists(&BatchItem { batch: "b-1".into(), index: 0 }).await);
    assert!(test.exists(&BatchItem { batch: "b-1".into(), index: 1 }).await);
    assert_eq!(batch_state(&test, &batch()).await.status, BatchStatus::Running);
    test.run_until_idle().await;
    assert_eq!(batch_state(&test, &batch()).await.status, BatchStatus::Completed);
}

#[tokio::test(start_paused = true)]
async fn a_duplicate_reply_counts_once_and_an_empty_batch_completes_at_once() {
    let test = harness().await;
    test.create(&batch(), &BatchState::default()).await;
    test.send(&batch(), start(&["x", "y"])).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    test.send(&batch(), BatchMsg::ItemDone { index: 0, result: 99 }).await; // before the child's own reply for 0
    test.run_until_idle().await;
    assert_eq!(batch_state(&test, &batch()).await.results, BTreeMap::from([(0, 99), (1, 1)]));

    let empty = Batch { id: "empty".into() };
    test.create(&empty, &BatchState::default()).await;
    test.send(&empty, start(&[])).await;
    test.run_until_idle().await;
    assert_eq!(batch_state(&test, &empty).await.status, BatchStatus::Completed);
}
