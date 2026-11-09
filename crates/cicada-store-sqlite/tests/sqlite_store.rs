//! File format, schema versions and closing.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use cicada::ActorAddress;
use cicada::address::UriTemplate;
use cicada::retention::{Keep, Retention};
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::{
    CommitRequest, CommitResult, CreateResult, Delivery, EnqueueResult, MessageStatus, NodeId, OutgoingMessage, Store,
    StoreError, TurnOutcome,
};
use cicada::time::{self, Timestamp};
use cicada_store_conformance::outcome_record;
use cicada_store_sqlite::{SqliteOptions, SqliteStore, schema};
use cicada_test::VirtualClock;
use common::TempDb;
use serde_json::json;

fn template() -> UriTemplate {
    UriTemplate::parse("actor://x/{id}").unwrap()
}

fn instance(id: &str) -> ActorAddress {
    template().expand(&BTreeMap::from([("id".to_owned(), id.to_owned())])).unwrap()
}

fn at_seconds(seconds: i64) -> Timestamp {
    time::from_micros(seconds * 1_000_000)
}

fn clock_at(seconds: i64) -> Arc<VirtualClock> {
    Arc::new(VirtualClock::new(at_seconds(seconds)))
}

fn types() -> BTreeSet<UriTemplate> {
    BTreeSet::from([template()])
}

#[tokio::test]
async fn database_uses_wal_and_has_the_four_tables() {
    let db = TempDb::new();
    db.open(Arc::new(VirtualClock::default())).await.close().await;
    assert_eq!(db.pragma("journal_mode").await, "wal");
    assert_eq!(db.pragma("user_version").await, schema::VERSION.to_string());
    let tables = db.column_names("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").await;
    assert_eq!(tables, ["actor_types", "actors", "messages", "outcomes", "sequences"]);
}

#[tokio::test]
async fn reopening_an_existing_database_keeps_its_data() {
    let db = TempDb::new();
    let a = instance("1");
    let store = db.open(Arc::new(VirtualClock::default())).await;
    assert_eq!(store.create(&a, &template(), None).await.unwrap(), CreateResult::Created);
    store.close().await;
    let store = db.open(Arc::new(VirtualClock::default())).await;
    assert_eq!(store.get(&a).await.unwrap().unwrap().template, template());
    store.close().await;
}

#[tokio::test]
async fn a_stored_retry_policy_this_version_cannot_decode_reads_as_no_policy_and_does_not_break_claims() {
    // Retry v3 review L1: e.g. a backoff kind written by a newer version, or a damaged row.
    let db = TempDb::new();
    let a = instance("1");
    let policy = RetryPolicy::new(3, Backoff::Fixed { delay: Duration::from_secs(1) }).unwrap();
    let store = db.open(clock_at(10)).await;
    store.create(&a, &template(), None).await.unwrap();
    let mut ids = Vec::new();
    for n in [1, 2] {
        let message = OutgoingMessage { retry: Some(policy.clone()), ..OutgoingMessage::new(a.clone(), "M", json!(n)) };
        let EnqueueResult::Enqueued(m) = store.enqueue(message).await.unwrap() else { panic!("enqueued") };
        ids.push(m.id);
    }
    let (bad, garbled) = (&ids[0], &ids[1]);
    db.exec(&format!(
        r#"UPDATE messages SET retry_policy = '{{"maxAttempts":3,"backoff":{{"kind":"jittered","delay":"PT1S"}}}}' WHERE id = '{bad}';
           UPDATE messages SET retry_policy = 'not json' WHERE id = '{garbled}';"#
    ))
    .await;
    let retries: Vec<_> = store.pending(&a, None, 100).await.unwrap().items.into_iter().map(|m| m.retry).collect();
    assert_eq!(retries, [None, None]);
    let Some(MessageStatus::Pending(m)) = store.status(bad).await.unwrap() else { panic!("pending") };
    assert!(m.retry.is_none());
    let node = NodeId::new("n").unwrap();
    let claim = store.claim_due(&types(), &node, Duration::from_secs(30), 1).await.unwrap().remove(0);
    assert_eq!(&claim.message.id, bad);
    assert!(claim.message.retry.is_none());
    let result =
        store.commit(CommitRequest::new(a.clone(), claim.epoch, bad.clone(), TurnOutcome::Done)).await.unwrap();
    assert!(matches!(result, CommitResult::Committed { .. }));
    assert!(outcome_record(&store, bad).await.unwrap().envelope.retry.is_none());
    let next = store.claim_due(&types(), &node, Duration::from_secs(30), 1).await.unwrap().remove(0);
    assert_eq!(&next.message.id, garbled);
    assert!(next.message.retry.is_none());
    store.close().await;
}

#[tokio::test]
async fn a_database_from_a_newer_schema_version_is_refused() {
    let db = TempDb::new();
    db.open(Arc::new(VirtualClock::default())).await.close().await;
    db.exec("PRAGMA user_version = 99").await;
    let result = SqliteStore::open(&db.file, SqliteOptions::default()).await;
    let Err(StoreError::Invalid(reason)) = result else { panic!("refused, got {result:?}") };
    assert_eq!(reason, format!("Database schema version 99 is newer than supported ({})", schema::VERSION));
}

