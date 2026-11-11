//! Clocks for tests.

use std::sync::Mutex;
use std::time::Duration;

use crate::store::Clock;
use crate::time::{self, Timestamp};

/// The default start of virtual time: a fixed, round instant so tests are reproducible.
pub fn default_start() -> Timestamp {
    time::parse_instant("2026-01-01T00:00:00Z").expect("a valid instant")
}

/// A [`Clock`] that only moves when told to. Thread-safe; time never goes backwards.
#[derive(Debug)]
pub struct VirtualClock {
    current: Mutex<Timestamp>,
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new(default_start())
    }
}

impl VirtualClock {
    /// A clock standing at `start`.
    pub fn new(start: Timestamp) -> Self {
        Self { current: Mutex::new(start) }
    }

    /// Moves time forward by `duration` and returns the new time.
    pub fn advance_by(&self, duration: Duration) -> Timestamp {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        *current = time::add(*current, duration).expect("virtual time stays in range");
        *current
    }

    /// Sets time to `instant`, which must not be earlier than the current time.
    pub fn set_to(&self, instant: Timestamp) {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        assert!(instant >= *current, "Virtual time cannot go backwards ({} -> {})", *current, instant);
        *current = instant;
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> Timestamp {
        *self.current.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A [`Clock`] that reads tokio's clock: `start` plus the time tokio's `Instant` moved since the clock was made. Under
/// a paused runtime (`#[tokio::test(start_paused = true)]`) that is virtual time, which advances only when every task
/// waits, so store time, handler sleeps, heartbeats and turn timeouts all share it.
#[derive(Debug)]
pub struct TokioClock {
    start: Timestamp,
    base: tokio::time::Instant,
}

impl TokioClock {
    /// A clock that reads `start` now.
    pub fn new(start: Timestamp) -> Self {
        Self { start, base: tokio::time::Instant::now() }
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Timestamp {
        time::add(self.start, self.base.elapsed()).expect("virtual time stays in range")
    }
}
