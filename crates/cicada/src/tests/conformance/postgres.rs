//! Runs the S1–S10 conformance suite against [`PostgresStore`] on a real PostgreSQL (`CICADA_PG_URL`).
//!
//! Time is the database clock, so the fixture is real-time: `advance` waits, durations scale with the tick, and the
//! suite skips its time-exact cases. Each case gets a fresh schema; peers are separate stores with their own pools on
//! that schema, as other nodes would be; `reopen` closes every handle and opens the schema again (S1).
//!
//! Each case takes its database through the limit that every PostgreSQL test of the crate shares
//! ([`crate::tests::postgres::common::permit`]), and peers (up to 16 per case, each used by one task) have pools of 1,
//! so that the connections stay below the default `max_connections` (100) of the docker-compose PostgreSQL.

use std::sync::Arc;
use std::time::Duration;

use crate::address::ActorAddress;
use crate::store::Backend;
use crate::store::postgres::{PostgresOptions, PostgresStore};
use crate::tests::conformance::{FixtureFactory, StoreFixture};
use crate::tests::postgres::common::TestDb;
use async_trait::async_trait;

struct Postgres;

struct PostgresFixture {
    db: TestDb,
    retention: Duration,
    store: Arc<PostgresStore>,
}

async fn open(db: &TestDb, retention: Duration) -> Arc<PostgresStore> {
    db.open_with(PostgresOptions { retention, ..db.config() }).await
}

async fn open_peer(db: &TestDb, retention: Duration) -> Arc<PostgresStore> {
    db.open_with(PostgresOptions { retention, max_pool_size: 1, ..db.config() }).await
}

#[async_trait]
impl FixtureFactory for Postgres {
    type Fixture = PostgresFixture;

    fn tick(&self) -> Duration {
        Duration::from_millis(200)
    }

    fn skip_reason(&self) -> Option<String> {
        crate::tests::postgres::common::pg_url().is_none().then(|| "CICADA_PG_URL is unset".to_owned())
    }

    async fn create(&self, retention: Duration) -> PostgresFixture {
        let db = TestDb::new("c").await.expect("CICADA_PG_URL is set");
        let store = open(&db, retention).await;
        PostgresFixture { db, retention, store }
    }
}

#[async_trait]
impl StoreFixture for PostgresFixture {
    fn store(&self) -> Arc<dyn Backend> {
        self.store.clone()
    }

    fn is_virtual_time(&self) -> bool {
        false
    }

    async fn advance(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    async fn peer(&self) -> Arc<dyn Backend> {
        open_peer(&self.db, self.retention).await
    }

    fn supports_removing_rows(&self) -> bool {
        true
    }

    async fn remove_row(&self, address: &ActorAddress) {
        // A canonical address holds no quote: every other character than `A-Z a-z 0-9 . _ ~ - : /` is encoded.
        self.db.sql(&format!("DELETE FROM \"{}\".actors WHERE address = '{address}'", self.db.schema)).await;
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

store_conformance_tests!(Postgres);
