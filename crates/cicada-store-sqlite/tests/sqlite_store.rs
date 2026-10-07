//! Port of the Kotlin `SqliteStoreTest`: file format, migrations of older databases, rolling deploys and closing.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use cicada::ActorAddress;
use cicada::address::UriTemplate;
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::{
    CommitRequest, CommitResult, CreateResult, Delivery, EnqueueResult, MessageId, MessageStatus, NodeId,
    OutcomeStatus, OutgoingMessage, Store, StoreError, TurnOutcome,
};
use cicada::time::{self, Timestamp};
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

/// Writes a database at schema `version` with the statements of that time, holding the instance `actor://x/1` with
/// one pending message `m-pending` and a `done` outcome `m-done` (from version 2: naming a retry).
async fn old_database(db: &TempDb, version: usize) {
    let mut sql = String::new();
    for v in 1..=version {
        for statement in schema::statements_of(v) {
            sql.push_str(statement);
            sql.push_str(";\n");
        }
    }
    sql.push_str(
        "INSERT INTO actors (uri, type, created_at, next_due_at) VALUES ('actor://x/1', 'actor://x/{id}', 0, 0);\n",
    );
    sql.push_str(
        "INSERT INTO messages (id, actor_uri, seq, type, payload, headers, deliver_at, attempt) \
         VALUES ('m-pending', 'actor://x/1', 2, 'M', '1', '{}', 0, 0);\n",
    );
    let envelope = r#"{"id":"m-done","to":"actor://x/1","from":null,"type":"M","payload":1,"deliverAt":"1970-01-01T00:00:00Z","correlationId":null,"headers":{},"seq":1,"attempt":1}"#;
    let (column, value) = if version >= 2 { (", retried_as", ", 'm-retry'") } else { ("", "") };
    sql.push_str(&format!(
        "INSERT INTO outcomes (message_id, actor_uri, status, error, finished_at, envelope{column}) \
         VALUES ('m-done', 'actor://x/1', 'done', NULL, 0, '{envelope}'{value});\n"
    ));
    sql.push_str("UPDATE sequences SET value = 2 WHERE name = 'seq';\n");
    sql.push_str(&format!("PRAGMA user_version = {version};\n"));
    db.exec(&sql).await;
}

#[tokio::test]
async fn version_1_to_5_databases_are_migrated_and_keep_their_data() {
    for version in 1..=5 {
        let db = TempDb::new();
        old_database(&db, version).await;
        let a = instance("1");
        let store = db.open(clock_at(10)).await;
        assert_eq!(db.pragma("user_version").await, schema::VERSION.to_string(), "from version {version}");
        let messages = db.columns("messages").await;
        let outcomes = db.columns("outcomes").await;
        for column in ["failures", "last_error", "retry_policy", "caused_by", "retention_policy"] {
            assert!(messages.iter().any(|c| c == column), "messages.{column} from version {version}");
        }
        for column in ["caused_by", "visible_until", "keep_last", "keep_seq", "stripped"] {
            assert!(outcomes.iter().any(|c| c == column), "outcomes.{column} from version {version}");
        }
        // Additive (review H1): nodes still on version 2 write and read `retried_as` during a rolling deploy.
        assert!(outcomes.iter().any(|c| c == "retried_as"), "retried_as is kept, unused");

        let done = store.outcome(&MessageId::new("m-done").unwrap()).await.unwrap().expect("the old outcome");
        assert_eq!(done.status, OutcomeStatus::Done);
        assert_eq!(done.envelope.failures, 0);
        assert!(done.envelope.retry.is_none(), "older envelopes have no policy");
        assert!(done.envelope.retention.is_none(), "older envelopes have no policy");
        assert!(done.envelope.caused_by.is_none());

        let claims = store.claim_due(&types(), &NodeId::new("n").unwrap(), Duration::from_secs(30), 1).await.unwrap();
        let [claim] = claims.as_slice() else { panic!("one claim, got {claims:?}") };
        assert_eq!(claim.message.id.as_str(), "m-pending");
        assert_eq!(claim.message.failures, 0);
        assert!(claim.message.last_error.is_none());
        assert!(claim.message.retry.is_none());
        assert!(claim.message.caused_by.is_none(), "older messages have no known cause");
        let rescheduled =
            TurnOutcome::Rescheduled { delivery: Delivery::After(Duration::from_secs(1)), error: "boom".into() };
        let result = store
            .commit(CommitRequest::new(a.clone(), claim.epoch, claim.message.id.clone(), rescheduled))
            .await
            .unwrap();
        assert!(matches!(result, CommitResult::Committed { .. }), "{result:?}");
        let pending = store.pending(&a, None, 100).await.unwrap().items;
        let [pending] = pending.as_slice() else { panic!("one pending message, got {pending:?}") };
        assert_eq!(pending.failures, 1);
        assert_eq!(pending.last_error.as_deref(), Some("boom"));
        assert!(pending.seq > 2, "a new seq from the migrated sequence");
        assert_eq!(pending.deliver_at, time::add(store.now().await.unwrap(), Duration::from_secs(1)).unwrap());
        store.close().await;
    }
}