#[tokio::test]
async fn a_closed_store_refuses_operations() {
    let db = TempDb::new();
    let store = db.open(Arc::new(VirtualClock::default())).await;
    store.close().await;
    store.close().await; // idempotent
    let error = store.get(&instance("1")).await.unwrap_err();
    assert!(error.is_unavailable());
    assert_eq!(error.to_string(), "store unavailable: Store is closed");
    assert!(store.create(&instance("1"), &template(), None).await.unwrap_err().is_unavailable());
}

#[tokio::test]
async fn store_time_never_goes_backwards_across_a_reopen() {
    // The high-water mark is persisted in `sequences('clock')`, so it holds for a restarted process whose clock
    // stands earlier.
    let db = TempDb::new();
    let store = db.open(clock_at(1_000)).await;
    store.create(&instance("1"), &template(), None).await.unwrap(); // a write persists the mark
    store.close().await;
    assert_eq!(db.text("SELECT value FROM sequences WHERE name = 'clock'").await.as_deref(), Some("1000000000"));
    let store = db.open(clock_at(10)).await;
    assert_eq!(store.now().await.unwrap(), at_seconds(1_000));
    store.close().await;
}

#[tokio::test]
async fn instants_are_integer_microseconds_and_json_is_text() {
    let db = TempDb::new();
    let clock = Arc::new(VirtualClock::new(time::from_micros(1_234_567)));
    let store = db.open(clock).await;
    let a = instance("1");
    store.create(&a, &template(), Some(json!({"n": 1}))).await.unwrap();
    store
        .enqueue(OutgoingMessage {
            headers: BTreeMap::from([("h".to_owned(), "v".to_owned())]),
            ..OutgoingMessage::new(a.clone(), "M", json!({"x": [1, 2]}))
        })
        .await
        .unwrap();
    store.close().await;
    assert_eq!(
        db.text("SELECT typeof(created_at) || ':' || created_at FROM actors").await.as_deref(),
        Some("integer:1234567")
    );
    assert_eq!(db.text("SELECT state FROM actors").await.as_deref(), Some(r#"{"n":1}"#));
    assert_eq!(db.text("SELECT payload FROM messages").await.as_deref(), Some(r#"{"x":[1,2]}"#));
    assert_eq!(db.text("SELECT headers FROM messages").await.as_deref(), Some(r#"{"h":"v"}"#));
    assert_eq!(db.text("SELECT deliver_at FROM messages").await.as_deref(), Some("1234567"));
}

#[tokio::test]
async fn a_hidden_record_is_minimal_without_payload_headers_correlation_id_or_errors() {
    // S10: a hidden record only keeps its id in use. The store never returns it, so its content is read from the file.
    let db = TempDb::new();
    let clock = clock_at(10);
    let store = db.open(clock.clone()).await;
    let a = instance("1");
    store.create(&a, &template(), None).await.unwrap();
    let node = NodeId::new("n").unwrap();
    // The first record is removed as soon as it fails; the second is hidden a second later and stripped by the sweep.
    let hide_later = Retention { done: None, failed: Some(Keep::For { age: Duration::from_secs(1) }) };
    let mut ids = Vec::new();
    for retention in [Retention::REMOVE_ON_FAILED, hide_later] {
        let message = OutgoingMessage {
            correlation_id: Some("corr".into()),
            headers: BTreeMap::from([("h".to_owned(), "v".to_owned())]),
            retention: Some(retention),
            ..OutgoingMessage::new(a.clone(), "M", json!({"secret": 1}))
        };
        let EnqueueResult::Enqueued(m) = store.enqueue(message).await.unwrap() else { panic!("enqueued") };
        let outcomes = [
            TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "again".into() },
            TurnOutcome::Failed("boom".into()),
        ];
        for outcome in outcomes {
            let claim = store.claim_due(&types(), &node, Duration::from_secs(30), 1).await.unwrap().remove(0);
            let result = store.commit(CommitRequest::new(a.clone(), claim.epoch, m.id.clone(), outcome)).await.unwrap();
            assert!(matches!(result, CommitResult::Committed { .. }));
        }
        ids.push(m.id);
    }
    clock.advance_by(Duration::from_secs(2));
    assert_eq!(store.sweep_expired().await.unwrap(), 0, "both records stay for the dedup window");
    store.close().await;
    for id in &ids {
        let row = db
            .text(&format!(
                "SELECT json_array(error, stripped, json(envelope)) FROM outcomes WHERE message_id = '{id}'"
            ))
            .await
            .expect("the minimal record");
        let row: serde_json::Value = serde_json::from_str(&row).unwrap();
        assert_eq!((&row[0], &row[1]), (&json!(null), &json!(1)), "no error, marked as stripped");
        let envelope = &row[2];
        assert_eq!(envelope["payload"], json!(null));
        assert_eq!(envelope["headers"], json!({}));
        assert_eq!(envelope["correlationId"], json!(null));
        assert_eq!(envelope["lastError"], json!(null));
        assert_eq!((&envelope["type"], &envelope["to"]), (&json!("M"), &json!(a.as_str())), "type and target stay");
    }
}
