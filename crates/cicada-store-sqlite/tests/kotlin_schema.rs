//! The schema is the Kotlin implementation's (ADR 0004). These checks use the Kotlin repository and the `sqlite3` CLI
//! when they are available and are skipped (with a note on stderr) when they are not:
//!
//! - the migration statements equal those of the Kotlin `Schema.kt` after `trimIndent()`, text for text (found next to
//!   this repository as `cicada/`, or at `CICADA_KOTLIN_DIR`);
//! - the stock `sqlite3` CLI, fed the Kotlin statements, builds a database whose `sqlite_master` equals that of a file
//!   created by [`SqliteStore`];
//! - a database file written by the Kotlin store (`CICADA_KOTLIN_DB`, copied first, never changed) opens, migrates and
//!   reads with the Rust store.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cicada::store::{Store, SystemClock};
use cicada::time::{self, DEFAULT_RETENTION};
use cicada_store_sqlite::{SqliteOptions, SqliteStore, schema};
use cicada_test::VirtualClock;
use common::TempDb;

const SCHEMA_KT: &str = "actors-store-sqlite/src/main/kotlin/nl/helico/cicada/store/sqlite/Schema.kt";

/// The Kotlin repository: `CICADA_KOTLIN_DIR`, or a directory `cicada` next to an ancestor of this crate.
fn kotlin_repo() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CICADA_KOTLIN_DIR") {
        return Some(PathBuf::from(dir));
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|a| a.join("cicada")).find(|c| c.join(SCHEMA_KT).is_file())
}

/// Kotlin's `String.trimIndent()`: drops a blank first and last line and the common indent of the non-blank lines.
fn trim_indent(text: &str) -> String {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let indent =
        lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    lines.iter().map(|l| if l.trim().is_empty() { "" } else { &l[indent..] }).collect::<Vec<_>>().join("\n")
}

/// The string literals of `source`, in order: `"""…"""` (trimIndent-ed) and `"…"`.
fn string_literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find('"') {
        rest = &rest[start..];
        if let Some(body) = rest.strip_prefix("\"\"\"") {
            let end = body.find("\"\"\"").expect("a closed raw string");
            out.push(trim_indent(&body[..end]));
            rest = &body[end + 3..];
        } else {
            let body = &rest[1..];
            let end = body.find('"').expect("a closed string");
            out.push(body[..end].to_owned());
            rest = &body[end + 1..];
        }
    }
    out
}

/// The migrations of `Schema.kt`: one list of statements per version.
fn kotlin_migrations(schema_kt: &str) -> Vec<Vec<String>> {
    let start = schema_kt.find("migrations: List<List<String>> = listOf(").expect("the migrations list");
    let end = schema_kt[start..].find("val VERSION").expect("the end of the migrations list") + start;
    let block: String =
        schema_kt[start..end].lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let block = &block[block.find("listOf(").expect("the outer listOf") + "listOf(".len()..];
    block.split("listOf(").skip(1).map(string_literals).collect()
}

#[test]
fn migrations_equal_the_kotlin_schema_text_for_text() {
    let Some(repo) = kotlin_repo() else {
        eprintln!("skipped: the Kotlin repository was not found (set CICADA_KOTLIN_DIR)");
        return;
    };
    let source = std::fs::read_to_string(repo.join(SCHEMA_KT)).expect("Schema.kt");
    let kotlin = kotlin_migrations(&source);
    assert_eq!(kotlin.len() as i64, schema::VERSION, "number of migrations");
    for (i, statements) in kotlin.iter().enumerate() {
        assert_eq!(schema::statements_of(i + 1), statements.as_slice(), "migration {}", i + 1);
    }
}

fn sqlite3_available() -> bool {
    Command::new("sqlite3").arg("-version").output().is_ok_and(|o| o.status.success())
}

