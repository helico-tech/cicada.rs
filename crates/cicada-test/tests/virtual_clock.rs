//! The virtual clock.

use std::time::Duration;

use cicada::store::Clock;
use cicada::time;
use cicada_test::VirtualClock;

#[test]
fn moves_only_when_told_to_and_never_backwards() {
    let clock = VirtualClock::default();
    let start = clock.now();
    assert_eq!(clock.now(), start);
    assert_eq!(clock.advance_by(Duration::from_secs(5)), time::add(start, Duration::from_secs(5)).unwrap());
    clock.set_to(time::add(start, Duration::from_secs(10)).unwrap());
    assert_eq!(clock.now(), time::add(start, Duration::from_secs(10)).unwrap());
    // A negative advance cannot be written: `Duration` is unsigned.
    let back = std::panic::catch_unwind(|| clock.set_to(start));
    assert!(back.is_err(), "virtual time cannot go backwards");
}
