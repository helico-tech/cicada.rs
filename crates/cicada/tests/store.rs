//! A host opens a store through the core and hands it to its systems: SQLite, PostgreSQL (when `CICADA_PG_URL` is
//! set, also asked for TLS), two systems on clones of one store, and closing and opening a SQLite file again. Public
//! API only.

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::{MessageStatus, OutcomeStatus, PostgresOptions, SqliteOptions};
use cicada::{SendResult, Store};
use serde::{Deserialize, Serialize};
use sqlx::Connection;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Counter {
    id: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CounterMsg {
    Add { n: i64 },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct CounterState {
    total: i64,
}

struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        let CounterMsg::Add { n } = message;
        turn.state_mut().total += n;
        Ok(())
    }
}

fn counter() -> Counter {
    Counter { id: "c1".into() }
}

/// Creates the counter, sends it `Add(n)`, runs due turns until the message is done, and returns the total.
async fn add(system: &ActorSystem, n: i64) -> i64 {
    system.create(&counter(), &CounterState::default()).await.unwrap();
    let SendResult::Accepted(message) = system.send(&counter(), CounterMsg::Add { n }).await.unwrap() else {
        panic!("the send is accepted")
    };
    for _ in 0..200 {
        system.run_due_turns(10).await.unwrap();
        if let Some(MessageStatus::Finished(outcome)) = system.status(&message.id).await.unwrap() {
            assert_eq!(outcome.status, OutcomeStatus::Done);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    total(system).await
}

async fn total(system: &ActorSystem) -> i64 {
    let snapshot = system.get_actor(&counter().address().unwrap()).await.unwrap().expect("the counter exists");
    serde_json::from_value::<CounterState>(snapshot.state).unwrap().total
}

#[tokio::test]
async fn a_node_on_sqlite_keeps_the_state_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cicada.db");
    let store = Store::sqlite(&file, SqliteOptions::default()).await.unwrap();
    let system = ActorSystem::builder(store.clone()).host(CounterActor).build();

    assert_eq!(add(&system, 3).await, 3);

    system.stop().await;
    store.close().await;
    let mut c = sqlx::SqliteConnection::connect(&format!("sqlite://{}", file.display())).await.unwrap();
    let state: String =
        sqlx::query_scalar("SELECT state FROM actors WHERE address = 'counter/c1'").fetch_one(&mut c).await.unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&state).unwrap(), serde_json::json!({"total": 3}));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_on_postgres_handles_a_message() {
    let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
        return;
    };
    let schema = format!("store_{}_{}", std::process::id(), &MessageId::random().as_str()[..8]);
    let store = Store::postgres(&url, PostgresOptions::for_schema(&schema)).await.unwrap();
    let system = ActorSystem::builder(store.clone()).host(CounterActor).build();

    assert_eq!(add(&system, 4).await, 4);

    system.stop().await;
    store.close().await;
    let mut c = sqlx::PgConnection::connect(&url).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA \"{schema}\" CASCADE"))).execute(&mut c).await.unwrap();
}

/// The feature `postgres` builds the driver with TLS: asked for it, the plain test database fails the open because it
/// offers none, not because the build lacks it ("SQLx was built without TLS support enabled").
#[tokio::test(flavor = "multi_thread")]
async fn postgres_asked_for_tls_fails_on_a_server_without_it() {
    let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
        return;
    };
    let url = format!("{url}{}sslmode=require", if url.contains('?') { '&' } else { '?' });
    let schema = format!("tls_{}_{}", std::process::id(), &MessageId::random().as_str()[..8]);

    let Err(error) = Store::postgres(&url, PostgresOptions::for_schema(&schema)).await else {
        panic!("the test database offers no TLS, so a store that requires it does not open")
    };

    let message = error.to_string();
    assert!(message.contains("server does not support TLS"), "{message}");
}

#[tokio::test]
async fn two_systems_on_clones_of_one_store_share_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::sqlite(dir.path().join("cicada.db"), SqliteOptions::default()).await.unwrap();
    let host = ActorSystem::builder(store.clone()).host(CounterActor).build();
    host.initialize().await.unwrap(); // registers the type, so the other system can resolve it
    let sender = ActorSystem::builder(store.clone()).build();

    sender.create(&counter(), &CounterState::default()).await.unwrap();
    let SendResult::Accepted(message) = sender.send(&counter(), CounterMsg::Add { n: 5 }).await.unwrap() else {
        panic!("the send is accepted")
    };
    assert_eq!(host.run_due_turns(10).await.unwrap(), 1);

    assert!(matches!(sender.status(&message.id).await.unwrap(), Some(MessageStatus::Finished(_))));
    assert_eq!(total(&sender).await, 5);
    store.close().await;
}

#[tokio::test]
async fn a_closed_sqlite_store_opens_again_with_its_data() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cicada.db");
    let store = Store::sqlite(&file, SqliteOptions::default()).await.unwrap();
    let system = ActorSystem::builder(store.clone()).host(CounterActor).build();
    system.start().await.unwrap();
    assert_eq!(add(&system, 2).await, 2);
    system.stop().await;
    store.close().await;
    assert!(system.get_actor(&counter().address().unwrap()).await.unwrap_err().to_string().contains("unavailable"));

    let reopened = Store::sqlite(&file, SqliteOptions::default()).await.unwrap();
    let system = ActorSystem::builder(reopened.clone()).host(CounterActor).build();

    assert_eq!(total(&system).await, 2);
    reopened.close().await;
}
