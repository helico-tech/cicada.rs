//! The upgrade from schema version 1: a database built with the migrations of that version, rows written with SQL as
//! a node of that version left them, then opened by this version.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cicada::actor::contract_of;
use cicada::prelude::*;
use cicada::store::{DeleteResult, MessageStatus, OutcomeStatus, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{AssertSqlSafe, Connection, Row, SqliteConnection};

use crate::{SqliteOptions, SqliteStore, schema};

/// type `maintenance.cleanup`, key `_`: stateless in version 1, `NoState` now.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cleanup;

impl ActorRef for Cleanup {
    const TYPE: &'static str = "maintenance.cleanup";
    type Message = CleanupMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CleanupMsg {
    Run,
}

struct CleanupActor;

impl Actor for CleanupActor {
    type Ref = Cleanup;
    type State = NoState;
    type Message = CleanupMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _: &mut Turn<Self>, _: CleanupMsg) -> HandlerResult {
        Ok(())
    }
}

/// A database file of schema version `version` in a temporary directory.
struct Db {
    _dir: tempfile::TempDir,
    file: PathBuf,
}

impl Db {
    async fn at_version(version: i64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("actors.db");
        schema::migrate_to(&file, version).await.unwrap();
        Self { _dir: dir, file }
    }

    async fn raw(file: &Path) -> SqliteConnection {
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(file)).await.unwrap()
    }

    /// Runs `sql` with `binds` (text values, in order) on a raw connection.
    async fn exec(&self, sql: &str, binds: &[&str]) -> Result<(), String> {
        let mut c = Self::raw(&self.file).await;
        let mut query = sqlx::query(AssertSqlSafe(sql.to_owned()));
        for bind in binds {
            query = query.bind(*bind);
        }
        let result = query.execute(&mut c).await.map(drop).map_err(|e| e.to_string());
        c.close().await.unwrap();
        result
    }

    async fn texts(file: &Path, sql: &str) -> Vec<String> {
        let mut c = Self::raw(file).await;
        let rows = sqlx::query(AssertSqlSafe(sql.to_owned())).fetch_all(&mut c).await.unwrap();
        c.close().await.unwrap();
        rows.iter().map(|r| r.try_get::<String, _>(0).unwrap_or_else(|_| r.get::<i64, _>(0).to_string())).collect()
    }

    async fn open(&self) -> Arc<SqliteStore> {
        Arc::new(SqliteStore::open(&self.file, SqliteOptions::default()).await.unwrap())
    }
}

fn address(text: &str) -> ActorAddress {
    ActorAddress::parse(text).unwrap()
}

/// The contract of `CleanupActor` as version 1 registered it: no state schema.
fn stateless_contract() -> String {
    let mut contract = contract_of::<CleanupActor>().unwrap().to_json();
    contract["state"] = Value::Null;
    contract.to_string()
}

const REGISTER: &str = "INSERT INTO actor_types (name, contract, contract_hash) VALUES (?, ?, ?)";

