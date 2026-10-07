//! Per-message retry: part of a message's delivery, set by its sender.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::time::{MAX_DELAY, format_duration, serde_duration};

/// How the runtime retries one message: part of the message's delivery, set by its **sender** next to the delivery
/// time, and stored with the message. Without a policy nothing is retried: a failed handler run ends the message
/// `failed`.
///
/// When a handler run of a message with a policy fails (the handler returns an error, panics or exceeds the type's
/// maximum turn), the failure is not permanent and attempts are left (`failures + 1 < max_attempts`), the dispatcher
/// puts the *same* message back into the mailbox, due after [`Backoff::delay`] of the number of failures so far: same
/// id, `failures` + 1, `lastError` set, no outcome in between. Otherwise the message ends `failed` with the handler's
/// error.
///
/// JSON (HTTP and storage):
/// `{"maxAttempts": 4, "backoff": {"kind": "exponential", "initial": "PT1S", "factor": 2.0, "max": "PT1M"}}` or
/// `{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT5S"}}`.
///
/// There is no jitter: delays are deterministic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawRetryPolicy")]
pub struct RetryPolicy {
    /// Handler runs of the message, including the first, in `1..=`[`Self::MAX_ATTEMPTS`]; 1 means no retry.
    max_attempts: u32,
    /// The delay before each retry.
    backoff: Backoff,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRetryPolicy {
    max_attempts: i64,
    backoff: Backoff,
}

impl TryFrom<RawRetryPolicy> for RetryPolicy {
    type Error = String;

    fn try_from(raw: RawRetryPolicy) -> Result<Self, String> {
        if !(1..=Self::MAX_ATTEMPTS as i64).contains(&raw.max_attempts) {
            return Err(format!(
                "retry maxAttempts must be in 1..{}, was {}",
                Self::MAX_ATTEMPTS,
                raw.max_attempts
            ));
        }
        Ok(Self { max_attempts: raw.max_attempts as u32, backoff: raw.backoff })
    }
}

impl RetryPolicy {
    /// The largest accepted `max_attempts`.
    pub const MAX_ATTEMPTS: u32 = 100;

    /// A policy of `max_attempts` handler runs with `backoff`; fails when `max_attempts` is outside
    /// `1..=`[`Self::MAX_ATTEMPTS`].
    pub fn new(max_attempts: u32, backoff: Backoff) -> Result<Self, String> {
        RetryPolicy::try_from(RawRetryPolicy { max_attempts: max_attempts as i64, backoff })
    }

    /// Handler runs of the message, including the first.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// The delay before each retry.
    pub fn backoff(&self) -> &Backoff {
        &self.backoff
    }

    /// Whether a message whose handler has now failed `failures` times in total (including the run that just failed)
    /// gets another attempt.
    pub fn retries_after(&self, failures: u32) -> bool {
        failures < self.max_attempts
    }

    /// Decodes a policy read back from storage or from a node, tolerantly: JSON that is not a valid policy (a backoff
    /// kind written by a newer version, a damaged row) is logged as a warning and read as *no policy*. `source` names
    /// the value in the warning.
    pub fn from_stored(json: &serde_json::Value, source: &str) -> Option<Self> {
        if json.is_null() {
            return None;
        }
        match serde_json::from_value(json.clone()) {
            Ok(policy) => Some(policy),
            Err(e) => {
                tracing::warn!("Ignoring the retry policy of {source}, which cannot be decoded: {e}");
                None
            }
        }
    }

    /// [`Self::from_stored`] of JSON text; text that is not JSON at all reads as no policy too.
    pub fn from_stored_text(text: &str, source: &str) -> Option<Self> {
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(json) => Self::from_stored(&json, source),
            Err(e) => {
                tracing::warn!("Ignoring the retry policy of {source}, which is not JSON: {e}");
                None
            }
        }
    }

    /// The policy's JSON, as stored and served.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a retry policy always serializes")
    }
}

/// The delay before each retry of a [`RetryPolicy`]: [`Backoff::Exponential`] or [`Backoff::Fixed`]. Build one with
/// [`Backoff::exponential`] or [`Backoff::fixed`], which validate it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", try_from = "RawBackoff")]
pub enum Backoff {
    /// `initial * factor^(retry - 1)`, capped at `max`: with factor 2, 1 s, 2 s, 4 s, ….
    Exponential {
        /// The first delay; positive.
        #[serde(with = "serde_duration")]
        initial: Duration,
        /// Growth per retry; finite and at least 1.
        factor: f64,
        /// The longest delay; at least `initial` and at most [`MAX_DELAY`].
        #[serde(with = "serde_duration")]
        max: Duration,
    },
    /// The same delay before every retry.
    Fixed {
        /// Positive, at most [`MAX_DELAY`].
        #[serde(with = "serde_duration")]
        delay: Duration,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum RawBackoff {
    Exponential {
        #[serde(deserialize_with = "field_duration")]
        initial: Duration,
        #[serde(default = "default_factor")]
        factor: f64,
        #[serde(deserialize_with = "field_duration")]
        max: Duration,
    },
    Fixed {
        #[serde(deserialize_with = "field_duration")]
        delay: Duration,
    },
}

/// A duration field that may be zero or negative in the JSON, so that the bounds check reports it properly.
fn field_duration<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    let signed = crate::time::serde_signed_duration::deserialize(deserializer)?;
    if signed.is_negative() {
        // Reported as "must be a positive duration" by the bounds check.
        return Ok(Duration::ZERO);
    }
    Ok(signed.non_negative())
}

fn default_factor() -> f64 {
    2.0
}

impl TryFrom<RawBackoff> for Backoff {
    type Error = String;

