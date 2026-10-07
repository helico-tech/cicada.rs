//! Data errors, fault injection inside a commit (S2), privileges and the schema migrations. Port of the Kotlin
//! `PostgresRobustnessTest`.

mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use cicada::address::{ActorAddress, UriTemplate};
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::{
    ActorCreate, CommitRequest, CommitResult, Delivery, EnqueueResult, MessageId, MessageStatus, NodeId, OutcomeStatus,
    OutgoingMessage, StateChange, Store, StoreError, TurnOutcome,
};
use cicada_store_postgres::PostgresStore;
use serde_json::json;
use sqlx::AssertSqlSafe;

fn template() -> UriTemplate {
    UriTemplate::parse("actor://pgr/{id}").unwrap()
}

fn types() -> BTreeSet<UriTemplate> {
    BTreeSet::from([template()])
}

fn a() -> ActorAddress {
    ActorAddress::parse("actor://pgr/a").unwrap()
}

fn node() -> NodeId {
    NodeId::new("n").unwrap()
}

const TTL: Duration = Duration::from_secs(30);

fn is_invalid<T: std::fmt::Debug>(result: Result<T, StoreError>) -> bool {
    matches!(result, Err(StoreError::Invalid(_)))
}

#[tokio::test(flavor = "multi_thread")]
async fn values_postgres_refuses_are_invalid_not_an_unavailable_store() {
    let db = test_db!("r");
    let store = db.open().await;
    store.create(&a(), &template(), None).await.unwrap();
    let nul = "a\u{0}b";
    assert!(is_invalid(store.enqueue(OutgoingMessage::new(a(), "T", json!(nul))).await), "NUL in a JSON string");
    let mut m = OutgoingMessage::new(a(), "T", json!(1));
    m.correlation_id = Some(nul.into());
    assert!(is_invalid(store.enqueue(m).await), "NUL in a text column");
    let mut m = OutgoingMessage::new(a(), "T", json!(1));
    m.id = MessageId::new(nul).unwrap();
    assert!(is_invalid(store.enqueue(m).await), "NUL in a key");
    let mut m = OutgoingMessage::new(a(), "T", json!(1));
    m.id = MessageId::new((0..600).map(|_| MessageId::random().to_string()).collect::<String>()).unwrap();
    assert!(is_invalid(store.enqueue(m).await), "an id too large for its index");
    assert!(store.pending(&a(), None, 100).await.unwrap().items.is_empty());

    // A commit with refused state applies nothing and keeps the lease, so it can be committed as failed.
    store.enqueue(OutgoingMessage::new(a(), "T", json!(1))).await.unwrap();
    let claim = store.claim_due(&types(), &node(), TTL, 1).await.unwrap().remove(0);
    let mut commit = CommitRequest::new(a(), claim.epoch, claim.message.id.clone(), TurnOutcome::Done);
    commit.state = StateChange::NewState(Some(json!({"s": nul})));
    assert!(is_invalid(store.commit(commit).await));
    assert!(matches!(store.status(&claim.message.id).await.unwrap(), Some(MessageStatus::Running { .. })));
    let failed = CommitRequest::new(a(), claim.epoch, claim.message.id, TurnOutcome::Failed("state refused".into()));
    assert!(matches!(store.commit(failed).await.unwrap(), CommitResult::Committed { .. }));
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s2_a_commit_failing_half_way_applies_nothing() {
    let db = test_db!("r");
    let store = db.open().await;
    store.create(&a(), &template(), None).await.unwrap();
    let target = ActorAddress::parse("actor://pgr/target").unwrap();
    store.create(&target, &template(), None).await.unwrap();
    let EnqueueResult::Enqueued(m) = store.enqueue(OutgoingMessage::new(a(), "T", json!(0))).await.unwrap() else {
        panic!("enqueued")
    };
    let s = &db.schema;
    db.sql(&format!(
        "CREATE FUNCTION \"{s}\".poison() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected fault'; END $$"
    ))
    .await;
    db.sql(&format!(
        "CREATE TRIGGER poison BEFORE INSERT ON \"{s}\".messages FOR EACH ROW WHEN (NEW.type = 'Poison') \
         EXECUTE FUNCTION \"{s}\".poison()"
    ))
    .await;
    let claim = store.claim_due(&types(), &node(), TTL, 1).await.unwrap().remove(0);
    let child = ActorAddress::parse("actor://pgr/child").unwrap();
    let mut commit = CommitRequest::new(a(), claim.epoch, claim.message.id, TurnOutcome::Done);
    commit.state = StateChange::NewState(Some(json!(1)));
    commit.creates = vec![ActorCreate { address: child.clone(), template: template(), state: None }];
    commit.sends = vec![
        OutgoingMessage::new(target.clone(), "Good", json!(1)),
        OutgoingMessage::new(target.clone(), "Poison", json!(2)),
    ];
    match store.commit(commit).await {
        Err(e @ StoreError::Unavailable(_)) => assert!(format!("{e:?}").contains("injected fault"), "{e:?}"),
        other => panic!("expected the injected fault, got {other:?}"),
    }
    assert!(store.outcome(&m.id).await.unwrap().is_none(), "no outcome");
    assert!(
        matches!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })),
        "the lease is still held"
    );
    assert!(store.pending(&target, None, 100).await.unwrap().items.is_empty(), "no send");
    assert!(store.get(&child).await.unwrap().is_none(), "no create");
    assert_eq!(store.get(&a()).await.unwrap().unwrap().version, 0, "state untouched");
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_can_start_as_a_user_without_create_privileges_once_the_schema_is_current() {
    let db = test_db!("r");
    db.open().await;
    db.close_stores().await;
    let s = &db.schema;
    let role = format!("dml_{s}");
    db.sql(&format!("CREATE ROLE \"{role}\" LOGIN PASSWORD 'dml'")).await;
    db.sql(&format!("GRANT USAGE ON SCHEMA \"{s}\" TO \"{role}\"")).await;
    db.sql(&format!("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA \"{s}\" TO \"{role}\"")).await;
    db.sql(&format!("GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA \"{s}\" TO \"{role}\"")).await;
    let result = async {
        let store = PostgresStore::connect_as(&db.url, Some(&role), Some("dml"), db.config()).await?;
        store.create(&a(), &template(), None).await?;
        let enqueued = store.enqueue(OutgoingMessage::new(a(), "T", json!(1))).await?;
        let claim = store.claim_due(&types(), &node(), TTL, 1).await?.remove(0);
        let committed = store.commit(CommitRequest::new(a(), claim.epoch, claim.message.id, TurnOutcome::Done)).await?;
        store.close().await;
        Ok::<_, StoreError>((enqueued, committed))
    }
    .await;
    db.sql(&format!("DROP OWNED BY \"{role}\"")).await;
    db.sql(&format!("DROP ROLE \"{role}\"")).await;
    let (enqueued, committed) = result.expect("a DML-only user can run the store");
    assert!(matches!(enqueued, EnqueueResult::Enqueued(_)));
    assert!(matches!(committed, CommitResult::Committed { .. }));
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_retry_policy_this_version_cannot_decode_reads_as_no_policy_and_does_not_break_claims() {
    let db = test_db!("r");
    let store = db.open().await;
    store.create(&a(), &template(), None).await.unwrap();
    let mut m = OutgoingMessage::new(a(), "T", json!(1));
    m.retry = Some(RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap());
    let EnqueueResult::Enqueued(bad) = store.enqueue(m).await.unwrap() else { panic!("enqueued") };
    db.sql(&format!(
        r#"UPDATE "{}".messages SET retry_policy = '{{"maxAttempts":3,"backoff":{{"kind":"jittered","delay":"PT1S"}}}}' WHERE id = '{}'"#,
        db.schema, bad.id
    ))
    .await;
    assert!(store.pending(&a(), None, 100).await.unwrap().items[0].retry.is_none());
    match store.status(&bad.id).await.unwrap() {
        Some(MessageStatus::Pending(message)) => assert!(message.retry.is_none()),
        other => panic!("expected pending, got {other:?}"),
    }
    let claim = store.claim_due(&types(), &node(), TTL, 1).await.unwrap().remove(0);
    assert_eq!(claim.message.id, bad.id);
    assert!(claim.message.retry.is_none());
    let committed =
        store.commit(CommitRequest::new(a(), claim.epoch, bad.id.clone(), TurnOutcome::Done)).await.unwrap();
    assert!(matches!(committed, CommitResult::Committed { .. }));
    let outcome = store.outcome(&bad.id).await.unwrap().unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert!(outcome.envelope.retry.is_none());
    db.cleanup().await;
}

async fn columns(db: &common::TestDb, table: &str) -> BTreeSet<String> {
    let mut c = db.connect().await;
    sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2",
    )
    .bind(&db.schema)
    .bind(table)
    .fetch_all(&mut c)
    .await
    .unwrap()
    .into_iter()
    .collect()
}

