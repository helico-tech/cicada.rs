//! Example 1: the stateless cleanup singleton.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::ActorRef;
use cicada::store::OutcomeStatus;
use cicada::time::{self, Timestamp};
use cicada_examples::singleton::{Cleanup, CleanupActor, CleanupMsg, TempFiles};
use cicada_test::TestActors;

/// Records every call and reports three purged files.
#[derive(Clone, Default)]
struct FakeFiles {
    calls: Arc<Mutex<Vec<(Timestamp, String)>>>,
}

impl TempFiles for FakeFiles {
    async fn purge_older_than(&self, cutoff: Timestamp, idempotency_key: &str) -> i32 {
        self.calls.lock().unwrap().push((cutoff, idempotency_key.to_owned()));
        3
    }
}

async fn harness(files: &FakeFiles) -> TestActors {
    let files = files.clone();
    TestActors::builder().with(move |node| node.host(CleanupActor { files })).build().await.unwrap()
}

#[tokio::test(start_paused = true)]
async fn the_singleton_purges_with_the_message_id_as_idempotency_key() {
    let files = FakeFiles::default();
    let test = harness(&files).await;
    test.create_stateless(&Cleanup).await;
    test.create_stateless(&Cleanup).await; // idempotent: every node may create it at start
    let purge = test.send(&Cleanup, CleanupMsg::Purge { retention_hours: 6 }).await;
    test.run_until_idle().await;

    let cutoff = time::sub(test.now(), Duration::from_secs(6 * 3600));
    assert_eq!(*files.calls.lock().unwrap(), vec![(cutoff, purge.id.as_str().to_owned())]);
    assert_eq!(test.outcome(&purge).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(common::list(&test, "actor://maintenance/").await, vec!["actor://maintenance/cleanup"]);
}

#[tokio::test(start_paused = true)]
async fn an_invalid_request_fails_only_its_own_message() {
    let files = FakeFiles::default();
    let test = harness(&files).await;
    test.create_stateless(&Cleanup).await;
    let bad = test.send(&Cleanup, CleanupMsg::Purge { retention_hours: -1 }).await;
    // The payload's default: 24 hours.
    let good = test.send_raw(&Cleanup.address().unwrap(), "Purge", serde_json::json!({}), Default::default()).await;
    test.run_until_idle().await;

    assert_eq!(test.outcome(&bad).await.unwrap().status, OutcomeStatus::Failed);
    let cicada::SendResult::Accepted(good) = good else { panic!("refused: {good:?}") };
    assert_eq!(test.outcome(&good).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(files.calls.lock().unwrap().len(), 1);
    assert_eq!(files.calls.lock().unwrap()[0].0, time::sub(test.now(), Duration::from_secs(24 * 3600)));
}
