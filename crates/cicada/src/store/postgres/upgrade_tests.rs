//! The upgrades from schema versions 1 to 6: a schema built with the migrations of that version, rows written
//! with SQL as a node of that version left them, then opened by this version. And rows that only a hand can write: two
//! actors that name each other as parent, a watch whose watcher is gone. Skipped when `CICADA_PG_URL` is unset.

use std::collections::hash_map::RandomState;
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::panic::AssertUnwindSafe;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::actor::contract_of;
use crate::prelude::*;
use crate::store::{
    Backend, CommitRequest, CommitResult, DeleteResult, EnqueueResult, MessageStatus, NodeId, OutcomeRecord,
    OutcomeStatus, OutgoingMessage, SetWatchResult, Store, StoreError, TurnOutcome, support,
};
use crate::time;
use crate::watch::WatchTarget;
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{AssertSqlSafe, Connection, PgConnection};

use crate::store::postgres::schema::{self, quote};
use sqlx::postgres::PgConnectOptions;

use crate::store::postgres::{PostgresOptions, PostgresStore};

static SCHEMAS: AtomicU32 = AtomicU32::new(0);

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

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
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

/// The contract of `NotesActor` as version 2 registered it: with the fields of its key, so with another hash.
fn v2_notes_contract() -> String {
    let mut contract = contract_of::<NotesActor>().unwrap().to_json();
    contract["key"] = json!(["name"]);
    contract.to_string()
}

/// The query of the upgrade note in `CHANGELOG.md` (0.6.0) that finds two keys that become one name.
const FIND_PAIRS: &str = "SELECT replace(address, '%3A', ':') AS address, count(*) AS actors FROM {s}.actors
    GROUP BY replace(address, '%3A', ':') HAVING count(*) > 1";

/// A fresh schema of an older schema version in the test database.
struct Old {
    url: String,
    name: String,
}

impl Old {
    async fn connect(url: &str) -> PgConnection {
        PgConnection::connect_with(&PgConnectOptions::from_str(url).unwrap()).await.expect("test database reachable")
    }

    /// Runs `test` on a fresh schema of `version` and drops the schema afterwards, also when `test` fails.
    async fn run<F: Future<Output = ()>>(version: i32, test: impl FnOnce(Old) -> F) {
        let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
            eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
            return;
        };
        let _running = crate::tests::postgres::common::permit().await;
        let random = RandomState::new().build_hasher().finish() & 0xFF_FFFF;
        let n = SCHEMAS.fetch_add(1, Ordering::SeqCst);
        let name = format!("upgrade_{}_{n}_{random:06x}", std::process::id());
        let mut c = Self::connect(&url).await;
        schema::migrate_to(&mut c, &name, version).await.unwrap();
        let result = AssertUnwindSafe(test(Old { url: url.clone(), name: name.clone() })).catch_unwind().await;
        sqlx::raw_sql(AssertSqlSafe(format!("DROP SCHEMA {} CASCADE", quote(&name)))).execute(&mut c).await.unwrap();
        c.close().await.unwrap();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    /// Runs `sql` with `{s}` replaced by the quoted schema.
    async fn exec(&self, sql: &str) -> Result<(), String> {
        let mut c = Self::connect(&self.url).await;
        let result = sqlx::raw_sql(AssertSqlSafe(sql.replace("{s}", &quote(&self.name))))
            .execute(&mut c)
            .await
            .map(drop)
            .map_err(|e| e.to_string());
        c.close().await.unwrap();
        result
    }

    /// The first column of every row of `sql`, as text; `{s}` is replaced by the quoted schema.
    async fn texts(&self, sql: &str) -> Vec<String> {
        let mut c = Self::connect(&self.url).await;
        let sql = format!("SELECT x::text FROM ({}) AS q (x)", sql.replace("{s}", &quote(&self.name)));
        let rows = sqlx::query_scalar(AssertSqlSafe(sql)).fetch_all(&mut c).await.unwrap();
        c.close().await.unwrap();
        rows
    }

