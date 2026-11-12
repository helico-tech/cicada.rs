//! Runs the conformance suite against `InMemoryStore` on the system clock, the way a backend with a server-side clock
//! (PostgreSQL) runs it: time advances by waiting, durations scale with the tick, and time-exact cases are skipped.
//! This checks the suite's real-time paths without a database.

use std::sync::Arc;
use std::time::Duration;

use crate::address::ActorAddress;
use crate::store::{Backend, SystemClock};
use crate::testing::InMemoryStore;
use crate::tests::conformance::{FixtureFactory, StoreFixture};
use async_trait::async_trait;

struct InMemoryRealTime;

struct RealTimeFixture {
    store: Arc<InMemoryStore>,
}

#[async_trait]
impl FixtureFactory for InMemoryRealTime {
    type Fixture = RealTimeFixture;

    fn tick(&self) -> Duration {
        Duration::from_millis(50)
    }

    async fn create(&self, retention: Duration) -> RealTimeFixture {
        RealTimeFixture { store: Arc::new(InMemoryStore::new(Arc::new(SystemClock), retention)) }
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
        self.store.remove_row_by_hand(address);
    }

    async fn close(&mut self) {
        self.store.close().await;
    }
}

store_conformance_tests!(InMemoryRealTime);
