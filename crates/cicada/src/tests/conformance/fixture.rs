//! What a backend supplies to run the suite: a factory of fresh fixtures, and the fixture itself.

use std::sync::Arc;
use std::time::Duration;

use crate::address::ActorAddress;
use crate::store::Backend;
use async_trait::async_trait;

/// One fresh, empty store under test, created per case by [`FixtureFactory::create`].
///
/// A fixture decides how time moves. With a virtual clock ([`is_virtual_time`](Self::is_virtual_time) = true)
/// [`advance`](Self::advance) moves the store clock exactly, and boundary cases run. With the real clock (e.g.
/// PostgreSQL's `now()`), `advance` waits at least the given duration; boundary-exact cases are skipped and the rest
/// use [`FixtureFactory::tick`]-scaled durations.
#[async_trait]
pub trait StoreFixture: Send + Sync + 'static {
    /// The store under test. After [`reopen`](Self::reopen) this is the new handle.
    fn store(&self) -> Arc<dyn Backend>;

    /// True when [`advance`](Self::advance) moves a virtual store clock exactly and instantly.
    fn is_virtual_time(&self) -> bool;

    /// Moves store time forward by at least `duration`: exactly when virtual, by waiting otherwise.
    async fn advance(&self, duration: Duration);

    /// Another handle on the same backend, as a second node would have (a separate connection or store instance).
    /// Used by concurrency cases. The default returns [`store`](Self::store) itself. Handles are closed by
    /// [`close`](Self::close).
    async fn peer(&self) -> Arc<dyn Backend> {
        self.store()
    }

    /// True when [`step_back`](Self::step_back) is supported: the fixture can move the *underlying* clock backwards,
    /// as an NTP correction of a wall clock would.
    fn supports_clock_step_back(&self) -> bool {
        false
    }

    /// Moves the clock the store reads backwards by `duration` (a wall-clock step back). The store must keep its own
    /// time monotonic regardless.
    async fn step_back(&self, _duration: Duration) {
        panic!("step_back is not supported")
    }

    /// True when [`remove_row`](Self::remove_row) is supported.
    fn supports_removing_rows(&self) -> bool {
        false
    }

    /// Deletes the row of the instance `address` by hand, past the store, as an operator with SQL could: its mailbox
    /// and the watches it holds stay. The calls of a store cannot leave a watch whose watcher does not exist.
    async fn remove_row(&self, _address: &ActorAddress) {
        panic!("remove_row is not supported")
    }

    /// True when the backend stores a negative zero as a zero (PostgreSQL JSONB keeps numbers as `numeric`, which has no
    /// negative zero). Every other number of a state or a payload is read back as it was written.
    fn normalizes_negative_zero(&self) -> bool {
        false
    }

    /// True when [`reopen`](Self::reopen) is supported, i.e. the backend is durable (S1).
    fn supports_reopen(&self) -> bool {
        false
    }

    /// Closes every handle and opens the same backend again, simulating a process restart. Afterwards
    /// [`store`](Self::store) is the new handle.
    async fn reopen(&mut self) {
        panic!("reopen is not supported")
    }

    /// Closes the store and every peer handle, and deletes any backing data.
    async fn close(&mut self);
}

/// A backend under test: creates a fresh fixture per case and sets the suite's unit of time.
#[async_trait]
pub trait FixtureFactory: Send + Sync {
    /// The fixture this backend creates.
    type Fixture: StoreFixture;

    /// The suite's unit of time. Lease TTLs, delays and retention in the cases are small multiples of it.
    /// Virtual-time backends can keep the default; real-time backends should use a value well above their operation
    /// latency, e.g. 250 ms.
    fn tick(&self) -> Duration {
        Duration::from_secs(1)
    }

    /// Retention for cases that do not test retention: long enough never to expire during a case.
    fn long_retention(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }

    /// Why the backend cannot be tested here (e.g. no database configured); every case is then skipped.
    fn skip_reason(&self) -> Option<String> {
        None
    }

    /// Creates a fresh, empty store that keeps outcomes for `retention`. Called once per case; the fixture is closed
    /// afterwards.
    async fn create(&self, retention: Duration) -> Self::Fixture;
}