/// Removes what schema versions 5 (causation) and 6 (retention) added.
async fn drop_version_5_and_6(db: &common::TestDb) {
    let s = &db.schema;
    db.sql(&format!("ALTER TABLE \"{s}\".messages DROP COLUMN caused_by, DROP COLUMN retention_policy")).await;
    db.sql(&format!(
        "ALTER TABLE \"{s}\".outcomes DROP COLUMN caused_by, DROP COLUMN visible_until, DROP COLUMN keep_last, \
         DROP COLUMN keep_seq, DROP COLUMN stripped"
    ))
    .await;
}

async fn drop_schema(db: &common::TestDb) {
    db.sql(&format!("DROP SCHEMA IF EXISTS \"{}\" CASCADE", db.schema)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn version_1_to_4_schemas_are_migrated_and_keep_their_data() {
    let db = test_db!("r");
    let s = db.schema.clone();
    for version in [4, 3, 2, 1] {
        let done = MessageId::random();
        let store = db.open().await;
        store.create(&a(), &template(), None).await.unwrap();
        let mut m = OutgoingMessage::new(a(), "T", json!(1));
        m.id = done.clone();
        store.enqueue(m).await.unwrap();
        let claim = store.claim_due(&types(), &node(), TTL, 1).await.unwrap().remove(0);
        let committed =
            store.commit(CommitRequest::new(a(), claim.epoch, done.clone(), TurnOutcome::Done)).await.unwrap();
        assert!(matches!(committed, CommitResult::Committed { .. }));
        store.enqueue(OutgoingMessage::new(a(), "T", json!(2))).await.unwrap();
        db.close_stores().await;
        // Back to the schema of `version`, as written by the code of that time.
        drop_version_5_and_6(&db).await;
        if version < 4 {
            db.sql(&format!("ALTER TABLE \"{s}\".messages DROP COLUMN retry_policy")).await;
        }
        if version < 3 {
            db.sql(&format!("ALTER TABLE \"{s}\".messages DROP COLUMN failures, DROP COLUMN last_error")).await;
        }
        db.sql(&format!("UPDATE \"{s}\".outcomes SET retried_as = 'some-retry'")).await;
        if version == 1 {
            db.sql(&format!("ALTER TABLE \"{s}\".outcomes DROP COLUMN retried_as")).await;
        }
        db.sql(&format!("DELETE FROM \"{s}\".cicada_schema_version WHERE version > {version}")).await;

        let store = db.open().await;
        let messages = columns(&db, "messages").await;
        let outcomes = columns(&db, "outcomes").await;
        for column in ["failures", "last_error", "retry_policy", "caused_by", "retention_policy"] {
            assert!(messages.contains(column), "messages.{column} from version {version}");
        }
        for column in ["caused_by", "visible_until", "keep_last", "keep_seq", "stripped"] {
            assert!(outcomes.contains(column), "outcomes.{column} from version {version}");
        }
        // Additive: nodes still on version 2 write and read `retried_as` during a rolling deploy.
        assert!(outcomes.contains("retried_as"), "retried_as is kept, unused");
        let outcome = store.outcome(&done).await.unwrap().unwrap();
        assert_eq!(outcome.status, OutcomeStatus::Done);
        assert_eq!(outcome.envelope.failures, 0);
        assert!(outcome.envelope.retry.is_none());
        let claim = store.claim_due(&types(), &node(), TTL, 1).await.unwrap().remove(0);
        assert_eq!(claim.message.failures, 0);
        assert!(claim.message.last_error.is_none() && claim.message.retry.is_none());
        assert!(claim.message.caused_by.is_none(), "older messages have no known cause");
        let rescheduled = TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "boom".into() };
        let committed =
            store.commit(CommitRequest::new(a(), claim.epoch, claim.message.id.clone(), rescheduled)).await.unwrap();
        assert!(matches!(committed, CommitResult::Committed { .. }));
        let pending = store.pending(&a(), None, 100).await.unwrap().items.remove(0);
        assert_eq!(pending.failures, 1);
        assert_eq!(pending.last_error.as_deref(), Some("boom"));
        assert!(pending.seq > claim.message.seq);
        db.close_stores().await;
        drop_schema(&db).await;
    }
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn version_2_to_4_nodes_can_still_write_and_read_after_the_latest_migration() {
    let db = test_db!("r");
    let s = db.schema.clone();
    for from_version_2 in [false, true] {
        db.open().await.create(&a(), &template(), None).await.unwrap();
        db.close_stores().await;
        if from_version_2 {
            drop_version_5_and_6(&db).await;
            db.sql(&format!(
                "ALTER TABLE \"{s}\".messages DROP COLUMN failures, DROP COLUMN last_error, DROP COLUMN retry_policy"
            ))
            .await;
            db.sql(&format!("DELETE FROM \"{s}\".cicada_schema_version WHERE version > 2")).await;
            db.open().await;
            db.close_stores().await;
        }
        let envelope = format!(
            r#"{{"id":"m-v2","to":"{}","from":null,"type":"T","payload":1,"deliverAt":"1970-01-01T00:00:00Z","correlationId":null,"headers":{{}},"seq":5,"attempt":1}}"#,
            a()
        );
        // The version 2 statements in shape: outcome upsert naming `retried_as`, message insert without `failures` /
        // `last_error`.
        db.sql(&format!(
            "INSERT INTO \"{s}\".outcomes (message_id, actor_uri, status, error, finished_at, envelope, retried_as)
             VALUES ('m-v2', '{}', 'done', NULL, now(), '{envelope}'::jsonb, 'm-v2-retry')
             ON CONFLICT (message_id) DO UPDATE SET actor_uri = excluded.actor_uri, status = excluded.status,
                 error = excluded.error, finished_at = excluded.finished_at, envelope = excluded.envelope,
                 retried_as = excluded.retried_as",
            a()
        ))
        .await;
        db.sql(&format!(
            "INSERT INTO \"{s}\".messages (id, actor_uri, seq, type, payload, sender, correlation_id, headers, deliver_at, attempt)
             VALUES ('m-v2-pending', '{}', nextval('\"{s}\".message_seq'), 'T', '1', NULL, NULL, '{{}}', now(), 0)",
            a()
        ))
        .await;
        let store = db.open().await;
        assert_eq!(store.outcome(&MessageId::new("m-v2").unwrap()).await.unwrap().unwrap().status, OutcomeStatus::Done);
        let pending = store.pending(&a(), None, 100).await.unwrap().items.remove(0);
        assert_eq!(pending.id.as_str(), "m-v2-pending");
        assert_eq!(pending.failures, 0);
        assert!(pending.last_error.is_none());
        assert!(pending.retry.is_none(), "inserted without retry_policy: no policy");
        assert!(pending.caused_by.is_none(), "inserted without caused_by: no known cause");
        db.close_stores().await;
        drop_schema(&db).await;
    }
    db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transaction_past_the_socket_timeout_fails_as_unavailable_and_the_store_recovers() {
    let db = test_db!("r");
    let config =
        cicada_store_postgres::PostgresStoreConfig { socket_timeout: Duration::from_millis(500), ..db.config() };
    let store = db.open_with(config).await;
    store.create(&a(), &template(), None).await.unwrap();
    // Hold the instance's row lock from another connection: the enqueue waits for it beyond the timeout.
    let mut blocker = db.connect().await;
    sqlx::raw_sql(AssertSqlSafe(format!(
        "BEGIN; SELECT 1 FROM \"{}\".actors WHERE uri = '{}' FOR UPDATE",
        db.schema,
        a()
    )))
    .execute(&mut blocker)
    .await
    .unwrap();
    match store.enqueue(OutgoingMessage::new(a(), "T", json!(1))).await {
        Err(e @ StoreError::Unavailable(_)) => assert!(e.to_string().contains("socket timeout"), "{e}"),
        other => panic!("expected a timeout, got {other:?}"),
    }
    sqlx::raw_sql("ROLLBACK").execute(&mut blocker).await.unwrap();
    assert!(matches!(store.enqueue(OutgoingMessage::new(a(), "T", json!(2))).await, Ok(EnqueueResult::Enqueued(_))));
    assert_eq!(store.pending(&a(), None, 100).await.unwrap().items.len(), 1, "the timed-out enqueue applied nothing");
    db.cleanup().await;
}
