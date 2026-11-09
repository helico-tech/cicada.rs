//! Data errors, fault injection inside a commit (S2), privileges.

mod common;

use std::collections::BTreeSet;
use std::str::FromStr;
use std::time::Duration;

use cicada::address::{ActorAddress, UriTemplate};
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::{
    ActorCreate, CommitRequest, CommitResult, EnqueueResult, MessageId, MessageStatus, NodeId, OutcomeStatus,
    OutgoingMessage, StateChange, Store, StoreError, TurnOutcome,
};
use cicada_store_conformance::outcome_record;
use cicada_store_postgres::{PgConnectOptions, PostgresStore};
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
    assert!(outcome_record(&*store, &m.id).await.is_none(), "no outcome");
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
        let options = PgConnectOptions::from_str(&db.url).expect("a PostgreSQL URL").username(&role).password("dml");
        let store = PostgresStore::connect_with(options, db.config()).await?;
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
    let outcome = outcome_record(&*store, &bad.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert!(outcome.envelope.retry.is_none());
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
