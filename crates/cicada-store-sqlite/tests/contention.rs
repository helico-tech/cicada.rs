//! Port of the Kotlin `SqliteContentionTest`: write contention with another process on the same file (`SQLITE_BUSY`)
//! and fault injection inside a commit (S2). The "other process" is a raw connection holding the write lock; faults are
//! injected with a trigger.

mod common;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Duration;

use cicada::ActorAddress;
use cicada::address::UriTemplate;
use cicada::store::{
    Claim, CommitRequest, CommitResult, EnqueueResult, MessageStatus, NodeId, OutgoingMessage, StateChange, Store,
    TurnOutcome,
};
use cicada_test::VirtualClock;
use common::{TempDb, open_with};
use serde_json::json;

fn template() -> UriTemplate {
    UriTemplate::parse("actor://busy/{id}").unwrap()
}

fn instance(id: &str) -> ActorAddress {
    template().expand(&BTreeMap::from([("id".to_owned(), id.to_owned())])).unwrap()
}

/// Makes every insert of a message of type `Poison` fail, in the middle of whatever transaction does it.
async fn install_poison_trigger(db: &TempDb) {
    db.exec(
        "CREATE TRIGGER poison BEFORE INSERT ON messages WHEN NEW.type = 'Poison' \
         BEGIN SELECT RAISE(ABORT, 'injected fault'); END",
    )
    .await;
}

async fn claim_one(store: &impl Store) -> Claim {
    let claims = store
        .claim_due(&BTreeSet::from([template()]), &NodeId::new("n").unwrap(), Duration::from_secs(30), 1)
        .await
        .unwrap();
    let [claim] = <[Claim; 1]>::try_from(claims).expect("one claim");
    claim
}

/// Commits `a`'s claim with a good send followed by a poisoned one; asserts nothing was applied.
async fn assert_failed_commit_applies_nothing(store: &impl Store, claim: Claim) {
    let a = instance("a");
    let target = instance("target");
    store.create(&target, &template(), None).await.unwrap();
    let commit = CommitRequest {
        state: StateChange::NewState(Some(json!(1))),
        sends: vec![
            OutgoingMessage::new(target.clone(), "Good", json!(1)),
            OutgoingMessage::new(target.clone(), "Poison", json!(2)),
        ],
        ..CommitRequest::new(a.clone(), claim.epoch, claim.message.id.clone(), TurnOutcome::Done)
    };
    let error = store.commit(commit).await.expect_err("the injected fault fails the commit");
    assert!(error.is_unavailable(), "{error}");
    assert!(store.outcome(&claim.message.id).await.unwrap().is_none(), "no outcome recorded");
    let status = store.status(&claim.message.id).await.unwrap();
    assert!(matches!(status, Some(MessageStatus::Running { .. })), "the lease is still held: {status:?}");
    assert!(store.pending(&target, None, 100).await.unwrap().items.is_empty(), "no send applied");
    assert_eq!(store.get(&a).await.unwrap().unwrap().version, 0, "state untouched");
    let result =
        store.commit(CommitRequest::new(a, claim.epoch, claim.message.id.clone(), TurnOutcome::Done)).await.unwrap();
    assert!(matches!(result, CommitResult::Committed { .. }), "{result:?}");
}

#[tokio::test]
async fn s2_a_commit_failing_half_way_applies_nothing() {
    let db = TempDb::new();
    let store = db.open(Arc::new(VirtualClock::default())).await;
    let a = instance("a");
    store.create(&a, &template(), None).await.unwrap();
    store.enqueue(OutgoingMessage::new(a.clone(), "M", json!(0))).await.unwrap();
    install_poison_trigger(&db).await;
    let claim = claim_one(&store).await;
    assert_failed_commit_applies_nothing(&store, claim).await;
    store.close().await;
}

