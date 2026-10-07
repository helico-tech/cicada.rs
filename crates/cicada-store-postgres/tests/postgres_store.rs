//! PostgreSQL-specific behaviour beyond the conformance suite: schema migration, clock, wake-ups, configuration. Port
//! of the Kotlin `PostgresStoreTest`.

mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use cicada::address::{ActorAddress, UriTemplate};
use cicada::store::{
    CommitRequest, CommitResult, EnqueueResult, NodeId, OutgoingMessage, Store, StoreError, TurnOutcome,
};
use cicada::time;
use cicada_store_postgres::{PostgresStore, PostgresStoreConfig, schema};
use futures::StreamExt;
use futures::future::join_all;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::timeout;

fn template() -> UriTemplate {
    UriTemplate::parse("actor://pg/{id}").unwrap()
}

fn other() -> UriTemplate {
    UriTemplate::parse("actor://pg-other/{id}").unwrap()
}

fn addr(uri: &str) -> ActorAddress {
    ActorAddress::parse(uri).unwrap()
}

fn types(t: UriTemplate) -> BTreeSet<UriTemplate> {
    BTreeSet::from([t])
}

async fn schema_versions(db: &common::TestDb) -> Vec<i32> {
    let mut c = db.connect().await;
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT version FROM \"{}\".cicada_schema_version ORDER BY version",
        db.schema
    )))
    .fetch_all(&mut c)
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_starts_on_a_new_schema_migrate_it_exactly_once() {
    let db = test_db!("s");
    join_all((0..6).map(|_| db.open())).await;
    assert_eq!(schema_versions(&db).await, (1..=schema::VERSION).collect::<Vec<_>>());
    db.open().await.close().await; // and again on an existing schema
    assert_eq!(schema_versions(&db).await, (1..=schema::VERSION).collect::<Vec<_>>());
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_schema_written_by_a_newer_version() {
    let db = test_db!("s");
    db.open().await.close().await;
    db.sql(&format!("INSERT INTO \"{}\".cicada_schema_version (version) VALUES (999)", db.schema)).await;
    match PostgresStore::connect(&db.url, db.config()).await {
        Err(StoreError::Invalid(message)) => assert!(message.contains("newer"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_stay_on_the_stores_schema_across_read_only_and_rolled_back_transactions() {
    let db = test_db!("s");
    let setup = db.open().await;
    let a = addr("actor://pg/a");
    setup.create(&a, &template(), None).await.unwrap();
    let EnqueueResult::Enqueued(m) = setup.enqueue(OutgoingMessage::new(a.clone(), "T", json!(1))).await.unwrap() else {
        panic!("enqueued")
    };
    // Concurrent operations force the pools to open fresh connections whose first transaction is read-only, or rolls
    // back (a duplicate enqueue); every connection must still see the store's schema afterwards.
    let read_only_first = db.open().await;
    let rollback_first = db.open().await;
    for _ in 0..3 {
        for found in join_all((0..8).map(|_| read_only_first.get(&a))).await {
            assert!(found.unwrap().is_some());
        }
        let duplicate = || {
            let mut again = OutgoingMessage::new(a.clone(), "T", json!(1));
            again.id = m.id.clone();
            rollback_first.enqueue(again)
        };
        for result in join_all((0..8).map(|_| duplicate())).await {
            assert_eq!(result.unwrap(), EnqueueResult::Duplicate(m.id.clone()));
        }
        for found in join_all((0..8).map(|_| rollback_first.get(&a))).await {
            assert!(found.unwrap().is_some());
        }
    }
    db.cleanup().await;
}

#[tokio::test]
async fn invalid_settings_are_refused_before_connecting() {
    let invalid = |config: PostgresStoreConfig| async move {
        matches!(PostgresStore::connect("postgres://nowhere.invalid/db", config).await, Err(StoreError::Invalid(_)))
    };
    assert!(invalid(PostgresStoreConfig::for_schema("Robert'); DROP TABLE students;--")).await);
    assert!(invalid(PostgresStoreConfig { max_pool_size: 0, ..Default::default() }).await);
    assert!(invalid(PostgresStoreConfig { channel: Some("no spaces".into()), ..Default::default() }).await);
    assert!(matches!(
        PostgresStore::connect("mysql://u:secret@db/x", PostgresStoreConfig::default()).await,
        Err(StoreError::Invalid(m)) if !m.contains("secret")
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_with_takes_sqlx_options_built_by_the_caller() {
    let db = test_db!("w");
    let options: cicada_store_postgres::PgConnectOptions = db.url.parse().unwrap();
    let store = PostgresStore::connect_with(options.application_name("custom-app"), db.config()).await.unwrap();
    store.now().await.unwrap();
    let mut c = db.connect().await;
    let names: Vec<String> =
        sqlx::query_scalar("SELECT application_name FROM pg_stat_activity WHERE application_name = 'custom-app'")
            .fetch_all(&mut c)
            .await
            .unwrap();
    assert!(!names.is_empty(), "the caller's option reaches the connections");
    store.close().await;
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_store_clock_is_the_database_clock() {
    let db = test_db!("s");
    let store = db.open().await;
    let mut c = db.connect().await;
    let db_now = async |c: &mut sqlx::PgConnection| -> time::Timestamp {
        sqlx::query_scalar("SELECT clock_timestamp()").fetch_one(c).await.unwrap()
    };
    let before = db_now(&mut c).await;
    let now = store.now().await.unwrap();
    let after = db_now(&mut c).await;
    assert!(time::truncate(before) <= now && now <= after, "{before} <= {now} <= {after}");
    assert_eq!(now, time::truncate(now), "microsecond precision");
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_store_refuses_operations() {
    let db = test_db!("s");
    let store = db.open().await;
    store.close().await;
    store.close().await; // idempotent
    match store.now().await {
        Err(e @ StoreError::Unavailable(_)) => assert!(e.to_string().contains("Store is closed"), "{e}"),
        other => panic!("expected unavailable, got {other:?}"),
    }
    db.cleanup().await;
}

/// Collects `store`'s wake-ups for `t` into a channel.
fn collect_wake_ups(store: &PostgresStore, t: UriTemplate) -> (mpsc::UnboundedReceiver<()>, tokio::task::JoinHandle<()>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let mut wake_ups = store.wake_ups(&types(t)).expect("the PostgreSQL store has wake-ups");
    let collector = tokio::spawn(async move {
        while wake_ups.next().await.is_some() {
            if sender.send(()).is_err() {
                break;
            }
        }
    });
    (receiver, collector)
}

async fn wait_listening(store: &PostgresStore) {
    common::eventually(Duration::from_secs(10), "the listener listens", || async { store.is_listening() }).await;
}

async fn none_within(receiver: &mut mpsc::UnboundedReceiver<()>, wait: Duration) -> bool {
    timeout(wait, receiver.recv()).await.is_err()
}

async fn some_within(receiver: &mut mpsc::UnboundedReceiver<()>, wait: Duration) -> bool {
    matches!(timeout(wait, receiver.recv()).await, Ok(Some(())))
}

#[tokio::test(flavor = "multi_thread")]
async fn wake_ups_follow_enqueues_sends_and_releases_of_the_requested_types_on_any_node() {
    let db = test_db!("s");
    let listening = db.open().await;
    let writer = db.open().await;
    let (mut received, collector) = collect_wake_ups(&listening, template());
    wait_listening(&listening).await;
    let short = Duration::from_millis(300);
    let long = Duration::from_secs(2);

    let a = addr("actor://pg/a");
    let b = addr("actor://pg-other/b");
    writer.create(&a, &template(), None).await.unwrap();
    writer.create(&b, &other(), None).await.unwrap();
    assert!(none_within(&mut received, short).await, "creates do not notify");

    // An enqueue on another node.
    assert!(matches!(writer.enqueue(OutgoingMessage::new(a.clone(), "T", json!(1))).await, Ok(EnqueueResult::Enqueued(_))));
    assert!(some_within(&mut received, long).await, "enqueue notifies");

    // Another type is filtered out.
    assert!(matches!(writer.enqueue(OutgoingMessage::new(b.clone(), "T", json!(1))).await, Ok(EnqueueResult::Enqueued(_))));
    assert!(none_within(&mut received, short).await, "other types are filtered");

    // A release makes the message claimable again.
    let node = NodeId::new("w").unwrap();
    let ttl = Duration::from_secs(10);
    let claim = writer.claim_due(&types(template()), &node, ttl, 1).await.unwrap().remove(0);
    writer.release(&claim.actor, claim.epoch).await.unwrap();
    assert!(some_within(&mut received, long).await, "release notifies");

    // A commit's sends notify their targets' types.
    let claim_b = writer.claim_due(&types(other()), &node, ttl, 1).await.unwrap().remove(0);
    let mut commit = CommitRequest::new(claim_b.actor, claim_b.epoch, claim_b.message.id, TurnOutcome::Done);
    commit.sends = vec![OutgoingMessage::new(a.clone(), "Reply", json!(2))];
    assert!(matches!(writer.commit(commit).await, Ok(CommitResult::Committed { .. })));
    assert!(some_within(&mut received, long).await, "a commit's send notifies");

    // A rolled-back (rejected) commit does not.
    let claim_a = writer.claim_due(&types(template()), &node, ttl, 1).await.unwrap().remove(0);
    while received.try_recv().is_ok() {} // drain wake-ups of the steps above
    let mut commit = CommitRequest::new(claim_a.actor, claim_a.epoch, claim_a.message.id, TurnOutcome::Done);
    commit.sends = vec![
        OutgoingMessage::new(a.clone(), "Self", json!(3)),
        OutgoingMessage::new(addr("actor://pg/absent"), "T", json!(4)),
    ];
    assert!(matches!(writer.commit(commit).await, Ok(CommitResult::Rejected(_))));
    assert!(none_within(&mut received, short).await, "rejected commits do not notify");
    collector.abort();
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_wake_up_listener_reconnects_after_losing_its_connection() {
    let db = test_db!("s");
    let config = PostgresStoreConfig { listener_reconnect_delay: Duration::from_millis(100), ..db.config() };
    let listening = db.open_with(config.clone()).await;
    let writer = db.open().await;
    let (mut received, collector) = collect_wake_ups(&listening, template());
    wait_listening(&listening).await;

    // Kill the listener's backend, as a database restart or a network failure would. Only this test's listener: it
    // is the one that listens on this test's channel.
    let mut c = db.connect().await;
    let killed: Vec<bool> = sqlx::query_scalar(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
         WHERE application_name = 'cicada-listener' AND query LIKE '%' || $1 || '%'",
    )
    .bind(config.effective_channel())
    .fetch_all(&mut c)
    .await
    .unwrap();
    assert_eq!(killed, vec![true], "one listener backend terminated");
    // The reconnect itself signals a wake-up (notifications may have been missed meanwhile).
    assert!(some_within(&mut received, Duration::from_secs(10)).await, "reconnect wakes");
    wait_listening(&listening).await;

    let a = addr("actor://pg/a");
    writer.create(&a, &template(), None).await.unwrap();
    writer.enqueue(OutgoingMessage::new(a, "T", json!(1))).await.unwrap();
    assert!(some_within(&mut received, Duration::from_secs(2)).await, "notifications arrive after reconnecting");
    collector.abort();
    db.cleanup().await;
}