    fn try_from(raw: RawBackoff) -> Result<Self, String> {
        match raw {
            RawBackoff::Exponential { initial, factor, max } => Backoff::exponential(initial, factor, max),
            RawBackoff::Fixed { delay } => Backoff::fixed(delay),
        }
    }
}

fn check_delay(name: &str, value: Duration) -> Result<(), String> {
    if value.is_zero() {
        return Err(format!("backoff {name} must be a positive duration, was {}", format_duration(value)));
    }
    if value > MAX_DELAY {
        return Err(format!(
            "backoff {name} must not exceed {}, was {}",
            format_duration(MAX_DELAY),
            format_duration(value)
        ));
    }
    Ok(())
}

impl Backoff {
    /// An exponential backoff; fails when a bound is violated.
    pub fn exponential(initial: Duration, factor: f64, max: Duration) -> Result<Self, String> {
        check_delay("initial", initial)?;
        check_delay("max", max)?;
        if !(factor.is_finite() && factor >= 1.0) {
            return Err(format!("backoff factor must be a finite number >= 1, was {factor}"));
        }
        if max < initial {
            return Err(format!(
                "backoff max ({}) must not be shorter than initial ({})",
                format_duration(max),
                format_duration(initial)
            ));
        }
        Ok(Backoff::Exponential { initial, factor, max })
    }

    /// A fixed backoff; fails when `delay` is zero or longer than [`MAX_DELAY`].
    pub fn fixed(delay: Duration) -> Result<Self, String> {
        check_delay("delay", delay)?;
        Ok(Backoff::Fixed { delay })
    }

    /// The delay before retry number `retry`: 1 for the first retry (after the first run failed), and so on; it
    /// equals the message's `failures` once the failure is recorded. Never longer than [`MAX_DELAY`].
    pub fn delay(&self, retry: u32) -> Duration {
        let retry = retry.max(1);
        match self {
            Backoff::Fixed { delay } => *delay,
            Backoff::Exponential { initial, factor, max } => {
                let raw = initial.as_secs_f64() * factor.powi((retry - 1).min(i32::MAX as u32) as i32);
                if !raw.is_finite() || raw >= max.as_secs_f64() {
                    *max
                } else {
                    Duration::from_secs_f64(raw).min(*max)
                }
            }
        }
        .min(MAX_DELAY)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn json_round_trip() {
        let policy = RetryPolicy::new(
            4,
            Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(60)).unwrap(),
        )
        .unwrap();
        let json = policy.to_json();
        assert_eq!(
            json,
            json!({"maxAttempts": 4, "backoff": {"kind": "exponential", "initial": "PT1S", "factor": 2.0, "max": "PT1M"}})
        );
        assert_eq!(serde_json::from_value::<RetryPolicy>(json).unwrap(), policy);
        let fixed: RetryPolicy =
            serde_json::from_value(json!({"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "5s"}})).unwrap();
        assert_eq!(fixed.backoff().delay(1), Duration::from_secs(5));
        let millis: RetryPolicy =
            serde_json::from_value(json!({"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": 250}})).unwrap();
        assert_eq!(millis.backoff().delay(3), Duration::from_millis(250));
    }

    #[test]
    fn validates() {
        for bad in [
            json!({"maxAttempts": 0, "backoff": {"kind": "fixed", "delay": "PT1S"}}),
            json!({"maxAttempts": 101, "backoff": {"kind": "fixed", "delay": "PT1S"}}),
            json!({"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT0S"}}),
            json!({"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "-PT1S"}}),
            json!({"maxAttempts": 2, "backoff": {"kind": "exponential", "initial": "PT2S", "max": "PT1S"}}),
            json!({"maxAttempts": 2, "backoff": {"kind": "exponential", "initial": "PT1S", "factor": 0.5, "max": "PT1S"}}),
            json!({"maxAttempts": 2, "backoff": {"kind": "linear", "delay": "PT1S"}}),
        ] {
            assert!(serde_json::from_value::<RetryPolicy>(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn exponential_delays() {
        let b = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(5)).unwrap();
        let delays: Vec<u64> = (1..=5).map(|r| b.delay(r).as_secs()).collect();
        assert_eq!(delays, vec![1, 2, 4, 5, 5]);
        assert_eq!(b.delay(10_000), Duration::from_secs(5));
    }

    #[test]
    fn stored_policies_are_tolerant() {
        assert!(RetryPolicy::from_stored(&json!({"maxAttempts": 2, "backoff": {"kind": "linear"}}), "t").is_none());
        assert!(RetryPolicy::from_stored_text("not json", "t").is_none());
        assert!(RetryPolicy::from_stored(&serde_json::Value::Null, "t").is_none());
    }
}
