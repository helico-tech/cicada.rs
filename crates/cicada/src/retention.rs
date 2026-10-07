//! Per-message retention: how long a message's record is kept once it has finished.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::store::OutcomeStatus;
use crate::time::{MAX_DELAY, format_duration, serde_duration};

/// How long a message's record is kept once it has finished: part of the message's delivery, set by its **sender**
/// next to the delivery time and retry policy, and stored with the message.
///
/// A rule per outcome: `done` for a message that ends `done`, `failed` for one that ends `failed`. A missing rule, and
/// the outcomes `cancelled` and `purged`, keep the store's default: the record is kept for the store's retention.
///
/// Whatever the rule, the store keeps a minimal record of every message (id, target, type, status, finish time and
/// cause) for its retention period, which is also the dedup window.
///
/// JSON: `{"done": {"kind": "remove"}, "failed": {"kind": "for", "age": "P30D"}}`, with `{"kind": "last", "count":
/// 100}` for [`Keep::Last`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retention {
    /// The rule for a message that ends `done`; `None` for the store default.
    #[serde(default)]
    pub done: Option<Keep>,
    /// The rule for a message that ends `failed`; `None` for the store default.
    #[serde(default)]
    pub failed: Option<Keep>,
}

impl Retention {
    /// Remove the record of a message that ends `done`; keep a failed one as usual.
    pub const REMOVE_ON_DONE: Retention = Retention { done: Some(Keep::Remove), failed: None };

    /// Remove the record of a message that ends `failed`; keep a done one as usual.
    pub const REMOVE_ON_FAILED: Retention = Retention { done: None, failed: Some(Keep::Remove) };

    /// The rule for a message that ended with `status`; `None` for the store default.
    pub fn rule_for(&self, status: OutcomeStatus) -> Option<&Keep> {
        match status {
            OutcomeStatus::Done => self.done.as_ref(),
            OutcomeStatus::Failed => self.failed.as_ref(),
            OutcomeStatus::Cancelled | OutcomeStatus::Purged => None,
        }
    }

    /// The policy's JSON, as stored and served.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a retention policy always serializes")
    }

    /// Decodes a policy read back from storage or from a node, tolerantly: JSON that is not a valid policy is logged as
    /// a warning and read as *no policy*, so the store default applies.
    pub fn from_stored(json: &serde_json::Value, source: &str) -> Option<Self> {
        if json.is_null() {
            return None;
        }
        match serde_json::from_value(json.clone()) {
            Ok(policy) => Some(policy),
            Err(e) => {
                tracing::warn!("Ignoring the retention policy of {source}, which cannot be decoded: {e}");
                None
            }
        }
    }

    /// [`Self::from_stored`] of JSON text.
    pub fn from_stored_text(text: &str, source: &str) -> Option<Self> {
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(json) => Self::from_stored(&json, source),
            Err(e) => {
                tracing::warn!("Ignoring the retention policy of {source}, which is not JSON: {e}");
                None
            }
        }
    }
}

/// One rule of a [`Retention`] policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", try_from = "RawKeep")]
pub enum Keep {
    /// Remove the record as soon as the message finishes.
    Remove,
    /// Keep the record for `age` after the message finished, then remove it.
    For {
        /// Positive, at most [`MAX_DELAY`].
        #[serde(with = "serde_duration")]
        age: Duration,
    },
    /// Keep the record while it is among the `count` most recent records of the same instance and the same outcome
    /// that also chose `Last`, and at most for the store's retention.
    Last {
        /// In `1..=`[`Keep::MAX_COUNT`].
        count: u32,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum RawKeep {
    Remove,
    For {
        #[serde(deserialize_with = "age_field")]
        age: Duration,
    },
    Last {
        count: i64,
    },
}

fn age_field<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    let signed = crate::time::serde_signed_duration::deserialize(deserializer)?;
    Ok(if signed.is_negative() { Duration::ZERO } else { signed.non_negative() })
}

impl TryFrom<RawKeep> for Keep {
    type Error = String;

    fn try_from(raw: RawKeep) -> Result<Self, String> {
        match raw {
            RawKeep::Remove => Ok(Keep::Remove),
            RawKeep::For { age } => Keep::keep_for(age),
            RawKeep::Last { count } => {
                if !(1..=Keep::MAX_COUNT as i64).contains(&count) {
                    return Err(format!("retention count must be in 1..{}, was {count}", Keep::MAX_COUNT));
                }
                Ok(Keep::Last { count: count as u32 })
            }
        }
    }
}

impl Keep {
    /// The largest accepted [`Keep::Last`] count.
    pub const MAX_COUNT: u32 = 10_000;

    /// [`Keep::For`] with a checked age.
    pub fn keep_for(age: Duration) -> Result<Self, String> {
        if age.is_zero() {
            return Err(format!("retention age must be a positive duration, was {}", format_duration(age)));
        }
        if age > MAX_DELAY {
            return Err(format!(
                "retention age must not exceed {}, was {}",
                format_duration(MAX_DELAY),
                format_duration(age)
            ));
        }
        Ok(Keep::For { age })
    }

    /// [`Keep::Last`] with a checked count.
    pub fn last(count: u32) -> Result<Self, String> {
        Keep::try_from(RawKeep::Last { count: count as i64 })
    }
}

impl schemars::JsonSchema for Retention {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Retention".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let keep = generator.subschema_for::<Option<Keep>>();
        schemars::json_schema!({
            "type": "object",
            "properties": { "done": keep, "failed": keep },
        })
    }
}

impl schemars::JsonSchema for Keep {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Keep".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let duration = crate::retry::duration_schema();
        schemars::json_schema!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": { "kind": { "type": "string", "const": "remove" } },
                    "required": ["kind"],
                },
                {
                    "type": "object",
                    "properties": { "kind": { "type": "string", "const": "for" }, "age": duration },
                    "required": ["kind", "age"],
                },
                {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "const": "last" },
                        "count": { "type": "integer", "minimum": 1, "maximum": Keep::MAX_COUNT },
                    },
                    "required": ["kind", "count"],
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn json_round_trip() {
        let r = Retention { done: Some(Keep::Remove), failed: Some(Keep::keep_for(Duration::from_secs(60)).unwrap()) };
        let json = r.to_json();
        assert_eq!(json, json!({"done": {"kind": "remove"}, "failed": {"kind": "for", "age": "PT1M"}}));
        assert_eq!(serde_json::from_value::<Retention>(json).unwrap(), r);
        let last: Retention = serde_json::from_value(json!({"done": {"kind": "last", "count": 3}})).unwrap();
        assert_eq!(last.done, Some(Keep::Last { count: 3 }));
        assert_eq!(serde_json::to_value(&last).unwrap(), json!({"done": {"kind": "last", "count": 3}, "failed": null}));
    }

    #[test]
    fn validates() {
        for bad in [
            json!({"done": {"kind": "last", "count": 0}}),
            json!({"done": {"kind": "last", "count": 10001}}),
            json!({"done": {"kind": "for", "age": "PT0S"}}),
            json!({"done": {"kind": "forever"}}),
        ] {
            assert!(serde_json::from_value::<Retention>(bad.clone()).is_err(), "{bad}");
        }
    }
}