fn sqlite3(file: &Path, sql: &str) -> String {
    let output = Command::new("sqlite3").arg(file).arg(sql).output().expect("sqlite3 runs");
    assert!(output.status.success(), "sqlite3 failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).expect("UTF-8 output")
}

const MASTER: &str = "PRAGMA user_version; SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name; \
                      SELECT name, value FROM sequences ORDER BY name;";

#[tokio::test]
async fn the_sqlite3_cli_builds_the_same_database_from_the_kotlin_statements() {
    if !sqlite3_available() {
        eprintln!("skipped: the sqlite3 CLI is not installed");
        return;
    }
    let Some(repo) = kotlin_repo() else {
        eprintln!("skipped: the Kotlin repository was not found (set CICADA_KOTLIN_DIR)");
        return;
    };
    let kotlin = kotlin_migrations(&std::fs::read_to_string(repo.join(SCHEMA_KT)).expect("Schema.kt"));
    let from_kotlin = TempDb::new();
    let mut script: String = kotlin.iter().flatten().map(|s| format!("{s};\n")).collect();
    script.push_str(&format!("PRAGMA user_version = {};", kotlin.len()));
    sqlite3(&from_kotlin.file, &script);

    let from_rust = TempDb::new();
    from_rust.open(Arc::new(VirtualClock::new(time::from_micros(0)))).await.close().await;
    let rust = sqlite3(&from_rust.file, MASTER);
    assert_eq!(rust, sqlite3(&from_kotlin.file, MASTER));
    assert_eq!(sqlite3(&from_rust.file, "PRAGMA journal_mode; PRAGMA integrity_check;"), "wal\nok\n");
}

#[tokio::test]
async fn a_database_written_by_the_kotlin_store_opens_and_reads() {
    let Some(original) = std::env::var_os("CICADA_KOTLIN_DB") else {
        eprintln!("skipped: set CICADA_KOTLIN_DB to a database file written by the Kotlin store");
        return;
    };
    let db = TempDb::new();
    std::fs::copy(&original, &db.file).expect("copy the database");
    for suffix in ["-wal", "-shm"] {
        let mut side = original.clone();
        side.push(suffix);
        if Path::new(&side).exists() {
            let mut target = db.file.clone().into_os_string();
            target.push(suffix);
            std::fs::copy(&side, target).expect("copy the WAL");
        }
    }
    let options =
        SqliteOptions { clock: Arc::new(SystemClock), retention: DEFAULT_RETENTION * 1000, ..SqliteOptions::default() };
    let store = SqliteStore::open(&db.file, options).await.expect("the Kotlin database opens");
    assert_eq!(db.pragma("user_version").await, schema::VERSION.to_string());
    let types = store.list_types().await.unwrap();
    let actors = store.list("", None, 1000).await.unwrap().items;
    let pending = store.list_pending("", None, 1000).await.unwrap().items;
    let failures = store.failures("", chrono_min(), None, 1000).await.unwrap().items;
    for address in &actors {
        store.get(address).await.unwrap().expect("listed instances exist");
    }
    for message in &pending {
        store.status(&message.id).await.unwrap().expect("pending messages have a status");
    }
    let outcome_ids = db.column_names("SELECT message_id FROM outcomes").await;
    for id in &outcome_ids {
        store.chain_entries(&[cicada::store::MessageId::new(id.as_str()).unwrap()]).await.unwrap();
    }
    eprintln!(
        "read {} types, {} instances, {} pending messages, {} failures, {} outcomes",
        types.len(),
        actors.len(),
        pending.len(),
        failures.len(),
        outcome_ids.len()
    );
    store.close().await;
}

fn chrono_min() -> cicada::time::Timestamp {
    time::from_micros(i64::MIN)
}

/// Writes a database with every kind of row to `CICADA_WRITE_DB`, for the Kotlin store to read: a registered type,
/// instances with state, pending messages with headers, correlation id, retry and retention policies and a cause, a
/// claimed one, a rescheduled one, and `done`, `failed`, `cancelled` and removed outcomes.
#[tokio::test]
async fn writes_a_database_for_the_kotlin_store() {
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;

    use cicada::address::UriTemplate;
    use cicada::retention::{Keep, Retention};
    use cicada::retry::{Backoff, RetryPolicy};
    use cicada::store::{
        CommitRequest, Delivery, EnqueueResult, NodeId, OutgoingMessage, StateChange, TurnOutcome, TypeDef,
    };
    use serde_json::json;

    let Some(target) = std::env::var_os("CICADA_WRITE_DB") else {
        eprintln!("skipped: set CICADA_WRITE_DB to the file to write for the Kotlin store");
        return;
    };
    let store = SqliteStore::open(&target, SqliteOptions::default()).await.unwrap();
    let template = UriTemplate::parse("actor://rust/{id}").unwrap();
    let def = TypeDef {
        template: template.clone(),
        contract: json!({"template": "actor://rust/{id}", "messages": ["M"]}),
        contract_hash: "rust-hash".into(),
    };
    store.register(def).await.unwrap();
    let address = |id: &str| template.expand(&BTreeMap::from([("id".to_owned(), id.to_owned())])).unwrap();
    let (a, b) = (address("a"), address("b"));
    store.create(&a, &template, Some(json!({"n": 1}))).await.unwrap();
    store.create(&b, &template, None).await.unwrap();
    let enqueue = |m: OutgoingMessage| {
        let store = &store;
        async move {
            match store.enqueue(m).await.unwrap() {
                EnqueueResult::Enqueued(m) => m,
                other => panic!("{other:?}"),
            }
        }
    };
    let rich = OutgoingMessage {
        headers: BTreeMap::from([("trace".to_owned(), "t-1".to_owned())]),
        correlation_id: Some("corr".into()),
        retry: Some(RetryPolicy::new(3, Backoff::Fixed { delay: Duration::from_secs(1) }).unwrap()),
        retention: Some(Retention { done: Some(Keep::Remove), failed: Some(Keep::Last { count: 5 }) }),
        from: Some(b.clone()),
        ..OutgoingMessage::new(a.clone(), "M", json!({"x": [1, 2.5, "s"]}))
    };
    let first = enqueue(rich.clone()).await;
    let _second = enqueue(OutgoingMessage { id: cicada::store::MessageId::random(), ..rich.clone() }).await;
    let later = enqueue(OutgoingMessage {
        delivery: Delivery::After(Duration::from_secs(3600)),
        caused_by: Some(first.id.clone()),
        ..OutgoingMessage::new(b.clone(), "M", json!(null))
    })
    .await;
    let node = NodeId::new("rust-node").unwrap();
    let types = BTreeSet::from([template.clone()]);
    // a: done (removed at once by its retention), then failed, then rescheduled; b: one cancelled
    let claim = store.claim_due(&types, &node, Duration::from_secs(30), 10).await.unwrap().remove(0);
    let done = CommitRequest {
        state: StateChange::NewState(Some(json!({"n": 2}))),
        sends: vec![OutgoingMessage {
            caused_by: Some(claim.message.id.clone()),
            ..OutgoingMessage::new(b.clone(), "M", json!(7))
        }],
        ..CommitRequest::new(a.clone(), claim.epoch, claim.message.id.clone(), TurnOutcome::Done)
    };
    store.commit(done).await.unwrap();
    let claim = store.claim_due(&types, &node, Duration::from_secs(30), 10).await.unwrap();
    for c in claim {
        let outcome = if c.actor == a { TurnOutcome::Failed("boom".into()) } else { TurnOutcome::Done };
        store.commit(CommitRequest::new(c.actor.clone(), c.epoch, c.message.id.clone(), outcome)).await.unwrap();
    }
    let retried = enqueue(OutgoingMessage::new(a.clone(), "M", json!("retry me"))).await;
    let c = store.claim_due(&types, &node, Duration::from_secs(30), 10).await.unwrap().remove(0);
    assert_eq!(c.message.id, retried.id);
    let rescheduled =
        TurnOutcome::Rescheduled { delivery: Delivery::After(Duration::from_secs(60)), error: "again".into() };
    store.commit(CommitRequest::new(a.clone(), c.epoch, c.message.id.clone(), rescheduled)).await.unwrap();
    store.cancel(&later.id).await.unwrap();
    let held = enqueue(OutgoingMessage::new(b.clone(), "M", json!({"held": true}))).await;
    let c = store.claim_due(&types, &node, Duration::from_secs(600), 10).await.unwrap().remove(0);
    assert_eq!(c.message.id, held.id);
    eprintln!("wrote {}", Path::new(&target).display());
    store.close().await;
}
