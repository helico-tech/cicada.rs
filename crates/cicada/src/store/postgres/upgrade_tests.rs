//! The upgrade from schema version 1: a schema built with the migrations of that version, rows written with SQL as a
//! node of that version left them, then opened by this version. Skipped when `CICADA_PG_URL` is unset.

use std::collections::hash_map::RandomState;
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::panic::AssertUnwindSafe;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::address::ActorAddress;
use crate::store::Store;
use futures::FutureExt;
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, Connection, PgConnection};

use crate::store::postgres::schema::{self, quote};
use crate::store::postgres::{PgConnectOptions, PostgresStore, PostgresStoreConfig};

static SCHEMAS: AtomicU32 = AtomicU32::new(0);

/// A fresh schema of schema version 1 in the test database.
struct V1 {
    url: String,
    name: String,
}

impl V1 {
    async fn connect(url: &str) -> PgConnection {
        PgConnection::connect_with(&PgConnectOptions::from_str(url).unwrap()).await.expect("test database reachable")
    }

    /// Runs `test` on a fresh schema of version 1 and drops the schema afterwards, also when `test` fails.
    async fn run<F: Future<Output = ()>>(test: impl FnOnce(V1) -> F) {
        let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
            eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
            return;
        };
        let _running = crate::tests::postgres::common::permit().await;
        let random = RandomState::new().build_hasher().finish() & 0xFF_FFFF;
        let n = SCHEMAS.fetch_add(1, Ordering::SeqCst);
        let name = format!("upgrade_{}_{n}_{random:06x}", std::process::id());
        let mut c = Self::connect(&url).await;
        schema::migrate_to(&mut c, &name, 1).await.unwrap();
        let result = AssertUnwindSafe(test(V1 { url: url.clone(), name: name.clone() })).catch_unwind().await;
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

    async fn version(&self) -> i32 {
        let mut c = Self::connect(&self.url).await;
        let sql = format!("SELECT max(version) FROM {}.cicada_schema_version", quote(&self.name));
        let version = sqlx::query_scalar(AssertSqlSafe(sql)).fetch_one(&mut c).await.unwrap();
        c.close().await.unwrap();
        version
    }

    async fn open(&self) -> PostgresStore {
        let config = PostgresStoreConfig { schema: self.name.clone(), max_pool_size: 2, ..Default::default() };
        PostgresStore::connect(&self.url, config).await.expect("the store opens")
    }
}

fn address(text: &str) -> ActorAddress {
    ActorAddress::parse(text).unwrap()
}

#[tokio::test]
async fn instances_with_and_without_a_state() {
    V1::run(async |db| {
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
        assert_eq!(db.version().await, 2);
    })
    .await;
}

#[tokio::test]
async fn the_store_refuses_a_missing_state() {
    V1::run(async |db| {
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
async fn a_type_with_state_keeps_its_registration() {
    V1::run(async |db| {
        let kept = r#"{"type":"kept","key":[],"state":{"type":"object"},"messages":{}}"#;
        db.exec(&format!(
            r#"INSERT INTO {{s}}.actor_types (name, contract, contract_hash) VALUES
                ('kept', '{kept}', 'sha256:kept'),
                ('stateless', '{{"type":"stateless","key":[],"state":null,"messages":{{}}}}', 'sha256:s'),
                ('older', '{{"type":"older","key":[],"messages":{{}}}}', 'sha256:o')"#
        ))
        .await
        .unwrap();

        let store = db.open().await;
        let types = store.list_types().await.unwrap();
        let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["kept"], "the registrations without a state schema are removed");
        assert_eq!(types[0].contract_hash, "sha256:kept");
        assert_eq!(types[0].contract, serde_json::from_str::<Value>(kept).unwrap());
        store.close().await;
    })
    .await;
}