    async fn version(&self) -> i32 {
        let mut c = Self::connect(&self.url).await;
        let sql = format!("SELECT max(version) FROM {}.cicada_schema_version", quote(&self.name));
        let version = sqlx::query_scalar(AssertSqlSafe(sql)).fetch_one(&mut c).await.unwrap();
        c.close().await.unwrap();
        version
    }

    async fn try_open(&self) -> Result<PostgresStore, crate::store::StoreError> {
        let config = PostgresOptions { schema: self.name.clone(), max_pool_size: 2, ..Default::default() };
        PostgresStore::connect(&self.url, config).await
    }

    async fn open(&self) -> PostgresStore {
        self.try_open().await.expect("the store opens")
    }
}

fn address(text: &str) -> ActorAddress {
    ActorAddress::parse(text).unwrap()
}

#[tokio::test]
async fn instances_with_and_without_a_state() {
    Old::run(1, async |db| {
        assert_eq!(db.version().await, 1);
        db.exec(
            r#"INSERT INTO {s}.actor_types (name, contract, contract_hash)
                VALUES ('x', '{"type":"x","key":["id"],"state":{"type":"object"},"messages":{}}', 'sha256:x');
               INSERT INTO {s}.actors (address, type, state, version, created_at) VALUES
                ('x/none', 'x', NULL, 0, now()), ('x/null', 'x', 'null', 0, now()), ('x/n', 'x', '{"n":1}', 4, now())"#,
        )
        .await
        .unwrap();

        let store = db.open().await;
        for (key, state, version) in [("none", json!({}), 0), ("null", json!({}), 0), ("n", json!({"n": 1}), 4)] {
            let record = store.get(&address(&format!("x/{key}"))).await.unwrap().unwrap();
            assert_eq!((record.state, record.version), (state, version), "x/{key}");
        }
        store.close().await;
        assert_eq!(db.version().await, schema::VERSION);
    })
    .await;
}

