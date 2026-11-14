//! The change feed on PostgreSQL (ADR 0034): one `@` payload per committing transaction on the store's channel, read by
//! the listener of every upgraded store on the schema, and a hub that is up only while the listener has its `LISTEN`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::json;
use sqlx::AssertSqlSafe;
use sqlx::postgres::PgListener;
use tokio::time::timeout;

use super::common::{TestDb, eventually, test_db};
use crate::address::{ActorAddress, TypeName};
use crate::change::{ActorChange, ActorChanges, HubItem};
use crate::store::postgres::PostgresStore;
use crate::store::{
    ActorCreate, Backend, CommitRequest, CommitResult, MessageId, NodeId, OutgoingMessage, StateChange, StoreError,
    TurnOutcome,
};
use crate::tests::conformance::type_def;

fn kind() -> TypeName {
    TypeName::parse("pgc").unwrap()
}

fn addr(key: &str) -> ActorAddress {
    ActorAddress::of(&kind(), key).unwrap()
}

/// A store on `db` with the type registered.
async fn open(db: &TestDb) -> Arc<PostgresStore> {
    let store = db.open().await;
    store.register(type_def(&kind(), "test")).await.unwrap();
    store
}

/// A stream of `store`'s hints under `prefix` whose first `Resync` (after the listener's `LISTEN`) is taken.
async fn subscribe(store: &PostgresStore, prefix: &str) -> ActorChanges {
    let mut changes = ActorChanges::new(store.changes(), prefix);
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync));
    assert!(store.is_listening(), "the first resync comes once LISTEN succeeded");
    changes
}

async fn next(changes: &mut ActorChanges) -> Option<ActorChange> {
    timeout(Duration::from_secs(20), changes.next()).await.expect("an item in time")
}

/// The items up to the hint of `marker`, which the test writes last.
async fn until(changes: &mut ActorChanges, marker: &ActorAddress) -> Vec<ActorChange> {
    let mut seen = Vec::new();
    loop {
        match next(changes).await {
            Some(ActorChange::Changed { address, .. }) if &address == marker => return seen,
            Some(change) => seen.push(change),
            None => panic!("the stream ended after {seen:?}"),
        }
    }
}

fn hints(keys: &[&str]) -> Vec<ActorChange> {
    keys.iter().map(|k| ActorChange::changed(addr(k))).collect()
}

/// Sends `payload` on the store's channel from a plain connection, as another program could.
async fn notify(db: &TestDb, payload: &str) {
    let mut c = db.connect().await;
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(db.config().effective_channel())
        .bind(payload)
        .execute(&mut c)
        .await
        .unwrap();
}

