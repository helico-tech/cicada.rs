//! Rules shared by stores: the registry, claim arguments, store time and delivery times.

use std::collections::BTreeSet;
use std::time::Duration;

use cicada::address::TypeName;
use cicada::store::support;
use cicada::store::{Delivery, RegisterResult, StoreError, TypeDef};
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp};
use serde_json::json;

fn type_def(name: &str, hash: &str) -> TypeDef {
    TypeDef { name: TypeName::parse(name).unwrap(), contract: json!({}), contract_hash: hash.into() }
}

// ---- registry ----

#[test]
fn registering_a_name_again_is_idempotent_for_an_equal_hash_and_a_conflict_for_another() {
    let first = type_def("jobs", "h1");
    assert_eq!(
        support::register_again(first.clone(), &type_def("jobs", "h1")),
        RegisterResult::Registered(first.clone())
    );
    match support::register_again(first.clone(), &type_def("jobs", "h2")) {
        RegisterResult::Conflict { existing, reason } => {
            assert_eq!(existing, first);
            assert_eq!(reason, support::hash_conflict_reason(&TypeName::parse("jobs").unwrap()));
            assert!(reason.contains("jobs"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn claim_arguments_with_nothing_to_claim_are_not_an_error() {
    let types = BTreeSet::from([TypeName::parse("jobs").unwrap()]);
    assert!(support::check_claim(&types, Duration::from_secs(1), 1).unwrap());
    assert!(!support::check_claim(&types, Duration::from_secs(1), 0).unwrap());
    assert!(!support::check_claim(&BTreeSet::new(), Duration::from_secs(1), 1).unwrap());
    assert!(matches!(support::check_claim(&types, Duration::ZERO, 1), Err(StoreError::Invalid(_))));
}

// ---- store time ----

fn now() -> Timestamp {
    time::parse_instant("2026-10-02T12:00:00.123456Z").unwrap()
}

fn plus(t: Timestamp, d: Duration) -> Timestamp {
    time::add(t, d).unwrap()
}

#[test]
fn truncates_to_microseconds_and_round_trips_epoch_micros() {
    let precise = time::parse_instant("2026-10-02T12:00:00.123456789Z").unwrap();
    assert_eq!(time::truncate(precise), time::parse_instant("2026-10-02T12:00:00.123456Z").unwrap());
    let before = time::parse_instant("1969-12-31T23:59:59.999999Z").unwrap();
    for i in [now(), before, time::from_micros(0)] {
        assert_eq!(time::from_micros(time::to_micros(i)), i);
    }
}

#[test]
fn delivery_is_never_earlier_than_now() {
    let secs5 = Duration::from_secs(5);
    assert_eq!(Delivery::Now.resolve(now()).unwrap(), now());
    assert_eq!(Delivery::At(time::sub(now(), secs5)).resolve(now()).unwrap(), now());
    assert_eq!(Delivery::At(plus(now(), secs5)).resolve(now()).unwrap(), plus(now(), secs5));
    assert_eq!(Delivery::After(secs5).resolve(now()).unwrap(), plus(now(), secs5));
    // A negative delay cannot be written: `Duration` is unsigned.
}

#[test]
fn delivery_is_rounded_up_to_microseconds_never_down() {
    let micro = Duration::from_micros(1);
    assert_eq!(Delivery::At(plus(now(), Duration::from_nanos(1))).resolve(now()).unwrap(), plus(now(), micro));
    assert_eq!(Delivery::After(Duration::from_nanos(999)).resolve(now()).unwrap(), plus(now(), micro));
    assert_eq!(Delivery::After(2 * micro).resolve(now()).unwrap(), plus(now(), 2 * micro));
}

#[test]
fn delivery_times_outside_the_supported_range_are_refused_never_wrapped() {
    assert!(Delivery::at(plus(MAX_INSTANT, Duration::from_micros(1))).is_err());
    assert!(Delivery::after(MAX_DELAY + Duration::from_secs(86_400)).is_err());
    let late = time::parse_instant("9500-01-01T00:00:00Z").unwrap();
    assert!(Delivery::After(MAX_DELAY).resolve(late).is_err());
    assert_eq!(Delivery::At(MAX_INSTANT).resolve(now()).unwrap(), MAX_INSTANT);
    assert_eq!(time::from_micros(time::to_micros(MAX_INSTANT)), MAX_INSTANT);
    // Beyond chrono's range, so it cannot even be parsed.
    assert!(time::parse_instant("+300000-01-01T00:00:00Z").is_err());
}
