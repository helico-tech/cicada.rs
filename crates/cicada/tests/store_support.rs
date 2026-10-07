//! Rules shared by stores: registry resolution, store time, delivery times and envelopes (Kotlin
//! `RegistrySupportTest`, `StoreSupportTest`).

use std::collections::BTreeMap;
use std::time::Duration;

use cicada::address::{ActorAddress, UriTemplate};
use cicada::store::support::{self, envelope_from_json, envelope_to_json};
use cicada::store::{Delivery, Message, MessageId, StoreError, TypeDef};
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp};
use serde_json::json;

fn type_def(template: &str) -> TypeDef {
    TypeDef { template: UriTemplate::parse(template).unwrap(), contract: json!({}), contract_hash: "h".into() }
}

fn a(s: &str) -> ActorAddress {
    ActorAddress::parse(s).unwrap()
}

// ---- registry ----

#[test]
fn an_address_resolves_to_the_one_template_that_matches_it() {
    let types = vec![type_def("actor://jobs/{id}"), type_def("actor://jobs/{id}/steps/{n}"), type_def("actor://pricing")];
    let r = support::resolve(&types, &a("actor://jobs/42/steps/1")).unwrap().unwrap();
    assert_eq!(r.type_def, types[1]);
    assert_eq!(r.params, BTreeMap::from([("id".to_owned(), "42".to_owned()), ("n".to_owned(), "1".to_owned())]));
    assert_eq!(support::resolve(&types, &a("actor://pricing")).unwrap().unwrap().type_def, types[2]);
    assert!(support::resolve(&types, &a("actor://jobs")).unwrap().is_none());
}

#[test]
fn overlapping_registered_templates_are_an_error_not_a_precedence_rule() {
    // Only a registry written before templates had to be disjoint can hold these two.
    let types = vec![type_def("actor://jobs/{id}"), type_def("actor://jobs/cleanup")];
    assert_eq!(support::resolve(&types, &a("actor://jobs/42")).unwrap().unwrap().type_def, types[0]);
    match support::resolve(&types, &a("actor://jobs/cleanup")) {
        Err(StoreError::Corrupt(e)) => assert!(e.contains("actor://jobs/cleanup"), "{e}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn overlapping_finds_another_template_that_shares_an_address_never_the_template_itself() {
    let types = vec![type_def("actor://jobs/{id}"), type_def("actor://sync/{tenant}")];
    let t = |s: &str| UriTemplate::parse(s).unwrap();
    assert_eq!(support::overlapping(&types, &t("actor://jobs/cleanup")), Some(&types[0]));
    assert_eq!(support::overlapping(&types, &t("actor://jobs/{id}")), None);
    assert_eq!(support::overlapping(&types, &t("actor://jobs/{id}/steps/{n}")), None);
    assert_eq!(
        support::overlap_reason(&t("actor://jobs/cleanup"), &types[0].template),
        "Template actor://jobs/cleanup overlaps actor://jobs/{id}: both match actor://jobs/cleanup"
    );
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
    // Kotlin's `+300000-01-01` cannot even be parsed: chrono's range ends before it.
    assert!(time::parse_instant("+300000-01-01T00:00:00Z").is_err());
}

#[test]
fn envelope_json_round_trips() {
    let m = Message {
        id: MessageId::random(),
        to: a("actor://a/b"),
        from: Some(a("actor://c/d")),
        message_type: "Run".into(),
        payload: json!({"x": 1, "s": "y"}),
        deliver_at: now(),
        correlation_id: Some("corr".into()),
        headers: [("retry".to_owned(), "2".to_owned())].into(),
        seq: 7,
        attempt: 3,
        failures: 0,
        last_error: None,
        retry: None,
        caused_by: None,
        retention: None,
    };
    assert_eq!(envelope_from_json(&envelope_to_json(&m)).unwrap(), m);
    let minimal = Message { from: None, correlation_id: None, headers: BTreeMap::new(), payload: json!("p"), ..m };
    assert_eq!(envelope_from_json(&envelope_to_json(&minimal)).unwrap(), minimal);
}
