//! Parsers answer any input with a value or an error, and what Cicada formats it parses back.
//!
//! Purely random text rarely gets past the first character of a format, so each parser also gets text shaped like its
//! own format, with numbers near the limits of the arithmetic.

use std::time::Duration;

use crate::address::{ActorAddress, TypeName, percent};
use crate::api::{BulkRequest, CreateActorRequest, SendRequest};
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::store::support::{
    decode_mailbox_cursor, decode_outcome_cursor, decode_pending_cursor, mailbox_cursor, outcome_cursor, pending_cursor,
};
use crate::store::{MessageId, OutgoingMessage, PageCursor};
use crate::time::{self, MAX_INSTANT, Timestamp};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

/// Digits of any length up to past the range of `i128`, sometimes with a fraction or a sign.
fn number_text() -> impl Strategy<Value = String> {
    "[+-]?[0-9]{0,42}(\\.[0-9]{0,22})?"
}

/// A duration in the ISO-8601 shape, with any mix of parts, numbers near the limits and the odd wrong letter.
fn duration_text() -> impl Strategy<Value = String> {
    let part = (number_text(), "[DHMSdhmsX]?").prop_map(|(n, u)| format!("{n}{u}")).boxed();
    (" ?-?[Pp]?", part.clone(), "[Tt]?", prop::collection::vec(part, 0..4))
        .prop_map(|(start, date, t, time)| format!("{start}{date}{t}{}", time.concat()))
}

/// The number just below where `value * nanos-per-unit` stops fitting in an `i128`. Random digits almost never land
/// here, and an overflow in adding the parts needs several parts at their limit at once.
fn near_limit(nanos_per_unit: i128) -> impl Strategy<Value = i128> {
    (0u32..3, 0i128..1_000).prop_map(move |(shift, below)| ((i128::MAX / nanos_per_unit) >> shift) - below)
}

/// A duration whose parts are all near the limit of the arithmetic, in the valid order.
fn duration_at_the_limit() -> impl Strategy<Value = String> {
    (
        prop::option::of(near_limit(86_400_000_000_000)),
        prop::option::of(near_limit(3_600_000_000_000)),
        prop::option::of(near_limit(60_000_000_000)),
        prop::option::of(near_limit(1_000_000_000)),
    )
        .prop_map(|(d, h, m, s)| {
            let date = d.map(|d| format!("{d}D")).unwrap_or_default();
            let time = [(h, 'H'), (m, 'M'), (s, 'S')]
                .into_iter()
                .filter_map(|(n, unit)| n.map(|n| format!("{n}{unit}")))
                .collect::<String>();
            format!("P{date}T{time}")
        })
}

fn instant_text() -> impl Strategy<Value = String> {
    "[+-]?[0-9]{1,7}-[0-9]{1,3}-[0-9]{1,3}[Tt ][0-9]{1,3}:[0-9]{1,3}:[0-9]{1,3}(\\.[0-9]{0,12})?([Zz]|[+-][0-9]{2}:[0-9]{2})?"
}

fn segment_text() -> impl Strategy<Value = String> {
    "([a-zA-Z0-9._~-]|:|%[0-9a-fA-F]{0,3}|%[cCeEfF][0-9a-fA-F]|é|/){0,12}"
}

fn address_text() -> impl Strategy<Value = String> {
    ("[a-z.-]{0,5}[A-Z0-9_]?", "/?", segment_text()).prop_map(|(t, slash, key)| format!("{t}{slash}{key}"))
}

fn cursor_text() -> impl Strategy<Value = String> {
    (number_text(), ":?", number_text(), ":?", ".{0,6}").prop_map(|(a, c1, b, c2, r)| format!("{a}{c1}{b}{c2}{r}"))
}

/// Duration text that is valid, wrong, or near the limits.
fn duration_json() -> impl Strategy<Value = Value> {
    prop_oneof![
        duration_text().prop_map(Value::from),
        duration_at_the_limit().prop_map(Value::from),
        "PT[0-9]{1,6}S".prop_map(Value::from),
        json_scalar()
    ]
}

fn json_scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        prop::num::f64::NORMAL.prop_map(Value::from),
        ".{0,8}".prop_map(Value::from),
    ]
}

fn json_any() -> impl Strategy<Value = Value> {
    json_scalar().prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::from),
            prop::collection::btree_map("[a-z]{1,4}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect::<Map<_, _>>())),
        ]
    })
}

