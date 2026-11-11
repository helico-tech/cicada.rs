//! The upgrades from schema versions 1 and 2: a database built with the migrations of that version, rows written with
//! SQL as a node of that version left them, then opened by this version.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::actor::contract_of;
use crate::prelude::*;
use crate::retry::{Backoff, RetryPolicy};
use crate::runtime::{RetryResult, SendResult};
use crate::store::{Backend, DeleteResult, MessageStatus, OutcomeStatus, OutgoingMessage, Store, StoreError, support};
use crate::time;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{AssertSqlSafe, Connection, Row, SqliteConnection};

use crate::store::sqlite::{SqliteOptions, SqliteStore, schema};

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

    async fn handle(&self, _: &mut Turn<Self>, _: CleanupMsg) -> HandlerResult {
        Ok(())
    }
}

/// type `notes`, a type with state; its names held a literal colon as `%3A` before version 3.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Notes {
    name: String,
}

impl ActorRef for Notes {
    const TYPE: &'static str = "notes";
    type Message = NotesMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum NotesMsg {
    Note { text: String },
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct NotesState {
    #[serde(default)]
    texts: Vec<String>,
}

struct NotesActor;

impl Actor for NotesActor {
    type Ref = Notes;
    type State = NotesState;

    async fn handle(&self, turn: &mut Turn<Self>, message: NotesMsg) -> HandlerResult {
        let NotesMsg::Note { text } = message;
        turn.state_mut().texts.push(text);
        Ok(())
    }
}

/// type `maker`: its turn creates a note.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Maker {
    name: String,
}

impl ActorRef for Maker {
    const TYPE: &'static str = "maker";
    type Message = MakerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum MakerMsg {
    MakeNote { name: String },
}

struct MakerActor;

impl Actor for MakerActor {
    type Ref = Maker;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: MakerMsg) -> HandlerResult {
        let MakerMsg::MakeNote { name } = message;
        turn.create(&Notes { name }, &NotesState::default())?;
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

/// The contract of `A` as version 2 registered it: with the fields of its key, so with another hash.
fn v2_contract<A: Actor>() -> String {
    let mut contract = contract_of::<A>().unwrap().to_json();
    contract["key"] = json!(["name"]);
    contract.to_string()
}

const REGISTER: &str = "INSERT INTO actor_types (name, contract, contract_hash) VALUES (?, ?, ?)";

/// The query of the upgrade note in `CHANGELOG.md` (0.6.0) that finds two keys that become one name.
const FIND_PAIRS: &str = "SELECT replace(address, '%3A', ':') AS address, count(*) AS actors FROM actors
    GROUP BY replace(address, '%3A', ':') HAVING count(*) > 1";

/// Store now, in microseconds, as text for a bind.
fn now_micros() -> String {
    time::to_micros(chrono::Utc::now()).to_string()
}

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
    assert_eq!(Db::texts(&db.file, "PRAGMA user_version").await, [schema::VERSION.to_string()]);
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
async fn every_registration_is_removed_and_every_actor_and_message_kept() {
    for version in [1, 2] {
        let db = Db::at_version(version).await;
        db.exec(REGISTER, &["notes", &v2_contract::<NotesActor>(), "sha256:v2"]).await.unwrap();
        db.exec(REGISTER, &["stateless", r#"{"type":"stateless","key":[],"state":null,"messages":{}}"#, "sha256:s"])
            .await
            .unwrap();
        db.exec("INSERT INTO actors (address, type, state, created_at) VALUES ('notes/a', 'notes', '{}', 0)", &[])
            .await
            .unwrap();
        db.exec(
            "INSERT INTO messages (id, actor_address, seq, type, payload, headers, deliver_at)
                VALUES ('m1', 'notes/a', 1, 'Note', '{\"text\":\"x\"}', '{}', 0)",
            &[],
        )
        .await
        .unwrap();

        let store = db.open().await;
        assert!(store.list_types().await.unwrap().is_empty(), "from version {version}");
        assert!(store.get(&address("notes/a")).await.unwrap().is_some(), "from version {version}");
        assert_eq!(store.pending(&address("notes/a"), None, 10).await.unwrap().items.len(), 1);
        store.close().await;
    }
}

/// Scenario: An old literal colon.
#[tokio::test]
async fn an_old_literal_colon() {
    let db = Db::at_version(2).await;
    db.exec(REGISTER, &["notes", &v2_contract::<NotesActor>(), "sha256:v2"]).await.unwrap();
    db.exec(
        "INSERT INTO actors (address, type, state, next_due_at, created_at) VALUES ('notes/a%3Ab', 'notes', '{}', 0, 0)",
        &[],
    )
    .await
    .unwrap();
    db.exec(
        "INSERT INTO messages (id, actor_address, seq, type, payload, headers, deliver_at)
            VALUES ('m2', 'notes/a%3Ab', 2, 'Note', '{\"text\":\"later\"}', '{}', 0)",
        &[],
    )
    .await
    .unwrap();
    let earlier = OutgoingMessage::new(address("notes/a:b"), "Note", json!({"text": "first"}))
        .into_message(time::from_micros(0), 1);
    let mut envelope = support::envelope_to_json(&earlier);
    envelope["to"] = json!("notes/a%3Ab");
    db.exec(
        "INSERT INTO outcomes (message_id, actor_address, status, finished_at, envelope) VALUES (?, 'notes/a%3Ab', 'done', ?, ?)",
        &[earlier.id.as_str(), &now_micros(), &envelope.to_string()],
    )
    .await
    .unwrap();
    db.exec("UPDATE sequences SET value = 2 WHERE name = 'seq'", &[]).await.unwrap();

    let store = db.open().await;
    let moved = address("notes/a:b");
    assert_eq!(store.get(&moved).await.unwrap().unwrap().address.as_str(), "notes/a:b");
    let mailbox = store.pending(&moved, None, 10).await.unwrap().items;
    assert_eq!(mailbox.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m2"]);
    let outcomes = store.outcomes(None, "notes/a:b", time::from_micros(0), None, 10).await.unwrap().items;
    assert_eq!(
        outcomes.iter().map(|o| (&o.message_id, o.actor.as_str())).collect::<Vec<_>>(),
        [(&earlier.id, "notes/a:b")]
    );
    assert_eq!(outcomes[0].envelope.to, moved);
    for column in
        ["SELECT address FROM actors", "SELECT actor_address FROM messages", "SELECT actor_address FROM outcomes"]
    {
        assert_eq!(Db::texts(&db.file, column).await, ["notes/a:b"], "{column}");
    }

    let system = ActorSystem::builder(Store::from_backend(store.clone())).host(NotesActor).build();
    system.initialize().await.expect("the node starts");
    assert_eq!(system.run_due_turns(10).await.unwrap(), 1);
    match system.status(&MessageId::new("m2").unwrap()).await.unwrap() {
        Some(MessageStatus::Finished(outcome)) => assert_eq!(outcome.status, OutcomeStatus::Done, "{outcome:?}"),
        other => panic!("not handled: {other:?}"),
    }
    assert_eq!(system.get_actor(&moved).await.unwrap().unwrap().state, json!({"texts": ["later"]}));
    system.stop().await;
    store.close().await;
}

/// Scenario: A sender with a literal colon.
#[tokio::test]
async fn a_sender_with_a_literal_colon() {
    let db = Db::at_version(2).await;
    db.exec("INSERT INTO actors (address, type, state, created_at) VALUES ('notes/x', 'notes', '{}', 0)", &[])
        .await
        .unwrap();
    db.exec(
        "INSERT INTO messages (id, actor_address, seq, type, payload, sender, headers, deliver_at)
            VALUES ('m1', 'notes/x', 1, 'Note', '{\"text\":\"x\"}', 'notes/a%3Ab', '{}', 0)",
        &[],
    )
    .await
    .unwrap();

    let store = db.open().await;
    match store.status(&MessageId::new("m1").unwrap()).await.unwrap() {
        Some(MessageStatus::Pending(message)) => assert_eq!(message.from, Some(address("notes/a:b"))),
        other => panic!("not pending: {other:?}"),
    }
    assert_eq!(Db::texts(&db.file, "SELECT sender FROM messages").await, ["notes/a%3Ab"], "the column is left alone");
    store.close().await;
}

/// Scenario: Two keys that become one name.
#[tokio::test]
async fn two_keys_that_become_one_name() {
    let db = Db::at_version(2).await;
    db.exec(
        "INSERT INTO actors (address, type, state, created_at)
            VALUES ('pairs/a%3Ab:c', 'pairs', '{}', 0), ('pairs/a:b%3Ac', 'pairs', '{}', 0)",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(Db::texts(&db.file, FIND_PAIRS).await, ["pairs/a:b:c"], "the query of the upgrade note finds the pair");

    let Err(error) = SqliteStore::open(&db.file, SqliteOptions::default()).await else { panic!("the open succeeded") };
    let StoreError::Invalid(message) = &error else { panic!("not an invalid store: {error:?}") };
    assert!(message.starts_with("The database cannot be migrated to schema version 3: "), "{message}");
    assert!(message.contains("UNIQUE constraint failed: actors.address"), "{message}");
    assert_eq!(Db::texts(&db.file, "PRAGMA user_version").await, ["2"]);
    assert_eq!(
        Db::texts(&db.file, "SELECT address FROM actors ORDER BY address").await,
        ["pairs/a%3Ab:c", "pairs/a:b%3Ac"]
    );
}

/// Scenario: A database that is locked while it is migrated.
#[tokio::test]
async fn a_database_that_is_locked_while_it_is_migrated_is_unavailable_and_migrated_later() {
    let db = Db::at_version(2).await;
    let mut writer = Db::raw(&db.file).await;
    sqlx::raw_sql("BEGIN IMMEDIATE").execute(&mut writer).await.unwrap();
    let options = || SqliteOptions { busy_timeout: std::time::Duration::from_millis(20), ..SqliteOptions::default() };
    let Err(error) = SqliteStore::open(&db.file, options()).await else { panic!("the open succeeded") };
    assert!(error.is_unavailable(), "{error:?}");
    sqlx::raw_sql("ROLLBACK").execute(&mut writer).await.unwrap();
    writer.close().await.unwrap();
    assert_eq!(Db::texts(&db.file, "PRAGMA user_version").await, ["2"]);

    SqliteStore::open(&db.file, options()).await.expect("the lock is gone").close().await;
    assert_eq!(Db::texts(&db.file, "PRAGMA user_version").await, [schema::VERSION.to_string()]);
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
    let system = ActorSystem::builder(Store::from_backend(store.clone())).host(CleanupActor).build();
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

/// Scenario: A type with state is upgraded in place.
#[tokio::test]
async fn a_type_with_state_is_upgraded_in_place() {
    let db = Db::at_version(2).await;
    db.exec(REGISTER, &["notes", &v2_contract::<NotesActor>(), "sha256:v2"]).await.unwrap();
    db.exec(
        "INSERT INTO actors (address, type, state, version, next_due_at, created_at)
            VALUES ('notes/n', 'notes', '{\"texts\":[\"kept\"]}', 3, 0, 0)",
        &[],
    )
    .await
    .unwrap();
    db.exec(
        "INSERT INTO messages (id, actor_address, seq, type, payload, headers, deliver_at)
            VALUES ('m1', 'notes/n', 1, 'Note', '{\"text\":\"new\"}', '{}', 0)",
        &[],
    )
    .await
    .unwrap();
    db.exec("UPDATE sequences SET value = 1 WHERE name = 'seq'", &[]).await.unwrap();

    let store = db.open().await;
    let system = ActorSystem::builder(Store::from_backend(store.clone())).host(NotesActor).build();
    system.initialize().await.expect("the node starts");
    let types = store.list_types().await.unwrap();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].contract_hash, contract_of::<NotesActor>().unwrap().hash());
    assert!(types[0].contract.get("key").is_none(), "{}", types[0].contract);
    let n = address("notes/n");
    assert_eq!(system.get_actor(&n).await.unwrap().unwrap().state, json!({"texts": ["kept"]}));

    assert_eq!(system.run_due_turns(10).await.unwrap(), 1);
    match system.status(&MessageId::new("m1").unwrap()).await.unwrap() {
        Some(MessageStatus::Finished(outcome)) => assert_eq!(outcome.status, OutcomeStatus::Done, "{outcome:?}"),
        other => panic!("not handled: {other:?}"),
    }
    assert_eq!(system.get_actor(&n).await.unwrap().unwrap().state, json!({"texts": ["kept", "new"]}));
    system.stop().await;
    store.close().await;
}

/// Scenario: A turn creates an actor of a type that is not registered yet.
#[tokio::test]
async fn a_turn_creates_an_actor_of_a_type_that_is_not_registered_yet() {
    let db = Db::at_version(2).await;
    db.exec(REGISTER, &["maker", &v2_contract::<MakerActor>(), "sha256:v2-maker"]).await.unwrap();
    db.exec(REGISTER, &["notes", &v2_contract::<NotesActor>(), "sha256:v2-notes"]).await.unwrap();
    db.exec(
        "INSERT INTO actors (address, type, state, next_due_at, created_at) VALUES ('maker/m', 'maker', '{}', 0, 0)",
        &[],
    )
    .await
    .unwrap();
    let retry = RetryPolicy::new(5, Backoff::fixed(std::time::Duration::from_secs(1)).unwrap()).unwrap();
    db.exec(
        "INSERT INTO messages (id, actor_address, seq, type, payload, headers, deliver_at, retry_policy)
            VALUES ('m1', 'maker/m', 1, 'MakeNote', '{\"name\":\"n\"}', '{}', 0, ?)",
        &[&retry.to_json().to_string()],
    )
    .await
    .unwrap();
    db.exec("UPDATE sequences SET value = 1 WHERE name = 'seq'", &[]).await.unwrap();

    // Only the maker is hosted: `notes` stays unregistered until a node that hosts it starts.
    let store = db.open().await;
    let system = ActorSystem::builder(Store::from_backend(store.clone())).host(MakerActor).build();
    system.initialize().await.expect("the node starts");
    assert_eq!(system.run_due_turns(10).await.unwrap(), 1);
    match system.status(&MessageId::new("m1").unwrap()).await.unwrap() {
        Some(MessageStatus::Finished(outcome)) => {
            assert_eq!(outcome.status, OutcomeStatus::Failed);
            assert!(outcome.error.as_deref().unwrap().contains("notes/n: its type is not registered"), "{outcome:?}");
            assert_eq!(outcome.envelope.attempt, 1, "no retry");
        }
        other => panic!("not failed: {other:?}"),
    }
    assert_eq!(store.get(&address("notes/n")).await.unwrap(), None);
    system.stop().await;
    store.close().await;
}

/// Scenario: A type that no node hosts any more.
#[tokio::test]
async fn a_type_that_no_node_hosts_any_more() {
    let db = Db::at_version(2).await;
    db.exec(REGISTER, &["gone", &v2_contract::<NotesActor>().replace("\"notes\"", "\"gone\""), "sha256:gone"])
        .await
        .unwrap();
    db.exec("INSERT INTO actors (address, type, state, created_at) VALUES ('gone/a', 'gone', '{\"n\":1}', 0)", &[])
        .await
        .unwrap();

    let store = db.open().await;
    let system = ActorSystem::builder(Store::from_backend(store.clone())).host(CleanupActor).build();
    system.initialize().await.expect("the node starts");
    let names: Vec<String> = system.types().await.unwrap().iter().map(|t| t.name().to_string()).collect();
    assert_eq!(names, ["maintenance.cleanup"], "gone is not registered");
    let gone = address("gone/a");
    assert_eq!(system.get_actor(&gone).await.unwrap().unwrap().state, json!({"n": 1}));
    assert!(matches!(system.delete_actor(&gone).await.unwrap(), DeleteResult::Deleted { .. }));
    assert_eq!(system.get_actor(&gone).await.unwrap(), None);
    system.stop().await;
    store.close().await;
}

/// Scenarios: The type is not registered; A retry while the type is not registered.
#[tokio::test]
async fn a_send_to_an_actor_whose_type_is_not_registered_answers_no_such_type_and_a_retry_is_accepted() {
    let db = Db::at_version(2).await;
    db.exec(REGISTER, &["notes", &v2_contract::<NotesActor>(), "sha256:v2-notes"]).await.unwrap();
    db.exec(
        "INSERT INTO actors (address, type, state, next_due_at, created_at) VALUES ('notes/a', 'notes', '{}', 0, 0)",
        &[],
    )
    .await
    .unwrap();
    db.exec(
        "INSERT INTO messages (id, actor_address, seq, type, payload, headers, deliver_at)
            VALUES ('m1', 'notes/a', 1, 'Nope', '{}', '{}', 0)",
        &[],
    )
    .await
    .unwrap();
    db.exec("UPDATE sequences SET value = 1 WHERE name = 'seq'", &[]).await.unwrap();

    // The upgrade removed the registration of `notes` and kept its actor.
    let store = db.open().await;
    let sender = ActorSystem::builder(Store::from_backend(store.clone())).build();
    let notes = address("notes/a");
    let note = || json!({"text": "x"});
    assert_eq!(
        sender.send_message(&notes, "Note", note(), SendOptions::new()).await.unwrap(),
        SendResult::NoSuchType,
        "the actor exists, its type is what is missing"
    );

    // A node that hosts `notes` registers it and fails `m1`; then the registration is gone again, as after the upgrade.
    let hosting = ActorSystem::builder(Store::from_backend(store.clone())).host(NotesActor).build();
    hosting.initialize().await.unwrap();
    assert_eq!(hosting.run_due_turns(10).await.unwrap(), 1);
    let m1 = MessageId::new("m1").unwrap();
    assert!(
        matches!(sender.status(&m1).await.unwrap(), Some(MessageStatus::Finished(o)) if o.status == OutcomeStatus::Failed)
    );
    hosting.stop().await;
    db.exec("DELETE FROM actor_types", &[]).await.unwrap();
    assert_eq!(sender.send_message(&notes, "Note", note(), SendOptions::new()).await.unwrap(), SendResult::NoSuchType);

    // A retry is no send: it is accepted and waits in the mailbox.
    let retried = match sender.retry_message(&m1).await.unwrap() {
        RetryResult::Accepted(m) => m,
        other => panic!("{other:?}"),
    };
    let pending = store.pending(&notes, None, 10).await.unwrap().items;
    assert_eq!(pending.iter().map(|m| &m.id).collect::<Vec<_>>(), [&retried.id]);
    sender.stop().await;
    store.close().await;
}