#[tokio::test]
async fn a_write_that_times_out_on_another_process_lock_leaves_the_writer_transactional() {
    let db = TempDb::new();
    let store = open_with(&db.file, Arc::new(VirtualClock::default()), Duration::from_millis(50)).await;
    let a = instance("a");
    store.create(&a, &template(), None).await.unwrap();
    store.enqueue(OutgoingMessage::new(a.clone(), "M", json!(0))).await.unwrap();
    install_poison_trigger(&db).await;
    {
        let mut other = db.raw().await;
        sqlx::raw_sql("BEGIN IMMEDIATE").execute(&mut other).await.unwrap();
        let error = store
            .enqueue(OutgoingMessage::new(a.clone(), "M", json!(1)))
            .await
            .expect_err("SQLITE_BUSY while another process writes");
        assert!(error.is_unavailable(), "{error}");
        sqlx::raw_sql("ROLLBACK").execute(&mut other).await.unwrap();
    }
    // The store recovered: writes work and are atomic again.
    let result = store.enqueue(OutgoingMessage::new(a.clone(), "M", json!(2))).await.unwrap();
    assert!(matches!(result, EnqueueResult::Enqueued(_)), "{result:?}");
    let claim = claim_one(&store).await;
    assert_failed_commit_applies_nothing(&store, claim).await;
    store.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_stores_on_one_file_contend_for_the_write_lock_without_losing_atomicity() {
    let db = TempDb::new();
    let clock = Arc::new(VirtualClock::default());
    let first = Arc::new(db.open(clock.clone()).await);
    let second = Arc::new(db.open(clock.clone()).await);
    let a = instance("a");
    first.create(&a, &template(), None).await.unwrap();
    let total = 200;
    let tasks: Vec<_> = [first.clone(), second.clone()]
        .into_iter()
        .enumerate()
        .map(|(i, store)| {
            let a = a.clone();
            tokio::spawn(async move {
                for n in 0..total / 2 {
                    let result =
                        store.enqueue(OutgoingMessage::new(a.clone(), format!("M{i}"), json!(n))).await.unwrap();
                    assert!(matches!(result, EnqueueResult::Enqueued(_)));
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
    let pending = first.pending(&a, None, 1000).await.unwrap().items;
    assert_eq!(pending.len(), total);
    let mut by_seq = pending.clone();
    by_seq.sort_by_key(|m| m.seq);
    assert_eq!(by_seq, pending);
    assert_eq!(pending.iter().map(|m| m.seq).collect::<HashSet<_>>().len(), total, "distinct seqs");
    first.close().await;
    second.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_write_leaves_no_transaction_open() {
    // A caller that drops a store future mid-write (a timeout, an aborted task) must not leave the writer inside a
    // transaction: the next write rolls it back first and works.
    let db = TempDb::new();
    let store = Arc::new(open_with(&db.file, Arc::new(VirtualClock::default()), Duration::from_secs(5)).await);
    let a = instance("a");
    store.create(&a, &template(), None).await.unwrap();
    let mut other = db.raw().await;
    sqlx::raw_sql("BEGIN IMMEDIATE").execute(&mut other).await.unwrap();
    // The write waits on the other connection's lock and is cancelled while it waits.
    let cancelled =
        tokio::time::timeout(Duration::from_millis(100), store.enqueue(OutgoingMessage::new(a.clone(), "M", json!(0))))
            .await;
    assert!(cancelled.is_err(), "the write was still waiting");
    sqlx::raw_sql("ROLLBACK").execute(&mut other).await.unwrap();
    let result = store.enqueue(OutgoingMessage::new(a.clone(), "M", json!(1))).await.unwrap();
    assert!(matches!(result, EnqueueResult::Enqueued(_)), "{result:?}");
    // The other connection can write again too: no transaction of the store holds the lock.
    sqlx::raw_sql("BEGIN IMMEDIATE").execute(&mut other).await.unwrap();
    sqlx::raw_sql("ROLLBACK").execute(&mut other).await.unwrap();
    assert_eq!(store.pending(&a, None, 10).await.unwrap().items.len(), 1);
    store.close().await;
}