/// An object made of `fields`, each present or not.
fn object(fields: Vec<(&'static str, BoxedStrategy<Value>)>) -> impl Strategy<Value = Value> {
    fields
        .into_iter()
        .map(|(key, value)| prop::option::of(value).prop_map(move |v| v.map(|v| (key.to_owned(), v))))
        .collect::<Vec<_>>()
        .prop_map(|fields| Value::Object(fields.into_iter().flatten().collect()))
}

fn backoff_json() -> impl Strategy<Value = Value> {
    let number =
        prop_oneof![prop::num::f64::ANY.prop_filter("finite", |f| f.is_finite()).prop_map(Value::from), json_scalar()];
    object(vec![
        ("kind", prop_oneof![Just(json!("fixed")), Just(json!("exponential")), json_scalar()].boxed()),
        ("delay", duration_json().boxed()),
        ("initial", duration_json().boxed()),
        ("max", duration_json().boxed()),
        ("factor", number.boxed()),
    ])
}

fn retry_json() -> impl Strategy<Value = Value> {
    object(vec![
        (
            "maxAttempts",
            prop_oneof![(0u32..10).prop_map(Value::from), any::<u32>().prop_map(Value::from), json_scalar()].boxed(),
        ),
        ("backoff", prop_oneof![backoff_json(), json_scalar()].boxed()),
    ])
}

fn retention_json() -> impl Strategy<Value = Value> {
    let rule = object(vec![
        ("kind", prop_oneof![Just(json!("remove")), Just(json!("for")), Just(json!("last")), json_scalar()].boxed()),
        ("age", duration_json().boxed()),
        ("count", prop_oneof![(0i64..2_000).prop_map(Value::from), json_scalar()].boxed()),
    ])
    .boxed();
    object(vec![
        ("done", prop_oneof![rule.clone(), json_scalar().boxed()].boxed()),
        ("failed", prop_oneof![rule, json_scalar().boxed()].boxed()),
    ])
}

fn send_body() -> impl Strategy<Value = String> {
    object(vec![
        ("id", prop_oneof![Just(json!("m1")), Just(json!(" ")), json_scalar()].boxed()),
        ("type", prop_oneof![Just(json!("Add")), json_scalar()].boxed()),
        ("payload", json_any().boxed()),
        ("deliverAt", prop_oneof![instant_text().prop_map(Value::from), json_scalar()].boxed()),
        ("delay", duration_json().boxed()),
        ("correlationId", json_scalar().boxed()),
        ("headers", json_any().boxed()),
        ("retry", prop_oneof![retry_json().boxed(), json_any().boxed()].boxed()),
        ("retention", prop_oneof![retention_json().boxed(), json_any().boxed()].boxed()),
        ("extra", json_scalar().boxed()),
    ])
    .prop_map(|v| v.to_string())
}

fn bulk_body() -> impl Strategy<Value = String> {
    object(vec![
        ("ids", json_any().boxed()),
        ("prefix", json_scalar().boxed()),
        ("messageType", json_scalar().boxed()),
        ("extra", json_scalar().boxed()),
    ])
    .prop_map(|v| v.to_string())
}

fn decode_all_bodies(text: &str) {
    if let Ok(send) = serde_json::from_str::<SendRequest>(text) {
        let _ = send.delivery();
        if let Some(retry) = send.retry {
            exercise_retry(&retry);
        }
    }
    let _ = serde_json::from_str::<CreateActorRequest>(text);
    if let Ok(bulk) = serde_json::from_str::<BulkRequest>(text) {
        let _ = bulk.selection();
    }
    if let Ok(retry) = serde_json::from_str::<RetryPolicy>(text) {
        exercise_retry(&retry);
    }
    let _ = serde_json::from_str::<Retention>(text);
}

/// An accepted policy is used later, at failure time, where a panic would take down a turn.
fn exercise_retry(policy: &RetryPolicy) {
    for retry in [0, 1, 2, 63, 64, 1_000, u32::MAX] {
        let _ = policy.retries_after(retry);
        let _ = policy.backoff().delay(retry);
    }
}

fn any_instant() -> impl Strategy<Value = Timestamp> {
    (-62_135_596_800_000_000_i64..=time::to_micros(MAX_INSTANT)).prop_map(time::from_micros)
}

fn message_at(deliver_at: Timestamp, seq: i64, to: &ActorAddress) -> crate::store::Message {
    OutgoingMessage::new(to.clone(), "M", json!({})).into_message(deliver_at, seq)
}

fn any_address() -> impl Strategy<Value = ActorAddress> {
    ("[a-z][a-z0-9._-]{0,20}", prop::collection::vec("\\PC{1,8}", 0..4))
        .prop_filter_map("a valid address", |(t, key)| ActorAddress::of(&TypeName::parse(&t).ok()?, key).ok())
}

proptest! {
    #[test]
    fn durations_answer_any_text(text in prop_oneof![duration_text(), duration_at_the_limit(), ".{0,60}"]) {
        let _ = time::parse_duration(&text);
    }

    #[test]
    fn instants_answer_any_text(text in prop_oneof![instant_text(), ".{0,60}"]) {
        let _ = time::parse_instant(&text);
    }

    #[test]
    fn percent_decoding_answers_any_text(text in prop_oneof![segment_text(), ".{0,40}"]) {
        if let Ok(decoded) = percent::decode(&text) {
            let canonical = percent::encode(&decoded);
            prop_assert_eq!(percent::decode(&canonical).unwrap(), decoded);
        }
    }

    #[test]
    fn addresses_answer_any_text(text in prop_oneof![address_text(), ".{0,40}"]) {
        if let Ok(address) = ActorAddress::parse(&text) {
            let _ = address.key_values();
            prop_assert_eq!(ActorAddress::parse(address.as_str()).unwrap(), address);
        }
        let _ = TypeName::parse(&text);
    }

    #[test]
    fn cursors_answer_any_text(text in prop_oneof![cursor_text(), ".{0,40}"]) {
        let cursor = PageCursor(text);
        let _ = decode_mailbox_cursor(&cursor);
        let _ = decode_pending_cursor(&cursor);
        let _ = decode_outcome_cursor(&cursor);
    }

    #[test]
    fn durations_round_trip(secs in prop_oneof![any::<u64>(), 0u64..100_000], nanos in prop_oneof![Just(0u32), 0u32..1_000_000_000]) {
        let duration = Duration::new(secs, nanos);
        prop_assert_eq!(time::parse_duration(&time::format_duration(duration)).unwrap(), duration);
    }

    #[test]
    fn instants_round_trip(instant in any_instant(), nanos in 0u64..1_000) {
        let instant = time::add(instant, Duration::from_nanos(nanos)).unwrap();
        prop_assert_eq!(time::parse_instant(&time::format_instant(instant)).unwrap(), instant);
    }

    #[test]
    fn percent_encoding_round_trips(value in "\\PC{0,16}") {
        prop_assert_eq!(percent::decode(&percent::encode(&value)).unwrap(), value);
    }

    #[test]
    fn addresses_round_trip(address in any_address()) {
        prop_assert_eq!(ActorAddress::parse(address.as_str()).unwrap(), address.clone());
        let again = ActorAddress::of(address.type_name(), address.key_values()).unwrap();
        prop_assert_eq!(again, address);
    }

    #[test]
    fn cursors_round_trip(at in any_instant(), seq in any::<i64>(), to in any_address(), id in "\\PC{1,12}") {
        let message = message_at(at, seq, &to);
        let micros = time::to_micros(at);
        prop_assert_eq!(decode_mailbox_cursor(&mailbox_cursor(&message)).unwrap(), (micros, seq));
        prop_assert_eq!(decode_pending_cursor(&pending_cursor(&message)).unwrap(), (to.to_string(), micros, seq));
        if let Ok(id) = MessageId::new(id) {
            let decoded = decode_outcome_cursor(&outcome_cursor(at, &id)).unwrap();
            prop_assert_eq!(decoded, (micros, id.to_string()));
        }
    }

    #[test]
    fn http_bodies_answer_any_text(text in ".{0,80}") {
        decode_all_bodies(&text);
    }

    #[test]
    fn http_bodies_answer_any_json(value in json_any()) {
        decode_all_bodies(&value.to_string());
    }

    #[test]
    fn http_bodies_answer_near_valid_json(text in prop_oneof![send_body(), bulk_body()]) {
        decode_all_bodies(&text);
    }

    #[test]
    fn policies_answer_near_valid_json(value in prop_oneof![retry_json(), retention_json()]) {
        decode_all_bodies(&value.to_string());
    }
}
