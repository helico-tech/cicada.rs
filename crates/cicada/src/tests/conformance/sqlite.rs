//! Runs the S1–S10 conformance suite against `SqliteStore` twice: on a virtual clock that the fixture can step
//! back, with peers as separate store instances on the same file (as separate processes would be) and reopening; and on the system clock, where time
//! advances by waiting.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::address::ActorAddress;
use crate::store::sqlite::{SqliteOptions, SqliteStore};
use crate::store::{Backend, Clock, SystemClock};
use crate::testing::VirtualClock;
use crate::tests::conformance::{FixtureFactory, StoreFixture};
use crate::time::{self, Timestamp};
use async_trait::async_trait;
use tempfile::TempDir;

/// A temporary directory with the database file; removed when dropped.
fn temp_db() -> (TempDir, PathBuf) {
    let dir = tempfile::Builder::new().prefix("cicada-sqlite").tempdir().expect("a temp dir");
    let file = dir.path().join("actors.db");
    (dir, file)
}

/// Deletes the row of `address` in the database file on a connection of its own, past the store.
async fn remove_row(file: &PathBuf, address: &ActorAddress) {
    use sqlx::Connection;
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(file);
    let mut c = sqlx::SqliteConnection::connect_with(&options).await.expect("a raw connection");
    sqlx::query("DELETE FROM actors WHERE address = ?").bind(address.as_str()).execute(&mut c).await.expect("deleted");
    c.close().await.expect("closed");
}

async fn open(file: &PathBuf, clock: Arc<dyn Clock>, retention: Duration) -> Arc<SqliteStore> {
    let options = SqliteOptions { clock, retention, ..SqliteOptions::default() };
    Arc::new(SqliteStore::open(file, options).await.expect("the store opens"))
}

mod virtual_time {
    use super::*;

    /// A virtual clock minus an offset, so the fixture can step the clock the store reads backwards.
    #[derive(Default)]
    struct SteppingClock {
        virtual_clock: VirtualClock,
        offset: Mutex<Duration>,
    }

    impl Clock for SteppingClock {
        fn now(&self) -> Timestamp {
            time::sub(self.virtual_clock.now(), *self.offset.lock().unwrap())
        }
    }

    struct Sqlite;

    struct SqliteFixture {
        _dir: TempDir,
        file: PathBuf,
        retention: Duration,
        clock: Arc<SteppingClock>,
        store: Arc<SqliteStore>,
        /// Every handle opened, the store included, so `reopen` and `close` close them all.
        handles: Mutex<Vec<Arc<SqliteStore>>>,
    }

    impl SqliteFixture {
        async fn open_handle(&self) -> Arc<SqliteStore> {
            let store = open(&self.file, self.clock.clone(), self.retention).await;
            self.handles.lock().unwrap().push(store.clone());
            store
        }

        async fn close_handles(&self) {
            let handles = std::mem::take(&mut *self.handles.lock().unwrap());
            for handle in handles {
                handle.close().await;
            }
        }
    }

    #[async_trait]
    impl FixtureFactory for Sqlite {
        type Fixture = SqliteFixture;

        async fn create(&self, retention: Duration) -> SqliteFixture {
            let (dir, file) = temp_db();
            let clock = Arc::new(SteppingClock::default());
            let store = open(&file, clock.clone(), retention).await;
            let handles = Mutex::new(vec![store.clone()]);
            SqliteFixture { _dir: dir, file, retention, clock, store, handles }
        }
    }

    #[async_trait]
    impl StoreFixture for SqliteFixture {
        fn store(&self) -> Arc<dyn Backend> {
            self.store.clone()
        }

        fn is_virtual_time(&self) -> bool {
            true
        }

        async fn advance(&self, duration: Duration) {
            self.clock.virtual_clock.advance_by(duration);
        }

        async fn peer(&self) -> Arc<dyn Backend> {
            self.open_handle().await
        }

        fn supports_removing_rows(&self) -> bool {
            true
        }

        async fn remove_row(&self, address: &ActorAddress) {
            remove_row(&self.file, address).await;
        }

        fn supports_clock_step_back(&self) -> bool {
            true
        }

        async fn step_back(&self, duration: Duration) {
            *self.clock.offset.lock().unwrap() += duration;
        }

        fn supports_reopen(&self) -> bool {
            true
        }

        async fn reopen(&mut self) {
            self.close_handles().await;
            self.store = self.open_handle().await;
        }

        async fn close(&mut self) {
            self.close_handles().await;
        }
    }

    store_conformance_tests!(Sqlite);
}

mod real_time {
    use super::*;

    struct SqliteRealTime;

    struct RealTimeFixture {
        _dir: TempDir,
        file: PathBuf,
        store: Arc<SqliteStore>,
    }

    #[async_trait]
    impl FixtureFactory for SqliteRealTime {
        type Fixture = RealTimeFixture;

        fn tick(&self) -> Duration {
            Duration::from_millis(150)
        }

        async fn create(&self, retention: Duration) -> RealTimeFixture {
            let (dir, file) = temp_db();
            RealTimeFixture { store: open(&file, Arc::new(SystemClock), retention).await, file, _dir: dir }
        }
    }

    #[async_trait]
    impl StoreFixture for RealTimeFixture {
        fn store(&self) -> Arc<dyn Backend> {
            self.store.clone()
        }

        fn is_virtual_time(&self) -> bool {
            false
        }

        async fn advance(&self, duration: Duration) {
            tokio::time::sleep(duration).await;
        }

        fn supports_removing_rows(&self) -> bool {
            true
        }

        async fn remove_row(&self, address: &ActorAddress) {
            remove_row(&self.file, address).await;
        }

        async fn close(&mut self) {
            self.store.close().await;
        }
    }

    store_conformance_tests!(SqliteRealTime);
}
