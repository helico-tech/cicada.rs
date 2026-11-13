//! Example 11: debouncing a burst of edits.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::NoState;
use cicada_actors::{DebouncerActor, DebouncerMsg, DebouncerState};
use cicada_examples::debounce::{
    Document, DocumentActor, DocumentMsg, DocumentState, IndexError, Indexer, IndexerActor, SearchIndex,
    reindex_debouncer,
};
use cicada_test::TestActors;

const SECOND: Duration = Duration::from_secs(1);

/// A search index that keeps every text it was given, and fails the first put of `flaky`.
#[derive(Clone, Default)]
struct FakeIndex {
    puts: Arc<Mutex<Vec<String>>>,
    failed_once: Arc<Mutex<bool>>,
}

impl SearchIndex for FakeIndex {
    async fn put(&self, _doc: &str, text: &str) -> Result<(), IndexError> {
        if text == "flaky" && !std::mem::replace(&mut *self.failed_once.lock().unwrap(), true) {
            return Err(IndexError("index down".into()));
        }
        self.puts.lock().unwrap().push(text.to_owned());
        Ok(())
    }
}

impl FakeIndex {
    fn puts(&self) -> Vec<String> {
        self.puts.lock().unwrap().clone()
    }
}

/// A node with the debouncer, the document `d1` and, unless `indexer` is false, the indexer.
async fn node(index: FakeIndex, indexer: bool) -> TestActors {
    let test = TestActors::builder()
        .with(move |node| node.host(DebouncerActor).host(DocumentActor).host(IndexerActor { index }))
        .build()
        .await
        .unwrap();
    if indexer {
        test.create(&Indexer, &NoState {}).await;
    }
    test.create(&doc(), &DocumentState::default()).await;
    test
}

fn doc() -> Document {
    Document { id: "d1".into() }
}

async fn edit(test: &TestActors, text: &str) {
    test.send(&doc(), DocumentMsg::Edit { text: text.into() }).await;
    test.run_until_idle().await;
}

#[tokio::test(start_paused = true)]
async fn three_quick_edits_reindex_the_last_version_once_after_two_quiet_seconds() {
    let index = FakeIndex::default();
    let test = node(index.clone(), true).await;
    for text in ["a", "ab", "abc"] {
        edit(&test, text).await;
        test.advance_time_by(SECOND / 2).await;
    }
    assert!(index.puts().is_empty(), "still within the quiet period");
    test.advance_time_by(SECOND + SECOND / 2).await;
    assert_eq!(index.puts(), ["abc"]);
    assert_eq!(test.state::<_, DocumentState>(&doc()).await.indexed, Some(3), "the reply reached the document");
    let debouncer: DebouncerState = test.state(&reindex_debouncer("d1")).await;
    assert_eq!((debouncer.quiet, debouncer.pending), (2 * SECOND, None), "created by the first edit, now empty");
}

#[tokio::test(start_paused = true)]
async fn edits_every_second_reindex_at_the_longest_wait() {
    let index = FakeIndex::default();
    let test = node(index.clone(), true).await;
    for n in 0..10 {
        edit(&test, &n.to_string()).await;
        test.advance_time_by(SECOND).await;
    }
    assert_eq!(index.puts(), ["9"], "ten seconds after the first edit");
    edit(&test, "10").await;
    test.advance_time_by(2 * SECOND).await;
    assert_eq!(index.puts(), ["9", "10"], "the next burst");
}

#[tokio::test(start_paused = true)]
async fn a_put_that_fails_is_tried_again_by_the_policy_of_the_submission() {
    let index = FakeIndex::default();
    let test = node(index.clone(), true).await;
    edit(&test, "flaky").await;
    test.advance_time_by(2 * SECOND).await;
    assert!(index.puts().is_empty(), "the first put failed");
    assert_eq!(test.state::<_, DebouncerState>(&reindex_debouncer("d1")).await.pending, None, "flushed");
    test.advance_time_by(5 * SECOND).await;
    assert_eq!(index.puts(), ["flaky"]);
    assert_eq!(test.state::<_, DocumentState>(&doc()).await.indexed, Some(1));
}

#[tokio::test(start_paused = true)]
async fn a_flush_to_a_missing_indexer_keeps_the_reindex_and_an_operator_flushes_it_later() {
    let index = FakeIndex::default();
    let test = node(index.clone(), false).await;
    edit(&test, "a").await;
    edit(&test, "ab").await;
    test.advance_time_by(2 * SECOND).await;
    let failures = test.failures().await;
    assert!(failures.iter().any(|f| f.envelope.message_type == "Flush"), "{failures:?}");
    let kept: DebouncerState = test.state(&reindex_debouncer("d1")).await;
    assert_eq!(kept.pending.unwrap().message.payload["version"], 2, "the latest submission stays");
    test.create(&Indexer, &NoState {}).await;
    test.send(&reindex_debouncer("d1"), DebouncerMsg::Flush).await;
    test.run_until_idle().await;
    assert_eq!(index.puts(), ["ab"]);
    assert_eq!(test.state::<_, DocumentState>(&doc()).await.indexed, Some(2));
}
