//! Retry policies: backoff, bounds, JSON form, and how envelopes carry them.

use std::time::Duration;

use cicada::address::ActorAddress;
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::support::{envelope_from_json, envelope_to_json};
use cicada::store::{Message, MessageId};
use cicada::time::{self, MAX_DELAY};
use serde_json::{Value, json};

const DAY: Duration = Duration::from_secs(86_400);

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn fixed(delay: Duration) -> Backoff {
    Backoff::fixed(delay).unwrap()
}

#[test]
fn exponential_grows_by_factor_up_to_max_fixed_stays() {
    let exp = Backoff::exponential(secs(1), 2.0, secs(10)).unwrap();
    assert_eq!((1..=6).map(|r| exp.delay(r)).collect::<Vec<_>>(), [1, 2, 4, 8, 10, 10].map(secs));
    let tripled = Backoff::exponential(Duration::from_millis(100), 3.0, secs(60)).unwrap();
    assert_eq!((1..=3).map(|r| tripled.delay(r)).collect::<Vec<_>>(), [100, 300, 900].map(Duration::from_millis));
    assert_eq!(tripled.delay(10_000), secs(60), "no overflow for huge retry numbers");
    assert_eq!((1..=3).map(|r| fixed(secs(5)).delay(r)).collect::<Vec<_>>(), vec![secs(5); 3]);
    assert_eq!(Backoff::exponential(DAY, 2.0, MAX_DELAY).unwrap().delay(1_000), MAX_DELAY);
}

#[test]
fn retries_after_counts_handler_runs_including_the_first() {
    let policy = RetryPolicy::new(3, fixed(secs(1))).unwrap();
    assert!(policy.retries_after(1));
    assert!(policy.retries_after(2));
    assert!(!policy.retries_after(3));
    assert!(!RetryPolicy::new(1, fixed(secs(1))).unwrap().retries_after(1));
}

#[test]
fn bounds_are_validated() {
    assert!(RetryPolicy::new(0, fixed(secs(1))).is_err());
    assert!(RetryPolicy::new(RetryPolicy::MAX_ATTEMPTS + 1, fixed(secs(1))).is_err());
    assert!(Backoff::fixed(Duration::ZERO).is_err());
    assert!(Backoff::fixed(MAX_DELAY + DAY).is_err());
    assert!(Backoff::exponential(Duration::ZERO, 2.0, secs(1)).is_err());
    assert!(Backoff::exponential(secs(1), 0.9, secs(60)).is_err());
    assert!(Backoff::exponential(secs(1), f64::NAN, secs(60)).is_err());
    assert!(Backoff::exponential(secs(1), f64::INFINITY, secs(60)).is_err());
    assert!(Backoff::exponential(secs(60), 2.0, secs(1)).is_err());
    assert!(Backoff::exponential(secs(1), 2.0, MAX_DELAY + DAY).is_err());
    // Negative and infinite durations cannot be written: `Duration` is unsigned and finite. From JSON, a negative one
    // is refused like zero.
    assert!(
        serde_json::from_value::<RetryPolicy>(
            json!({"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "-PT1S"}})
        )
        .is_err()
    );
    RetryPolicy::new(RetryPolicy::MAX_ATTEMPTS, fixed(MAX_DELAY)).unwrap(); // the bounds themselves are fine
}

#[test]
fn json_form_kind_discriminator_iso_durations_round_trip() {
    let exp = RetryPolicy::new(4, Backoff::exponential(secs(1), 2.0, secs(60)).unwrap()).unwrap();
    assert_eq!(
        exp.to_json(),
        json!({"maxAttempts": 4, "backoff": {"kind": "exponential", "initial": "PT1S", "factor": 2.0, "max": "PT1M"}})
    );
    let fix = RetryPolicy::new(3, fixed(secs(5))).unwrap();
    assert_eq!(fix.to_json(), json!({"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT5S"}}));
    for policy in [&exp, &fix] {
        assert_eq!(&serde_json::from_value::<RetryPolicy>(policy.to_json()).unwrap(), policy);
    }
    // The factor defaults to 2.
    assert_eq!(
        serde_json::from_value::<RetryPolicy>(
            json!({"maxAttempts": 4, "backoff": {"kind": "exponential", "initial": "PT1S", "max": "PT1M"}})
        )
        .unwrap(),
        exp
    );
    // Durations are ISO-8601 strings only: not a short string, not a number of milliseconds.
    for (initial, max) in [(json!(1000), json!("PT1M")), (json!("PT1S"), json!("1m"))] {
        let backoff = json!({"kind": "exponential", "initial": initial, "max": max});
        assert!(serde_json::from_value::<RetryPolicy>(json!({"maxAttempts": 4, "backoff": backoff})).is_err());
    }
    assert!(
        serde_json::from_value::<RetryPolicy>(json!({"maxAttempts": 0, "backoff": {"kind": "fixed", "delay": "PT1S"}}))
            .is_err()
    );
}

fn message(retry: Option<RetryPolicy>) -> Message {
    Message {
        id: MessageId::new("m").unwrap(),
        to: ActorAddress::parse("actor://x/1").unwrap(),
        from: None,
        message_type: "T".into(),
        payload: json!({}),
        deliver_at: time::parse_instant("2026-01-01T00:00:00Z").unwrap(),
        correlation_id: None,
        headers: Default::default(),
        seq: 1,
        attempt: 1,
        failures: 0,
        last_error: None,
        retry,
        caused_by: None,
        retention: None,
    }
}

#[test]
fn the_envelope_carries_the_policy_and_older_envelopes_decode_without_one() {
    let m = message(Some(RetryPolicy::new(2, fixed(secs(1))).unwrap()));
    assert_eq!(envelope_from_json(&envelope_to_json(&m)).unwrap(), m);
    let plain = message(None);
    assert_eq!(envelope_to_json(&plain)["retry"], Value::Null);
    assert_eq!(envelope_from_json(&envelope_to_json(&plain)).unwrap(), plain);
    let mut old = envelope_to_json(&m);
    for key in ["retry", "failures", "lastError"] {
        old.as_object_mut().unwrap().remove(key);
    }
    assert!(envelope_from_json(&old).unwrap().retry.is_none());
}

#[test]
fn stored_policies_are_decoded_tolerantly_one_this_version_cannot_read_is_no_policy() {
    // A policy written by a newer version (or a damaged row) must not break claims or reads.
    let future = r#"{"maxAttempts":3,"backoff":{"kind":"jittered","delay":"PT1S","spread":0.5}}"#;
    let invalid = r#"{"maxAttempts":0,"backoff":{"kind":"fixed","delay":"PT1S"}}"#;
    for text in [future, invalid, "not json", "\"three\"", "{}"] {
        assert!(RetryPolicy::from_stored_text(text, "test").is_none(), "{text}");
    }
    assert!(RetryPolicy::from_stored(&Value::Null, "test").is_none());
    let valid = RetryPolicy::new(2, fixed(secs(1))).unwrap();
    assert_eq!(RetryPolicy::from_stored_text(&valid.to_json().to_string(), "test"), Some(valid.clone()));
    assert!(serde_json::from_str::<RetryPolicy>(future).is_err());

    let m = message(Some(valid));
    let mut envelope = envelope_to_json(&m);
    envelope["retry"] = serde_json::from_str(future).unwrap();
    assert_eq!(envelope_from_json(&envelope).unwrap(), message(None));
}