#[tokio::test]
async fn the_store_refuses_a_missing_state() {
    Old::run(1, async |db| {
        db.exec(
            r#"INSERT INTO {s}.actor_types (name, contract, contract_hash)
                VALUES ('x', '{"type":"x","key":["id"],"state":{},"messages":{}}', 'sha256:x')"#,
        )
        .await
        .unwrap();
        db.open().await.close().await;
        for sql in [
            "INSERT INTO {s}.actors (address, type, state, created_at) VALUES ('x/a', 'x', NULL, now())",
            "INSERT INTO {s}.actors (address, type, created_at) VALUES ('x/b', 'x', now())",
        ] {
            let error = db.exec(sql).await.unwrap_err();
            assert!(error.contains(r#"null value in column "state""#), "{sql}: {error}");
        }
    })
    .await;
}

#[tokio::test]
async fn every_registration_is_removed_and_every_actor_and_message_kept() {
    for version in [1, 2] {
        Old::run(version, async |db| {
            db.exec(&format!(
                r#"INSERT INTO {{s}}.actor_types (name, contract, contract_hash) VALUES
                    ('notes', '{}', 'sha256:v2'),
                    ('stateless', '{{"type":"stateless","key":[],"state":null,"messages":{{}}}}', 'sha256:s');
                   INSERT INTO {{s}}.actors (address, type, state, created_at) VALUES ('notes/a', 'notes', '{{}}', now());
                   INSERT INTO {{s}}.messages (id, actor_address, seq, type, payload, headers, deliver_at)
                    VALUES ('m1', 'notes/a', 1, 'Note', '{{"text":"x"}}', '{{}}', now())"#,
                v2_notes_contract()
            ))
            .await
            .unwrap();

            let store = db.open().await;
            assert!(store.list_types().await.unwrap().is_empty(), "from version {version}");
            assert!(store.get(&address("notes/a")).await.unwrap().is_some(), "from version {version}");
            assert_eq!(store.pending(&address("notes/a"), None, 10).await.unwrap().items.len(), 1);
            store.close().await;
        })
        .await;
    }
}

/// Scenario: An old literal colon.
#[tokio::test]
async fn an_old_literal_colon() {
    Old::run(2, async |db| {
        let earlier = OutgoingMessage::new(address("notes/a:b"), "Note", json!({"text": "first"}))
            .into_message(time::from_micros(0), 1, false);
        let mut envelope = support::envelope_to_json(&earlier);
        envelope["to"] = json!("notes/a%3Ab");
        db.exec(&format!(
            r#"INSERT INTO {{s}}.actor_types (name, contract, contract_hash) VALUES ('notes', '{contract}', 'sha256:v2');
               INSERT INTO {{s}}.actors (address, type, state, next_due_at, created_at)
                VALUES ('notes/a%3Ab', 'notes', '{{}}', now(), now());
               INSERT INTO {{s}}.messages (id, actor_address, seq, type, payload, headers, deliver_at)
                VALUES ('m2', 'notes/a%3Ab', 2, 'Note', '{{"text":"later"}}', '{{}}', now());
               SELECT setval('{{s}}.message_seq', 2);
               INSERT INTO {{s}}.outcomes (message_id, actor_address, status, finished_at, envelope)
                VALUES ('{id}', 'notes/a%3Ab', 'done', now(), '{envelope}')"#,
            contract = v2_notes_contract(),
            id = earlier.id,
        ))
        .await
        .unwrap();

        let store = Arc::new(db.open().await);
        let moved = address("notes/a:b");
        assert_eq!(store.get(&moved).await.unwrap().unwrap().address.as_str(), "notes/a:b");
        let mailbox = store.pending(&moved, None, 10).await.unwrap().items;
        assert_eq!(mailbox.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m2"]);
        let outcomes = store.outcomes(None, "notes/a:b", time::from_micros(0), None, 10).await.unwrap().items;
        let found: Vec<_> = outcomes.iter().map(|o| (&o.message_id, o.actor.as_str())).collect();
        assert_eq!(found, [(&earlier.id, "notes/a:b")]);
        assert_eq!(outcomes[0].envelope.to, moved);
        for column in
            ["SELECT address FROM {s}.actors", "SELECT actor_address FROM {s}.messages", "SELECT actor_address FROM {s}.outcomes"]
        {
            assert_eq!(db.texts(column).await, ["notes/a:b"], "{column}");
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
    })
    .await;
}

/// Scenario: A sender with a literal colon.
#[tokio::test]
async fn a_sender_with_a_literal_colon() {
    Old::run(2, async |db| {
        db.exec(
            r#"INSERT INTO {s}.actors (address, type, state, created_at) VALUES ('notes/x', 'notes', '{}', now());
               INSERT INTO {s}.messages (id, actor_address, seq, type, payload, sender, headers, deliver_at)
                VALUES ('m1', 'notes/x', 1, 'Note', '{"text":"x"}', 'notes/a%3Ab', '{}', now())"#,
        )
        .await
        .unwrap();

        let store = db.open().await;
        match store.status(&MessageId::new("m1").unwrap()).await.unwrap() {
            Some(MessageStatus::Pending(message)) => assert_eq!(message.from, Some(address("notes/a:b"))),
            other => panic!("not pending: {other:?}"),
        }
        assert_eq!(db.texts("SELECT sender FROM {s}.messages").await, ["notes/a%3Ab"], "the column is left alone");
        store.close().await;
    })
    .await;
}

/// Scenario: Two keys that become one name.
#[tokio::test]
async fn two_keys_that_become_one_name() {
    Old::run(2, async |db| {
        db.exec(
            r#"INSERT INTO {s}.actors (address, type, state, created_at)
                VALUES ('pairs/a%3Ab:c', 'pairs', '{}', now()), ('pairs/a:b%3Ac', 'pairs', '{}', now())"#,
        )
        .await
        .unwrap();
        assert_eq!(db.texts(FIND_PAIRS).await, ["pairs/a:b:c"], "the query of the upgrade note finds the pair");

        let Err(error) = db.try_open().await else { panic!("the open succeeded") };
        let StoreError::Invalid(message) = &error else { panic!("not an invalid store: {error:?}") };
        assert!(message.starts_with("The database cannot be migrated to schema version 3: "), "{message}");
        assert!(message.contains("actors_pkey"), "{message}");
        assert_eq!(db.version().await, 2);
        assert_eq!(
            db.texts("SELECT address FROM {s}.actors ORDER BY address").await,
            ["pairs/a%3Ab:c", "pairs/a:b%3Ac"]
        );
    })
    .await;
}

/// Scenario: A database of schema version 3 is opened.
#[tokio::test]
async fn a_database_of_schema_version_3_is_opened_and_its_actors_have_no_parent() {
    Old::run(3, async |db| {
        db.exec(
            r#"INSERT INTO {s}.actors (address, type, state, next_due_at, created_at)
                VALUES ('notes/a', 'notes', '{"texts":["kept"]}', now(), now()), ('notes/b', 'notes', '{}', NULL, now());
               INSERT INTO {s}.messages (id, actor_address, seq, type, payload, headers, deliver_at)
                VALUES ('m1', 'notes/a', 1, 'Note', '{"text":"x"}', '{}', now());
               SELECT setval('{s}.message_seq', 1)"#,
        )
        .await
        .unwrap();

        let store = db.open().await;
        assert_eq!(db.version().await, schema::VERSION);
        for key in ["a", "b"] {
            let record = store.get(&address(&format!("notes/{key}"))).await.unwrap().expect("still there");
            assert_eq!(record.parent, None, "notes/{key}");
        }
        assert_eq!(store.get(&address("notes/a")).await.unwrap().unwrap().state, json!({"texts": ["kept"]}));
        assert_eq!(store.pending(&address("notes/a"), None, 10).await.unwrap().items.len(), 1);
        assert!(store.children(&address("notes/a"), None, 10).await.unwrap().items.is_empty());
        // Every actor is independent: deleting one leaves the other.
        let deleted = store.delete(&address("notes/a"), &MessageId::random()).await.unwrap();
        assert_eq!(deleted, DeleteResult::Deleted { purged: 1 });
        assert!(store.get(&address("notes/b")).await.unwrap().is_some());
        store.close().await;
    })
    .await;
}

/// Scenario: A database of schema version 4 is opened.
#[tokio::test]
async fn a_database_of_schema_version_4_is_opened_and_nothing_in_it_is_a_notification() {
    Old::run(4, async |db| {
        // An outcome as version 4 kept it: its envelope has no mark.
        db.exec(
            r#"INSERT INTO {s}.actors (address, type, state, next_due_at, created_at)
                VALUES ('notes/a', 'notes', '{}', now(), now());
               INSERT INTO {s}.messages (id, actor_address, seq, type, payload, headers, deliver_at)
                VALUES ('m1', 'notes/a', 1, 'Note', '{"text":"x"}', '{}', now());
               INSERT INTO {s}.outcomes (message_id, actor_address, status, finished_at, envelope)
                VALUES ('m0', 'notes/a', 'done', now(), '{"id": "m0", "to": "notes/a", "from": null, "type": "Note",
                    "payload": {}, "deliverAt": "2026-01-01T00:00:00Z", "correlationId": null, "headers": {},
                    "seq": 0, "attempt": 1}');
               SELECT setval('{s}.message_seq', 1)"#,
        )
        .await
        .unwrap();

        let store = db.open().await;
        assert_eq!(db.version().await, schema::VERSION);
        let pending = store.pending(&address("notes/a"), None, 10).await.unwrap().items;
        assert_eq!(pending.len(), 1, "the message is still there");
        assert!(!pending[0].notification);
        let outcome = expect_finished(store.status(&MessageId::new("m0").unwrap()).await.unwrap());
        assert!(!outcome.envelope.notification, "the outcome is still there and not a notification");
        assert!(store.watches(None, None, None, 10).await.unwrap().items.is_empty(), "no watch is stored");
        store.close().await;
    })
    .await;
}

/// Scenario: A database of schema version 5 is opened.
#[tokio::test]
async fn a_database_of_schema_version_5_is_opened_and_no_message_has_a_lost_claim() {
    Old::run(5, async |db| {
        // An outcome as version 5 kept it: its envelope has no count.
        db.exec(
            r#"INSERT INTO {s}.actors (address, type, state, next_due_at, created_at)
                VALUES ('notes/a', 'notes', '{}', now(), now());
               INSERT INTO {s}.messages (id, actor_address, seq, type, payload, headers, deliver_at, attempt)
                VALUES ('m1', 'notes/a', 1, 'Note', '{"text":"x"}', '{}', now(), 7);
               INSERT INTO {s}.outcomes (message_id, actor_address, status, finished_at, envelope)
                VALUES ('m0', 'notes/a', 'done', now(), '{"id": "m0", "to": "notes/a", "from": null, "type": "Note",
                    "payload": {}, "deliverAt": "2026-01-01T00:00:00Z", "correlationId": null, "headers": {},
                    "seq": 0, "attempt": 3, "notification": false}');
               SELECT setval('{s}.message_seq', 1)"#,
        )
        .await
        .unwrap();

        let store = db.open().await;
        assert_eq!(db.version().await, schema::VERSION);
        let pending = store.pending(&address("notes/a"), None, 10).await.unwrap().items;
        assert_eq!(pending.len(), 1, "the message is still there");
        assert_eq!((pending[0].attempt, pending[0].lost_claims), (7, 0));
        let outcome = expect_finished(store.status(&MessageId::new("m0").unwrap()).await.unwrap());
        assert_eq!((outcome.envelope.attempt, outcome.envelope.lost_claims), (3, 0), "the outcome reads as none");
        store.close().await;
    })
    .await;
}

/// The node statements of a node of schema version 6, as it prepared them: the read and the heartbeat, with their
/// column lists.
const V6_LIST_NODES: &str = "SELECT id, hosted, started_at, last_seen FROM nodes ORDER BY id";
const V6_HEARTBEAT: &str = "INSERT INTO nodes (id, hosted, started_at, last_seen) VALUES ($1, $2, $3, $3)
                 ON CONFLICT (id) DO UPDATE SET hosted = excluded.hosted, last_seen = excluded.last_seen";

/// Runs the node statements of version 6 on `c`: a heartbeat of `id`, then the read; the ids it read.
async fn v6_node_statements(c: &mut PgConnection, id: &str) -> Vec<String> {
    sqlx::query(V6_HEARTBEAT).bind(id).bind(json!(["notes"])).bind(chrono::Utc::now()).execute(&mut *c).await.unwrap();
    let rows = sqlx::query(V6_LIST_NODES).fetch_all(&mut *c).await.unwrap();
    rows.iter().map(|r| sqlx::Row::get::<String, _>(r, "id")).collect()
}

/// Scenarios: Upgrade an existing node table; A running old connection survives the additive migration.
#[tokio::test]
async fn a_database_of_schema_version_6_keeps_its_nodes_and_its_running_old_nodes() {
    Old::run(6, async |db| {
        db.exec(
            r#"INSERT INTO {s}.nodes (id, hosted, started_at, last_seen) VALUES ('kept', '["notes"]', now(), now())"#,
        )
        .await
        .unwrap();
        // A node of version 6 that keeps running: its connection has prepared its node statements.
        let mut old = Old::connect(&db.url).await;
        sqlx::raw_sql(AssertSqlSafe(format!("SET search_path TO {}", quote(&db.name))))
            .execute(&mut old)
            .await
            .unwrap();
        assert_eq!(v6_node_statements(&mut old, "old").await, ["kept", "old"]);

        let store = db.open().await;
        assert_eq!(db.version().await, schema::VERSION);
        let rows = store.list_nodes().await.unwrap();
        assert_eq!(rows.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["kept", "old"]);
        assert!(rows.iter().all(|n| (&n.hostname, &n.cicada_version, &n.label) == (&None, &None, &None)), "{rows:?}");
        let details = crate::store::NodeDetails {
            hostname: Some("new-host".into()),
            cicada_version: Some("0.19.0".into()),
            label: Some("new".into()),
        };
        store.heartbeat_node(&NodeId::new("old").unwrap(), &Default::default(), &details).await.unwrap();

        // The old connection's prepared statements still run: an update leaves the details of the new writer, an
        // insert has none.
        assert_eq!(v6_node_statements(&mut old, "old").await, ["kept", "old"]);
        assert_eq!(v6_node_statements(&mut old, "older").await, ["kept", "old", "older"]);
        old.close().await.unwrap();
        let rows = store.list_nodes().await.unwrap();
        let by_id = |id: &str| rows.iter().find(|n| n.id.as_str() == id).unwrap().clone();
        assert_eq!(by_id("old").hostname.as_deref(), Some("new-host"), "an old update keeps the details");
        assert_eq!(by_id("old").label.as_deref(), Some("new"));
        assert_eq!((by_id("older").hostname, by_id("older").label), (None, None), "an old insert has none");
        // A new heartbeat replaces them, clearing the absent ones.
        store.heartbeat_node(&NodeId::new("old").unwrap(), &Default::default(), &Default::default()).await.unwrap();
        let old_row = store.list_nodes().await.unwrap().into_iter().find(|n| n.id.as_str() == "old").unwrap();
        assert_eq!((old_row.hostname, old_row.cicada_version, old_row.label), (None, None, None));
        store.close().await;
    })
    .await;
}

fn expect_finished(status: Option<MessageStatus>) -> OutcomeRecord {
    match status {
        Some(MessageStatus::Finished(outcome)) => outcome,
        other => panic!("expected an outcome, got {other:?}"),
    }
}

/// Scenario: A watcher that is gone.
#[tokio::test]
async fn a_watcher_whose_row_was_removed_by_hand_is_skipped_and_its_watch_removed() {
    Old::run(schema::VERSION, async |db| {
        let store = db.open().await;
        store.register(contract_of::<NotesActor>().unwrap().to_type_def()).await.unwrap();
        let (x, w) = (address("notes/x"), address("notes/w"));
        for a in [&x, &w] {
            store.create(a, json!({})).await.unwrap();
        }
        assert_eq!(store.watch(&WatchTarget::Actor(x.clone()), &w).await.unwrap(), SetWatchResult::Created);
        db.exec("DELETE FROM {s}.actors WHERE address = 'notes/w'").await.unwrap();

        let sent = match store.enqueue(OutgoingMessage::new(x.clone(), "Note", json!({"text": "t"}))).await.unwrap() {
            EnqueueResult::Enqueued(m) => m,
            other => panic!("{other:?}"),
        };
        let types = std::collections::BTreeSet::from([x.type_name().clone()]);
        let node = NodeId::new("n").unwrap();
        let claim = store.claim_due(&types, &node, std::time::Duration::from_secs(30), 10).await.unwrap().remove(0);
        let failure =
            CommitRequest::new(x.clone(), claim.epoch, claim.message.id.clone(), TurnOutcome::Failed("boom".into()));
        assert!(matches!(store.commit(failure).await.unwrap(), CommitResult::Committed { .. }), "the commit succeeds");
        assert_eq!(expect_finished(store.status(&sent.id).await.unwrap()).status, OutcomeStatus::Failed);
        assert!(store.watches(None, None, None, 10).await.unwrap().items.is_empty(), "the watch is removed");
        assert_eq!(db.texts("SELECT count(*) FROM {s}.messages").await, ["0"], "nothing was written");
        store.close().await;
    })
    .await;
}

/// Scenario: Two rows that name each other.
#[tokio::test]
async fn a_delete_ends_on_two_rows_that_name_each_other() {
    Old::run(schema::VERSION, async |db| {
        db.exec(
            r#"INSERT INTO {s}.actors (address, type, state, parent, created_at)
                VALUES ('notes/a', 'notes', '{}', 'notes/b', now()), ('notes/b', 'notes', '{}', 'notes/a', now()),
                       ('notes/c', 'notes', '{}', 'notes/b', now()), ('notes/other', 'notes', '{}', NULL, now());
               INSERT INTO {s}.messages (id, actor_address, seq, type, payload, headers, deliver_at)
                VALUES ('m1', 'notes/b', 1, 'Note', '{}', '{}', now()), ('m2', 'notes/c', 2, 'Note', '{}', '{}', now());
               SELECT setval('{s}.message_seq', 2)"#,
        )
        .await
        .unwrap();

        let store = db.open().await;
        assert_eq!(
            store.delete(&address("notes/a"), &MessageId::random()).await.unwrap(),
            DeleteResult::Deleted { purged: 2 }
        );
        assert_eq!(db.texts("SELECT address FROM {s}.actors ORDER BY address").await, ["notes/other"]);
        store.close().await;
    })
    .await;
}