/// Creates `count` children of `parent` through turns of `parent`, at most 50 in each.
async fn spawn_children(store: &PostgresStore, parent: &ActorAddress, count: usize) {
    let node = NodeId::new("n").unwrap();
    for chunk in (0..count).collect::<Vec<_>>().chunks(50) {
        store.enqueue(OutgoingMessage::new(parent.clone(), "Spawn", json!({}))).await.unwrap();
        let types = BTreeSet::from([kind()]);
        let claim = store.claim_due(&types, &node, Duration::from_secs(60), 1).await.unwrap().remove(0);
        let mut commit = CommitRequest::new(parent.clone(), claim.epoch, claim.message.id, TurnOutcome::Done);
        commit.creates = chunk
            .iter()
            .map(|i| ActorCreate { address: addr(&format!("{}-{i:03}", parent.key())), state: json!({}), child: true })
            .collect();
        assert!(matches!(store.commit(commit).await.unwrap(), CommitResult::Committed { .. }));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_commit_on_one_store_reaches_the_feeds_of_every_store_on_the_schema_and_a_rollback_reaches_none() {
    let db = test_db!("c");
    let writer = open(&db).await;
    let reader = open(&db).await;
    let mut theirs = subscribe(&reader, "pgc/").await;
    let mut own = subscribe(&writer, "").await;
    writer.create(&addr("a"), json!({})).await.unwrap();
    writer.create(&addr("a"), json!({})).await.unwrap(); // exists: nothing
    writer.set_paused(&addr("a"), true).await.unwrap();
    // A commit that is rejected rolls back: neither its creates nor its wake-ups are sent.
    let node = NodeId::new("n").unwrap();
    writer.enqueue(OutgoingMessage::new(addr("a"), "T", json!({}))).await.unwrap();
    writer.set_paused(&addr("a"), false).await.unwrap();
    let claim = writer.claim_due(&BTreeSet::from([kind()]), &node, Duration::from_secs(60), 1).await.unwrap().remove(0);
    let mut commit = CommitRequest::new(addr("a"), claim.epoch, claim.message.id.clone(), TurnOutcome::Done);
    commit.creates = vec![ActorCreate { address: addr("never"), state: json!({}), child: false }];
    commit.sends = vec![OutgoingMessage::new(addr("absent"), "T", json!({}))];
    commit.state = StateChange::NewState(json!({"n": 1}));
    assert!(matches!(writer.commit(commit).await.unwrap(), CommitResult::Rejected(_)));
    let mut commit = CommitRequest::new(addr("a"), claim.epoch, claim.message.id, TurnOutcome::Done);
    commit.state = StateChange::NewState(json!({"n": 1}));
    assert!(matches!(writer.commit(commit).await.unwrap(), CommitResult::Committed { .. }));
    writer.delete(&addr("a"), &MessageId::random()).await.unwrap();
    writer.create(&addr("marker"), json!({})).await.unwrap();
    let expected = hints(&["a", "a", "a", "a", "a"]); // create, pause, resume, new state, delete
    assert_eq!(until(&mut theirs, &addr("marker")).await, expected);
    assert_eq!(until(&mut own, &addr("marker")).await, expected);
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn feed_payloads_are_validated_as_a_whole_and_other_payloads_are_wake_ups() {
    let db = test_db!("c");
    let store = open(&db).await;
    let mut wake_ups = store.wake_ups(&BTreeSet::from([kind()])).unwrap();
    let mut changes = subscribe(&store, "").await;
    assert_eq!(timeout(Duration::from_secs(10), wake_ups.next()).await.unwrap(), Some(()), "LISTEN wakes");
    notify(&db, "@pgc/x pgc/y").await;
    assert_eq!(next(&mut changes).await, Some(ActorChange::changed(addr("x"))));
    assert_eq!(next(&mut changes).await, Some(ActorChange::changed(addr("y"))));
    for malformed in ["@pgc/x bad", "@ pgc/x", "@pgc/x  pgc/y", "@pgc/x ", "@pgc/%7e", "@v2:pgc/x", "@"] {
        notify(&db, malformed).await;
        assert_eq!(next(&mut changes).await, Some(ActorChange::Resync), "{malformed:?}");
    }
    notify(&db, "pgc").await; // a wake-up of the type: not the feed's
    notify(&db, "").await;
    notify(&db, "!cicada-listener-probe").await;
    store.create(&addr("marker"), json!({})).await.unwrap();
    assert_eq!(until(&mut changes, &addr("marker")).await, vec![]);
    assert_eq!(timeout(Duration::from_secs(10), wake_ups.next()).await.unwrap(), Some(()), "the type's wake-up");
    // The create's `@` payload woke nobody: the next wake-up is this enqueue's.
    store.enqueue(OutgoingMessage::new(addr("marker"), "T", json!({}))).await.unwrap();
    assert_eq!(timeout(Duration::from_secs(10), wake_ups.next()).await.unwrap(), Some(()));
    assert!(timeout(Duration::from_millis(300), wake_ups.next()).await.is_err(), "nothing more");
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hundred_removed_actors_are_named_and_a_hundred_and_one_are_one_resync() {
    let db = test_db!("c");
    let store = open(&db).await;
    let other = open(&db).await;
    let mut changes = subscribe(&other, "").await;
    for (root, children) in [("p", 99), ("q", 100)] {
        store.create(&addr(root), json!({})).await.unwrap();
        spawn_children(&store, &addr(root), children).await;
    }
    store.create(&addr("marker-1"), json!({})).await.unwrap();
    assert_eq!(until(&mut changes, &addr("marker-1")).await.len(), 2 + 99 + 100, "each create");
    store.delete(&addr("p"), &MessageId::random()).await.unwrap(); // 100 actors
    store.delete(&addr("q"), &MessageId::random()).await.unwrap(); // 101 actors
    for _ in 0..100 {
        assert!(matches!(next(&mut changes).await, Some(ActorChange::Changed { .. })));
    }
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync));
    // Hints after a resync are read against a fresh receiver: write the marker now.
    store.create(&addr("marker-2"), json!({})).await.unwrap();
    assert_eq!(until(&mut changes, &addr("marker-2")).await, vec![]);
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_re_run_transaction_starts_with_an_empty_collection() {
    let db = test_db!("c");
    let store = open(&db).await;
    let mut changes = subscribe(&store, "").await;
    store.create(&addr("p"), json!({})).await.unwrap();
    spawn_children(&store, &addr("p"), 3).await;
    store.create(&addr("marker-1"), json!({})).await.unwrap();
    until(&mut changes, &addr("marker-1")).await;
    // The first attempt removes the whole tree, `p-001` included, and rolls back. Before the re-run, `p-001` leaves
    // the tree, so the attempt that commits never touches it: a collection that survived the re-run would report it.
    let s = db.schema.clone();
    let mut c = db.connect().await;
    *store.rerun_next_commit.lock().unwrap() = Some(Box::pin(async move {
        sqlx::raw_sql(AssertSqlSafe(format!("UPDATE \"{s}\".actors SET parent = NULL WHERE address = 'pgc/p-001'")))
            .execute(&mut c)
            .await
            .unwrap();
    }));
    store.delete(&addr("p"), &MessageId::random()).await.unwrap();
    assert!(store.rerun_next_commit.lock().unwrap().is_none(), "the fault ran");
    assert!(store.get(&addr("p-001")).await.unwrap().is_some(), "the re-run left it");
    store.create(&addr("marker-2"), json!({})).await.unwrap();
    assert_eq!(until(&mut changes, &addr("marker-2")).await, hints(&["p", "p-000", "p-002"]));
    db.cleanup().await;
}

/// Installs a trigger that raises `sqlstate` for the actor `key`: at the statement, or deferred to `COMMIT`.
async fn raise_on_insert(db: &TestDb, name: &str, key: &str, sqlstate: &str, deferred: bool) {
    let s = &db.schema;
    db.sql(&format!(
        "CREATE FUNCTION \"{s}\".{name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
         RAISE EXCEPTION 'injected {sqlstate}' USING ERRCODE = '{sqlstate}'; END $$"
    ))
    .await;
    let (kind, timing) =
        if deferred { ("CONSTRAINT TRIGGER", "DEFERRABLE INITIALLY DEFERRED") } else { ("TRIGGER", "") };
    db.sql(&format!(
        "CREATE {kind} {name} AFTER INSERT ON \"{s}\".actors {timing} FOR EACH ROW \
         WHEN (NEW.address = 'pgc/{key}') EXECUTE FUNCTION \"{s}\".{name}()"
    ))
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_commit_that_fails_with_54000_is_unavailable_and_a_statement_that_does_stays_invalid() {
    let db = test_db!("c");
    let store = open(&db).await;
    let mut changes = subscribe(&store, "").await;
    raise_on_insert(&db, "full_queue", "at-commit", "54000", true).await;
    raise_on_insert(&db, "too_long", "at-statement", "54000", false).await;
    match store.create(&addr("at-commit"), json!({})).await {
        Err(e @ StoreError::Unavailable(_)) => assert!(format!("{e:?}").contains("injected 54000"), "{e:?}"),
        other => panic!("expected unavailable, got {other:?}"),
    }
    match store.create(&addr("at-statement"), json!({})).await {
        Err(StoreError::Invalid(message)) => assert!(message.contains("injected 54000"), "{message}"),
        other => panic!("expected invalid, got {other:?}"),
    }
    assert_eq!(store.get(&addr("at-commit")).await.unwrap(), None);
    store.create(&addr("marker"), json!({})).await.unwrap();
    assert_eq!(until(&mut changes, &addr("marker")).await, vec![], "no partial effect, no hint");
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_caller_s_commit_that_lands_is_reported() {
    let db = test_db!("c");
    let store = open(&db).await;
    let reader = open(&db).await;
    let mut changes = subscribe(&reader, "").await;
    let s = &db.schema;
    db.sql(&format!(
        "CREATE FUNCTION \"{s}\".slow() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(1); \
         RETURN NULL; END $$"
    ))
    .await;
    db.sql(&format!(
        "CREATE CONSTRAINT TRIGGER slow AFTER INSERT ON \"{s}\".actors DEFERRABLE INITIALLY DEFERRED FOR EACH ROW \
         WHEN (NEW.address = 'pgc/slow') EXECUTE FUNCTION \"{s}\".slow()"
    ))
    .await;
    let task = {
        let store = store.clone();
        tokio::spawn(async move { store.create(&addr("slow"), json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(400)).await; // inside the slow COMMIT
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    eventually(Duration::from_secs(10), "the commit lands", || async {
        reader.get(&addr("slow")).await.unwrap().is_some()
    })
    .await;
    assert_eq!(next(&mut changes).await, Some(ActorChange::changed(addr("slow"))), "committed, so notified");
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_listener_connection_resyncs_once_listen_succeeded_again_and_hints_follow() {
    let db = test_db!("c");
    let store = open(&db).await;
    let mut changes = subscribe(&store, "").await;
    let mut c = db.connect().await;
    let killed: Vec<bool> = sqlx::query_scalar(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
         WHERE application_name = 'cicada-listener' AND query LIKE '%' || $1 || '%'",
    )
    .bind(db.config().effective_channel())
    .fetch_all(&mut c)
    .await
    .unwrap();
    assert_eq!(killed, vec![true], "one listener backend terminated");
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync), "after the reconnect");
    assert!(store.is_listening());
    store.create(&addr("after"), json!({})).await.unwrap();
    assert_eq!(next(&mut changes).await, Some(ActorChange::changed(addr("after"))));
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_flood_of_wake_ups_makes_a_slow_reader_resync_and_hints_go_on() {
    let db = test_db!("c");
    let store = open(&db).await;
    let mut changes = subscribe(&store, "").await;
    let (mut hub, _) = store.changes().subscribe().unwrap();
    let mut c = db.connect().await;
    sqlx::query("SELECT pg_notify($1, 'wake' || g) FROM generate_series(1, 1500) AS g")
        .bind(db.config().effective_channel())
        .execute(&mut c)
        .await
        .unwrap();
    store.create(&addr("marker"), json!({})).await.unwrap();
    // The listener fed all of it to the hub once the marker's batch is there.
    timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(HubItem::Changes(batch)) = hub.recv().await
                && batch.contains(&addr("marker"))
            {
                return;
            }
        }
    })
    .await
    .expect("the marker's batch");
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync), "lagged");
    store.create(&addr("after"), json!({})).await.unwrap();
    assert_eq!(until(&mut changes, &addr("after")).await, vec![]);
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_the_store_ends_its_streams_and_a_later_subscription_and_never_restarts_the_listener() {
    let db = test_db!("c");
    let store = open(&db).await;
    let mut changes = subscribe(&store, "").await;
    let channel = db.config().effective_channel();
    // The probe's connection: the sender of a probe notification, seen by an observer outside the store's pools.
    let mut observer = PgListener::connect(&db.url).await.unwrap();
    observer.listen(&channel).await.unwrap();
    let probe = timeout(Duration::from_secs(15), async {
        loop {
            let notification = observer.recv().await.unwrap();
            if notification.payload() == "!cicada-listener-probe" {
                return notification.process_id() as i32;
            }
        }
    })
    .await
    .expect("a probe in time");
    drop(observer);
    let mut c = db.connect().await;
    let listener: Vec<i32> = sqlx::query_scalar(
        "SELECT pid FROM pg_stat_activity WHERE application_name = 'cicada-listener' AND query LIKE '%' || $1 || '%'",
    )
    .bind(&channel)
    .fetch_all(&mut c)
    .await
    .unwrap();
    assert_eq!(listener.len(), 1, "one listener backend");
    let listener = listener[0];
    assert_ne!(listener, probe, "the probe has its own connection");
    store.create(&addr("queued"), json!({})).await.unwrap();
    store.close().await;
    assert_eq!(next(&mut changes).await, None);
    let mut later = ActorChanges::new(store.changes(), "");
    assert_eq!(next(&mut later).await, None);
    assert!(!store.is_listening());
    // By the saved process ids: the probe's query has parameters, and the listener's last one may be its UNLISTEN.
    drop(c);
    eventually(Duration::from_secs(10), "neither the listener's nor the probe's connection is left", || async {
        let mut c = db.connect().await;
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE pid = ANY($1)")
            .bind(vec![listener, probe])
            .fetch_one(&mut c)
            .await
            .unwrap();
        left == 0
    })
    .await;
    assert!(matches!(store.create(&addr("x"), json!({})).await, Err(StoreError::Unavailable(_))));
    db.cleanup().await;
}