#[tokio::test]
async fn version_2_nodes_can_still_write_and_read_after_the_latest_migration() {
    // Review H1: during a rolling deploy, nodes on version 2 keep running against the migrated database.
    for fresh in [true, false] {
        let db = TempDb::new();
        if !fresh {
            old_database(&db, 2).await;
        }
        let clock = clock_at(10);
        db.open(clock.clone()).await.close().await;
        assert_eq!(db.pragma("user_version").await, schema::VERSION.to_string());
        let envelope = r#"{"id":"m-v2","to":"actor://x/1","from":null,"type":"M","payload":1,"deliverAt":"1970-01-01T00:00:00Z","correlationId":null,"headers":{},"seq":5,"attempt":1}"#;
        // The version 2 statements, verbatim in shape: outcome upsert naming `retried_as`, message insert without
        // `failures` / `last_error`.
        db.exec(&format!(
            "INSERT INTO outcomes (message_id, actor_uri, status, error, finished_at, envelope, retried_as) \
             VALUES ('m-v2', 'actor://x/1', 'done', NULL, 10000000, '{envelope}', 'm-v2-retry') \
             ON CONFLICT (message_id) DO UPDATE SET actor_uri = excluded.actor_uri, status = excluded.status, \
             error = excluded.error, finished_at = excluded.finished_at, envelope = excluded.envelope, \
             retried_as = excluded.retried_as;
             INSERT INTO messages (id, actor_uri, seq, type, payload, headers, deliver_at, attempt) \
             VALUES ('m-v2-pending', 'actor://x/1', 99, 'M', '1', '{{}}', 0, 0);"
        ))
        .await;
        assert_eq!(
            db.text("SELECT retried_as FROM outcomes WHERE message_id = 'm-v2'").await.as_deref(),
            Some("m-v2-retry")
        );
        assert_eq!(db.text("SELECT failures FROM messages WHERE id = 'm-v2-pending'").await.as_deref(), Some("0"));
        assert_eq!(
            db.text("SELECT count(*) FROM messages WHERE id = 'm-v2-pending' AND last_error IS NULL AND retry_policy IS NULL")
                .await
                .as_deref(),
            Some("1")
        );

        let store = db.open(clock.clone()).await;
        let outcome = store.outcome(&MessageId::new("m-v2").unwrap()).await.unwrap().expect("the version 2 outcome");
        assert_eq!(outcome.status, OutcomeStatus::Done);
        // A message inserted the version 2/3 way (no retry_policy) reads as one without a policy.
        let status = store.status(&MessageId::new("m-v2-pending").unwrap()).await.unwrap();
        let Some(MessageStatus::Pending(pending)) = status else { panic!("pending, got {status:?}") };
        assert!(pending.retry.is_none());
        assert!(pending.caused_by.is_none(), "inserted without caused_by: no known cause");
        store.close().await;
    }
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
    assert!(store.outcome(bad).await.unwrap().unwrap().envelope.retry.is_none());
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
async fn zero_readers_are_refused() {
    let db = TempDb::new();
    let result = SqliteStore::open(&db.file, SqliteOptions { readers: 0, ..SqliteOptions::default() }).await;
    assert!(matches!(result, Err(StoreError::Invalid(ref r)) if r == "readers must be positive"), "{result:?}");
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
