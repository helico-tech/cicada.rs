//! Runs the conformance suite against `InMemoryStore` with a virtual clock that the fixture can step back.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::store::{Clock, Store};
use cicada::time::{self, Timestamp};
use cicada_store_conformance::{FixtureFactory, StoreFixture};
use cicada_test::{InMemoryStore, VirtualClock};

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

struct InMemory;

struct InMemoryFixture {
    clock: Arc<SteppingClock>,
    store: Arc<InMemoryStore>,
}

#[async_trait]
impl FixtureFactory for InMemory {
    type Fixture = InMemoryFixture;

    async fn create(&self, retention: Duration) -> InMemoryFixture {
        let clock = Arc::new(SteppingClock::default());
        let store = Arc::new(InMemoryStore::new(clock.clone(), retention));
        InMemoryFixture { clock, store }
    }
}

#[async_trait]
impl StoreFixture for InMemoryFixture {
    fn store(&self) -> Arc<dyn Store> {
        self.store.clone()
    }

    fn is_virtual_time(&self) -> bool {
        true
    }

    async fn advance(&self, duration: Duration) {
        self.clock.virtual_clock.advance_by(duration);
    }

    fn supports_clock_step_back(&self) -> bool {
        true
    }

    async fn step_back(&self, duration: Duration) {
        *self.clock.offset.lock().unwrap() += duration;
    }

    async fn close(&mut self) {
        self.store.close().await;
    }
}

cicada_store_conformance::store_conformance_tests!(InMemory);