#[tokio::test]
async fn instances_with_and_without_a_state() {
    let db = Db::at_version(1).await;
    assert_eq!(Db::texts(&db.file, "PRAGMA user_version").await, ["1"]);
    db.exec(REGISTER, &["x", r#"{"type":"x","key":["id"],"state":{"type":"object"},"messages":{}}"#, "sha256:x"])
        .await
        .unwrap();
    db.exec(
        "INSERT INTO actors (address, type, state, version, created_at) VALUES
            ('x/none', 'x', NULL, 0, 0), ('x/null', 'x', 'null', 0, 0), ('x/n', 'x', '{\"n\":1}', 4, 0)",
        &[],
    )
    .await
    .unwrap();

    let store = db.open().await;
    for (key, state, version) in [("none", json!({}), 0), ("null", json!({}), 0), ("n", json!({"n": 1}), 4)] {
        let record = store.get(&address(&format!("x/{key}"))).await.unwrap().unwrap();
        assert_eq!((record.state, record.version), (state, version), "x/{key}");
    }
    store.close().await;
    assert_eq!(Db::texts(&db.file, "PRAGMA user_version").await, ["2"]);
}

#[tokio::test]
async fn the_store_refuses_a_missing_state() {
    let db = Db::at_version(1).await;
    db.exec(REGISTER, &["x", r#"{"type":"x","key":["id"],"state":{},"messages":{}}"#, "sha256:x"]).await.unwrap();
    db.open().await.close().await;
    for sql in [
        "INSERT INTO actors (address, type, state, created_at) VALUES ('x/a', 'x', NULL, 0)",
        "INSERT INTO actors (address, type, created_at) VALUES ('x/b', 'x', 0)",
    ] {
        let error = db.exec(sql, &[]).await.unwrap_err();
        assert!(error.contains("NOT NULL constraint failed: actors.state"), "{sql}: {error}");
    }
}

#[tokio::test]
async fn a_type_with_state_keeps_its_registration() {
    let db = Db::at_version(1).await;
    let kept = r#"{"type":"kept","key":[],"state":{"type":"object"},"messages":{}}"#;
    db.exec(REGISTER, &["kept", kept, "sha256:kept"]).await.unwrap();
    db.exec(REGISTER, &["stateless", r#"{"type":"stateless","key":[],"state":null,"messages":{}}"#, "sha256:s"])
        .await
        .unwrap();
    db.exec(REGISTER, &["older", r#"{"type":"older","key":[],"messages":{}}"#, "sha256:o"]).await.unwrap();

    let store = db.open().await;
    let types = store.list_types().await.unwrap();
    let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["kept"], "the registrations without a state schema are removed");
    assert_eq!(types[0].contract_hash, "sha256:kept");
    assert_eq!(types[0].contract, serde_json::from_str::<Value>(kept).unwrap());
    store.close().await;
}

#[tokio::test]
async fn the_indexes_of_sqlite_survive() {
    let indexes = "SELECT name || ': ' || sql FROM sqlite_master
        WHERE type = 'index' AND tbl_name = 'actors' AND sql IS NOT NULL ORDER BY name";
    let db = Db::at_version(1).await;
    db.exec(REGISTER, &["x", r#"{"type":"x","key":["id"],"state":{},"messages":{}}"#, "sha256:x"]).await.unwrap();
    db.exec("INSERT INTO actors (address, type, state, next_due_at, created_at) VALUES ('x/a', 'x', NULL, 5, 0)", &[])
        .await
        .unwrap();
    db.open().await.close().await;

    let migrated = Db::texts(&db.file, indexes).await;
    assert_eq!(
        migrated,
        [
            "actors_due: CREATE INDEX actors_due ON actors (type, next_due_at) WHERE next_due_at IS NOT NULL AND paused = 0",
            "actors_running: CREATE INDEX actors_running ON actors (address) WHERE lease_owner IS NOT NULL",
        ]
    );
    let fresh = tempfile::tempdir().unwrap();
    let file = fresh.path().join("actors.db");
    SqliteStore::open(&file, SqliteOptions::default()).await.unwrap().close().await;
    assert_eq!(migrated, Db::texts(&file, indexes).await, "as a new database has");
    let columns = "SELECT name || ' ' || type || ' ' || \"notnull\" FROM pragma_table_info('actors') ORDER BY cid";
    assert_eq!(Db::texts(&db.file, columns).await, Db::texts(&file, columns).await);
}

#[tokio::test]
async fn a_stateless_type_is_upgraded_in_place() {
    let db = Db::at_version(1).await;
    db.exec(REGISTER, &["maintenance.cleanup", &stateless_contract(), "sha256:stateless"]).await.unwrap();
    db.exec(
        "INSERT INTO actors (address, type, state, next_due_at, created_at)
            VALUES ('maintenance.cleanup/_', 'maintenance.cleanup', NULL, 0, 0)",
        &[],
    )
    .await
    .unwrap();
    db.exec(
        "INSERT INTO messages (id, actor_address, seq, type, payload, headers, deliver_at)
            VALUES ('m1', 'maintenance.cleanup/_', 1, 'Run', '{}', '{}', 0)",
        &[],
    )
    .await
    .unwrap();
    db.exec("UPDATE sequences SET value = 1 WHERE name = 'seq'", &[]).await.unwrap();

    let store = db.open().await;
    let system = ActorSystem::builder(store.clone()).host(CleanupActor).build();
    system.initialize().await.expect("the node starts");
    let types = store.list_types().await.unwrap();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].contract_hash, contract_of::<CleanupActor>().unwrap().hash());
    assert_eq!(types[0].contract["state"]["title"], "NoState");
    assert_eq!(system.get_actor(&Cleanup.address().unwrap()).await.unwrap().unwrap().state, json!({}));

    assert_eq!(system.run_due_turns(10).await.unwrap(), 1);
    match system.status(&MessageId::new("m1").unwrap()).await.unwrap() {
        Some(MessageStatus::Finished(outcome)) => assert_eq!(outcome.status, OutcomeStatus::Done, "{outcome:?}"),
        other => panic!("not handled: {other:?}"),
    }
    system.stop().await;
    store.close().await;
}

#[tokio::test]
async fn a_type_that_no_node_hosts_any_more() {
    let db = Db::at_version(1).await;
    db.exec(REGISTER, &["gone", r#"{"type":"gone","key":["id"],"state":null,"messages":{}}"#, "sha256:gone"])
        .await
        .unwrap();
    db.exec("INSERT INTO actors (address, type, state, created_at) VALUES ('gone/a', 'gone', NULL, 0)", &[])
        .await
        .unwrap();

    let store = db.open().await;
    let system = ActorSystem::builder(store.clone()).host(CleanupActor).build();
    system.initialize().await.expect("the node starts");
    let names: Vec<String> = system.types().await.unwrap().iter().map(|t| t.name().to_string()).collect();
    assert_eq!(names, ["maintenance.cleanup"], "gone is not registered");
    let gone = address("gone/a");
    assert_eq!(system.get_actor(&gone).await.unwrap().unwrap().state, json!({}));
    assert!(matches!(system.delete_actor(&gone).await.unwrap(), DeleteResult::Deleted { .. }));
    assert_eq!(system.get_actor(&gone).await.unwrap(), None);
    system.stop().await;
    store.close().await;
}
