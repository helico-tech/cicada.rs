//! Runs the S1–S10 conformance suite against [`PostgresStore`] on a real PostgreSQL (`CICADA_PG_URL`). Port of the
//! Kotlin `PostgresStoreConformanceTest`.
//!
//! Time is the database clock, so the fixture is real-time: `advance` waits, durations scale with the tick, and the
//! suite skips its time-exact cases. Each case gets a fresh schema; peers are separate stores with their own pools on
//! that schema, as other nodes would be; `reopen` closes every handle and opens the schema again (S1).
//!
//! At most [`PARALLEL_CASES`] cases run at once, and peers (up to 16 per case, each used by one task) have pools of 1, so that the cases'
//! connections stay below the default `max_connections` (100) of the docker-compose PostgreSQL; the Kotlin suite runs
//! its container with 500.

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cicada::store::Store;
use cicada_store_conformance::{FixtureFactory, StoreFixture};
use cicada_store_postgres::{PostgresStore, PostgresStoreConfig};
use common::TestDb;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const PARALLEL_CASES: usize = 4;

static RUNNING: std::sync::LazyLock<Arc<Semaphore>> = std::sync::LazyLock::new(|| Arc::new(Semaphore::new(PARALLEL_CASES)));

struct Postgres;

struct PostgresFixture {
    db: TestDb,
    retention: Duration,
    store: Arc<PostgresStore>,
    _running: OwnedSemaphorePermit,
}

async fn open(db: &TestDb, retention: Duration) -> Arc<PostgresStore> {
    db.open_with(PostgresStoreConfig { retention, ..db.config() }).await
}

async fn open_peer(db: &TestDb, retention: Duration) -> Arc<PostgresStore> {
    db.open_with(PostgresStoreConfig { retention, max_pool_size: 1, ..db.config() }).await
}

#[async_trait]
impl FixtureFactory for Postgres {
    type Fixture = PostgresFixture;

    fn tick(&self) -> Duration {
        Duration::from_millis(200)
    }

    fn skip_reason(&self) -> Option<String> {
        common::pg_url().is_none().then(|| "CICADA_PG_URL is unset".to_owned())
    }

    async fn create(&self, retention: Duration) -> PostgresFixture {
        let running = RUNNING.clone().acquire_owned().await.expect("the semaphore is never closed");
        let db = TestDb::new("c").expect("CICADA_PG_URL is set");
        let store = open(&db, retention).await;
        PostgresFixture { db, retention, store, _running: running }
    }
}

#[async_trait]
impl StoreFixture for PostgresFixture {
    fn store(&self) -> Arc<dyn Store> {
        self.store.clone()
    }

    fn is_virtual_time(&self) -> bool {
        false
    }

    async fn advance(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    async fn peer(&self) -> Arc<dyn Store> {
        open_peer(&self.db, self.retention).await
    }

    fn supports_reopen(&self) -> bool {
        true
    }

    async fn reopen(&mut self) {
        self.db.close_stores().await;
        self.store = open(&self.db, self.retention).await;
    }

    async fn close(&mut self) {
        self.db.cleanup().await;
    }
}

cicada_store_conformance::store_conformance_tests!(Postgres);
