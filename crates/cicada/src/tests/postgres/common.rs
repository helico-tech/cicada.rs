//! Shared helpers for tests against a real PostgreSQL: the database from `CICADA_PG_URL`, one fresh schema per test.

#![allow(dead_code)]

use std::collections::hash_map::RandomState;
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use sqlx::{AssertSqlSafe, Connection, PgConnection};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::store::Store;
use crate::store::postgres::{PgConnectOptions, PostgresStore, PostgresStoreConfig};

/// The test database URL, or `None` (with a note) when `CICADA_PG_URL` is unset.
pub fn pg_url() -> Option<String> {
    match std::env::var("CICADA_PG_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
            None
        }
    }
}

/// A [`TestDb`] for this test, or returns from the test when `CICADA_PG_URL` is unset.
macro_rules! test_db {
    ($prefix:literal) => {
        match $crate::tests::postgres::common::TestDb::new($prefix).await {
            Some(db) => db,
            None => return,
        }
    };
}

pub(crate) use test_db;

/// How many PostgreSQL tests hold a database at once. Every PostgreSQL test of the crate runs in one test binary, so
/// they share this limit; with it the connections of the tests (a conformance case opens up to 16 peers with a pool of
/// 1 each) stay below the 100 connections of the docker-compose and CI databases.
const PARALLEL_TESTS: usize = 4;

static RUNNING: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(PARALLEL_TESTS)));

/// A place under the shared limit, held for as long as the permit lives.
pub async fn permit() -> OwnedSemaphorePermit {
    RUNNING.clone().acquire_owned().await.expect("the semaphore is never closed")
}

static SCHEMAS: AtomicU32 = AtomicU32::new(0);

/// A fresh schema in the test database, taken under the shared limit. Stores opened through it are closed and the
/// schema is dropped by [`Self::cleanup`]; when that never ran (a failed test), `Drop` still drops the schema.
pub struct TestDb {
    pub url: String,
    pub schema: String,
    stores: Mutex<Vec<Arc<PostgresStore>>>,
    cleaned: AtomicBool,
    _running: OwnedSemaphorePermit,
}

impl TestDb {
    /// A fresh schema once the shared limit allows it, or `None` when `CICADA_PG_URL` is unset.
    pub async fn new(prefix: &str) -> Option<Self> {
        let url = pg_url()?;
        let running = permit().await;
        let random = RandomState::new().build_hasher().finish() & 0xFF_FFFF;
        let schema =
            format!("{prefix}_{}_{}_{random:06x}", std::process::id(), SCHEMAS.fetch_add(1, Ordering::SeqCst) + 1);
        Some(Self { url, schema, stores: Mutex::new(Vec::new()), cleaned: AtomicBool::new(false), _running: running })
    }

    /// The settings tests use: this schema, a small pool.
    pub fn config(&self) -> PostgresStoreConfig {
        PostgresStoreConfig { schema: self.schema.clone(), max_pool_size: 4, ..Default::default() }
    }

    /// Opens a store on this schema with [`Self::config`].
    pub async fn open(&self) -> Arc<PostgresStore> {
        self.open_with(self.config()).await
    }

    /// Opens a store on this schema with `config`.
    pub async fn open_with(&self, config: PostgresStoreConfig) -> Arc<PostgresStore> {
        let store = Arc::new(PostgresStore::connect(&self.url, config).await.expect("store opens"));
        self.stores.lock().unwrap().push(store.clone());
        store
    }

    /// A plain connection (auto-commit) for set-up and inspection.
    pub async fn connect(&self) -> PgConnection {
        let options = PgConnectOptions::from_str(&self.url).expect("a PostgreSQL URL");
        PgConnection::connect_with(&options).await.expect("test database reachable")
    }

    /// Runs `statement` on a plain connection.
    pub async fn sql(&self, statement: &str) {
        let mut c = self.connect().await;
        sqlx::raw_sql(AssertSqlSafe(statement.to_owned()))
            .execute(&mut c)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }

    /// Closes every store opened through this and forgets them.
    pub async fn close_stores(&self) {
        let stores: Vec<_> = std::mem::take(&mut *self.stores.lock().unwrap());
        for store in stores {
            store.close().await;
        }
    }

    /// Closes the stores and drops the schema.
    pub async fn cleanup(&self) {
        self.close_stores().await;
        let mut c = self.connect().await;
        drop_schema(&mut c, &self.schema).await;
        self.cleaned.store(true, Ordering::SeqCst);
    }
}

async fn drop_schema(c: &mut PgConnection, schema: &str) {
    sqlx::raw_sql(AssertSqlSafe(format!("SET lock_timeout = '10s'; DROP SCHEMA IF EXISTS \"{schema}\" CASCADE")))
        .execute(c)
        .await
        .unwrap_or_else(|e| panic!("dropping schema {schema}: {e}"));
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if self.cleaned.load(Ordering::SeqCst) {
            return;
        }
        let (url, schema) = (self.url.clone(), self.schema.clone());
        let _ = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async {
                let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
                if let Ok(mut c) = PgConnection::connect_with(&options).await {
                    drop_schema(&mut c, &schema).await;
                }
            });
        })
        .join();
    }
}

/// Waits until `condition` holds, polling every 20 ms; panics after `limit`.
pub async fn eventually<F, Fut>(limit: Duration, what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let wait = async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    if tokio::time::timeout(limit, wait).await.is_err() {
        panic!("timed out after {limit:?} waiting for: {what}");
    }
}
