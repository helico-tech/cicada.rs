//! Decoding of the request fields that need more than serde's derive: the strict `retry` and `retention` policies (an
//! unknown key at any depth is an error), durations that name their field in errors, and the lenient policies of
//! served envelopes.

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serializer};
use serde_json::{Map, Value};

use crate::retention::{Keep, Retention};
use crate::retry::{Backoff, RetryPolicy};
use crate::time::{MAX_DELAY, SignedNanos, format_duration, parse_duration};

const SOURCE: &str = "a served message";

pub(super) fn serialize_delay<S: Serializer>(value: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(d) => serializer.serialize_str(&format_duration(*d)),
        None => serializer.serialize_none(),
    }
}

/// A send's `delay`: a duration string or milliseconds; negative reads as zero ("now").
pub(super) fn delay<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Duration>, D::Error> {
    let Some(value) = Option::<Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let signed = duration_field("delay", &value).map_err(serde::de::Error::custom)?;
    if signed.is_negative() {
        return Ok(Some(Duration::ZERO));
    }
    to_duration(signed)
        .map(Some)
        .ok_or_else(|| serde::de::Error::custom(format!("A delay must not exceed {}", format_duration(MAX_DELAY))))
}

pub(super) fn retry<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<RetryPolicy>, D::Error> {
    match Option::<Value>::deserialize(deserializer)? {
        None => Ok(None),
        Some(value) => retry_policy(&value).map(Some).map_err(serde::de::Error::custom),
    }
}

pub(super) fn retention<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Retention>, D::Error> {
    match Option::<Value>::deserialize(deserializer)? {
        None => Ok(None),
        Some(value) => retention_policy(&value).map(Some).map_err(serde::de::Error::custom),
    }
}

pub(super) fn lenient_retry<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<RetryPolicy>, D::Error> {
    Ok(Option::<Value>::deserialize(deserializer)?.and_then(|v| RetryPolicy::from_stored(&v, SOURCE)))
}

pub(super) fn lenient_retention<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Retention>, D::Error> {
    Ok(Option::<Value>::deserialize(deserializer)?.and_then(|v| Retention::from_stored(&v, SOURCE)))
}

/// A duration field named `field` (in errors): an ISO-8601 or short-form duration string, or whole milliseconds.
fn duration_field(field: &str, value: &Value) -> Result<SignedNanos, String> {
    match value {
        Value::String(text) => {
            parse_duration(text).map_err(|_| format!("{field} '{text}' is not an ISO-8601 duration such as PT5S"))
        }
        Value::Number(n) => {
            let millis = n.as_i64().map(i128::from).or_else(|| n.as_u64().map(i128::from));
            millis
                .map(|ms| SignedNanos(ms * 1_000_000))
                .ok_or_else(|| format!("{field} must be a whole number of milliseconds: {n}"))
        }
        _ => Err(format!("{field} must be a duration string or milliseconds")),
    }
}

/// A non-negative duration, or `None` when it does not fit.
fn to_duration(signed: SignedNanos) -> Option<Duration> {
    let nanos = u128::try_from(signed.0).ok()?;
    let secs = u64::try_from(nanos / 1_000_000_000).ok()?;
    Some(Duration::new(secs, (nanos % 1_000_000_000) as u32))
}

/// A policy duration: negative reads as zero and an overflowing one as the largest, so the policy's bounds check
/// reports either.
fn policy_duration(field: &str, value: Option<&Value>, what: &str) -> Result<Duration, String> {
    let value = value.ok_or_else(|| format!("{what} needs {field}"))?;
    let signed = duration_field(field, value)?;
    if signed.is_negative() {
        return Ok(Duration::ZERO);
    }
    Ok(to_duration(signed).unwrap_or(Duration::MAX))
}

fn object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value.as_object().ok_or_else(|| format!("{what} must be a JSON object, was {value}"))
}

fn only_keys(object: &Map<String, Value>, what: &str, allowed: &[&str]) -> Result<(), String> {
    match object.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(key) => Err(format!("Unknown key '{key}' in {what}; expected {}", allowed.join(", "))),
        None => Ok(()),
    }
}

fn retry_policy(value: &Value) -> Result<RetryPolicy, String> {
    let o = object(value, "retry")?;
    only_keys(o, "retry", &["maxAttempts", "backoff"])?;
    let max = o.get("maxAttempts").ok_or("retry needs maxAttempts")?;
    let max = max.as_i64().ok_or_else(|| format!("retry maxAttempts must be an integer, was {max}"))?;
    if !(1..=i64::from(RetryPolicy::MAX_ATTEMPTS)).contains(&max) {
        return Err(format!("retry maxAttempts must be in 1..{}, was {max}", RetryPolicy::MAX_ATTEMPTS));
    }
    let backoff = backoff(o.get("backoff").ok_or("retry needs a backoff")?)?;
    RetryPolicy::new(max as u32, backoff)
}

fn backoff(value: &Value) -> Result<Backoff, String> {
    let o = object(value, "retry backoff")?;
    match o.get("kind").and_then(Value::as_str) {
        Some("exponential") => {
            only_keys(o, "exponential backoff", &["kind", "initial", "factor", "max"])?;
            let initial = policy_duration("backoff.initial", o.get("initial"), "An exponential backoff")?;
            let factor = match o.get("factor") {
                None | Some(Value::Null) => 2.0,
                Some(f) => f.as_f64().ok_or_else(|| format!("backoff.factor must be a number, was {f}"))?,
            };
            let max = policy_duration("backoff.max", o.get("max"), "An exponential backoff")?;
            Backoff::exponential(initial, factor, max)
        }
        Some("fixed") => {
            only_keys(o, "fixed backoff", &["kind", "delay"])?;
            Backoff::fixed(policy_duration("backoff.delay", o.get("delay"), "A fixed backoff")?)
        }
        Some(other) => Err(format!("Unknown backoff kind '{other}'; expected exponential or fixed")),
        None => Err("A retry backoff needs a kind: exponential or fixed".to_owned()),
    }
}

fn retention_policy(value: &Value) -> Result<Retention, String> {
    let o = object(value, "retention")?;
    only_keys(o, "retention", &["done", "failed"])?;
    let rule = |name: &str| match o.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => keep(name, v).map(Some),
    };
    Ok(Retention { done: rule("done")?, failed: rule("failed")? })
}

fn keep(name: &str, value: &Value) -> Result<Keep, String> {
    let what = format!("retention {name}");
    let o = object(value, &what)?;
    match o.get("kind").and_then(Value::as_str) {
        Some("remove") => {
            only_keys(o, &what, &["kind"])?;
            Ok(Keep::Remove)
        }
        Some("for") => {
            only_keys(o, &what, &["kind", "age"])?;
            Keep::keep_for(policy_duration("age", o.get("age"), &format!("A {what} rule of kind for"))?)
        }
        Some("last") => {
            only_keys(o, &what, &["kind", "count"])?;
            let count = o.get("count").ok_or_else(|| format!("A {what} rule of kind last needs count"))?;
            let count = count.as_i64().ok_or_else(|| format!("retention count must be an integer, was {count}"))?;
            if !(1..=i64::from(Keep::MAX_COUNT)).contains(&count) {
                return Err(format!("retention count must be in 1..{}, was {count}", Keep::MAX_COUNT));
            }
            Keep::last(count as u32)
        }
        Some(other) => Err(format!("Unknown retention kind '{other}'; expected remove, for or last")),
        None => Err(format!("A {what} rule needs a kind: remove, for or last")),
    }
}
