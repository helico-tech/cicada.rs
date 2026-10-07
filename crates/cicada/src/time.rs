//! Store time: microsecond instants, bounds on delivery times, and the wire formats of instants and durations.
//!
//! Every instant a store hands out has microsecond precision (the precision of every backend). Delivery times are
//! bounded by [`MAX_INSTANT`], so every backend stores them without overflow. Instants travel as ISO-8601 strings in
//! UTC (`2026-01-01T00:00:00Z`, fractions in groups of three digits); durations as ISO-8601 durations (`PT5S`), and are
//! also accepted in the short form `5s`, `1h 30m` or as a JSON number of milliseconds.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

/// An instant in UTC with nanosecond resolution; stores hand out microsecond instants.
pub type Timestamp = DateTime<Utc>;

/// Default outcome retention (and dedup window): 7 days.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(7 * 24 * 3600);

/// The latest instant a store accepts as a delivery time: `9999-12-31T23:59:59.999999Z`.
pub const MAX_INSTANT: Timestamp = match DateTime::from_timestamp(253_402_300_799, 999_999_000) {
    Some(t) => t,
    None => panic!("MAX_INSTANT is representable"),
};

/// The longest accepted delivery delay, about 1000 years. Longer delays are refused rather than silently overflowing;
/// "never" is not a delivery time.
pub const MAX_DELAY: Duration = Duration::from_secs(365_000 * 24 * 3600);

/// A time value outside what the store supports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TimeError(pub String);

/// Truncates `instant` to whole microseconds (towards the past), the precision every backend can store exactly.
pub fn truncate(instant: Timestamp) -> Timestamp {
    from_micros(to_micros(instant))
}

/// Rounds `instant` up to whole microseconds, so that a requested delivery time is never moved earlier.
pub fn ceil(instant: Timestamp) -> Timestamp {
    let truncated = truncate(instant);
    if truncated == instant { instant } else { from_micros(to_micros(truncated) + 1) }
}

/// Microseconds since the epoch (floored); the integer representation used by SQL backends.
pub fn to_micros(instant: Timestamp) -> i64 {
    instant.timestamp_micros()
}

/// Inverse of [`to_micros`]. Saturates at the bounds jiff can represent.
pub fn from_micros(micros: i64) -> Timestamp {
    DateTime::from_timestamp_micros(micros).unwrap_or(if micros < 0 {
        DateTime::<Utc>::MIN_UTC
    } else {
        DateTime::<Utc>::MAX_UTC
    })
}

/// `instant + duration`, failing beyond what can be represented.
pub fn add(instant: Timestamp, duration: Duration) -> Result<Timestamp, TimeError> {
    TimeDelta::from_std(duration).ok().and_then(|d| instant.checked_add_signed(d)).ok_or_else(|| {
        TimeError(format!("{} + {} is out of range", format_instant(instant), format_duration(duration)))
    })
}

/// `instant - duration`, saturating at the earliest representable instant.
pub fn sub(instant: Timestamp, duration: Duration) -> Timestamp {
    TimeDelta::from_std(duration).ok().and_then(|d| instant.checked_sub_signed(d)).unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// The time from `earlier` to `later`, zero when `later` is not after `earlier`.
pub fn between(earlier: Timestamp, later: Timestamp) -> Duration {
    (later - earlier).to_std().unwrap_or(Duration::ZERO)
}

/// Formats an instant the way the HTTP contract writes it: ISO-8601 in UTC, with the fraction in groups of three
/// digits (milli-, micro- or nanoseconds) and no fraction for whole seconds.
pub fn format_instant(instant: Timestamp) -> String {
    let nanos = instant.timestamp_subsec_nanos() % 1_000_000_000;
    let base = instant.format("%Y-%m-%dT%H:%M:%S").to_string();
    if nanos == 0 {
        return format!("{base}Z");
    }
    let (value, width) = if nanos.is_multiple_of(1_000_000) {
        (nanos / 1_000_000, 3)
    } else if nanos.is_multiple_of(1_000) {
        (nanos / 1_000, 6)
    } else {
        (nanos, 9)
    };
    format!("{base}.{value:0width$}Z")
}

/// Parses an ISO-8601 instant with an offset (`2026-01-01T00:00:00Z`, `2026-01-01T01:00:00+01:00`).
pub fn parse_instant(text: &str) -> Result<Timestamp, TimeError> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| TimeError(format!("'{text}' is not an ISO-8601 instant")))
}

/// Formats a duration as ISO-8601 the way the HTTP contract writes it: hours, minutes and seconds only (`PT48H`,
/// `PT1M30S`, `PT0.500S`), `PT0S` for zero.
pub fn format_duration(duration: Duration) -> String {
    if duration.is_zero() {
        return "PT0S".to_owned();
    }
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    let nanos = duration.subsec_nanos();
    let mut out = String::from("PT");
    if hours > 0 {
        out.push_str(&format!("{hours}H"));
    }
    if minutes > 0 {
        out.push_str(&format!("{minutes}M"));
    }
    if seconds > 0 || nanos > 0 {
        out.push_str(&seconds.to_string());
        if nanos > 0 {
            let digits = format!("{nanos:09}");
            let trimmed = digits.trim_end_matches('0');
            let width = trimmed.len().div_ceil(3) * 3;
            out.push('.');
            out.push_str(&digits[..width]);
        }
        out.push('S');
    }
    out
}

/// A parsed duration, which may be negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedNanos(pub i128);

impl SignedNanos {
    /// The duration, with a negative one counted as zero.
    pub fn non_negative(self) -> Duration {
        if self.0 <= 0 { Duration::ZERO } else { Duration::from_nanos(self.0.min(u64::MAX as i128) as u64) }
    }

    /// True when negative.
    pub fn is_negative(self) -> bool {
        self.0 < 0
    }
}

/// Parses a duration: ISO-8601 (`PT5S`, `P1DT2H`, `-PT1M`; a day is 24 hours) or the short form (`5s`, `1h 30m`,
/// `1.5s`, `500ms`, `2d`, `-(1h 30m)`).
pub fn parse_duration(text: &str) -> Result<SignedNanos, TimeError> {
    let err = || TimeError(format!("'{text}' is not an ISO-8601 duration such as PT5S"));
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(err());
    }
    let (negative, body) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let nanos = if body.starts_with('P') || body.starts_with('p') {
        parse_iso(&body[1..]).ok_or_else(err)?
    } else {
        let body =
            if negative { body.strip_prefix('(').and_then(|b| b.strip_suffix(')')).unwrap_or(body) } else { body };
        parse_short(body).ok_or_else(err)?
    };
    Ok(SignedNanos(if negative { -nanos } else { nanos }))
}

const NANOS_PER: [(char, i128); 4] =
    [('D', 86_400_000_000_000), ('H', 3_600_000_000_000), ('M', 60_000_000_000), ('S', 1_000_000_000)];

fn parse_iso(body: &str) -> Option<i128> {
    let (date, time) = match body.find(['T', 't']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let mut total: i128 = 0;
    let mut any = false;
    if !date.is_empty() {
        let (n, rest) = number_then_unit(date)?;
        if !rest.eq_ignore_ascii_case("D") {
            return None;
        }
        total += scale(n, NANOS_PER[0].1)?;
        any = true;
    }
    if let Some(mut time) = time {
        if time.is_empty() {
            return None;
        }
        let mut last = 0;
        while !time.is_empty() {
            let (n, rest) = number_then_unit(time)?;
            let unit = rest.chars().next()?.to_ascii_uppercase();
            let index = NANOS_PER.iter().position(|(u, _)| *u == unit).filter(|i| *i >= 1)?;
            if index <= last && last != 0 {
                return None;
            }
            last = index;
            total += scale(n, NANOS_PER[index].1)?;
            time = &rest[1..];
            any = true;
        }
    }
    any.then_some(total)
}

fn parse_short(body: &str) -> Option<i128> {
    let mut total: i128 = 0;
    let mut any = false;
    for part in body.split_whitespace() {
        let (n, unit) = number_then_unit(part)?;
        let factor = match unit {
            "d" => 86_400_000_000_000,
            "h" => 3_600_000_000_000,
            "m" => 60_000_000_000,
            "s" => 1_000_000_000,
            "ms" => 1_000_000,
            "us" => 1_000,
            "ns" => 1,
            _ => return None,
        };
        total += scale(n, factor)?;
        any = true;
    }
    any.then_some(total)
}

/// A decimal number (`12`, `1.5`, `-3`) followed by the rest of the text.
fn number_then_unit(text: &str) -> Option<(&str, &str)> {
    let end = text
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))))
        .map(|(i, _)| i)?;
    if end == 0 {
        return None;
    }
    Some((&text[..end], &text[end..]))
}

/// `number` (decimal text) times `factor` nanoseconds, rounded towards zero.
fn scale(number: &str, factor: i128) -> Option<i128> {
    let (negative, digits) = match number.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, number.strip_prefix('+').unwrap_or(number)),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((w, f)) => (w, f),
        None => (digits, ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let whole: i128 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let mut value = whole.checked_mul(factor)?;
    if !fraction.is_empty() {
        if !fraction.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut divisor: i128 = 1;
        let mut numerator: i128 = 0;
        for c in fraction.chars().take(18) {
            numerator = numerator * 10 + c.to_digit(10)? as i128;
            divisor *= 10;
        }
        value = value.checked_add(numerator.checked_mul(factor)? / divisor)?;
    }
    Some(if negative { -value } else { value })
}

/// Serde for an instant as its ISO-8601 string (see [`format_instant`]).
pub mod serde_instant {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{Timestamp, format_instant, parse_instant};

    /// Serializes as ISO-8601.
    pub fn serialize<S: Serializer>(value: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format_instant(*value))
    }

    /// Deserializes from ISO-8601.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_instant(&text).map_err(serde::de::Error::custom)
    }

    /// The same for `Option<Timestamp>`.
    pub mod option {
        use serde::{Deserialize, Deserializer, Serializer};

        use super::super::{Timestamp, format_instant, parse_instant};

        /// Serializes as ISO-8601 or null.
        pub fn serialize<S: Serializer>(value: &Option<Timestamp>, serializer: S) -> Result<S::Ok, S::Error> {
            match value {
                Some(t) => serializer.serialize_str(&format_instant(*t)),
                None => serializer.serialize_none(),
            }
        }

        /// Deserializes from ISO-8601 or null.
        pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Timestamp>, D::Error> {
            match Option::<String>::deserialize(deserializer)? {
                Some(text) => parse_instant(&text).map(Some).map_err(serde::de::Error::custom),
                None => Ok(None),
            }
        }
    }
}

/// Serde for a non-negative duration: written as ISO-8601, read from ISO-8601, the short form or a JSON number of
/// milliseconds. A negative value is refused.
pub mod serde_duration {
    use std::time::Duration;

    use serde::{Deserializer, Serializer};

    use super::format_duration;

    /// Serializes as ISO-8601.
    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format_duration(*value))
    }

    /// Deserializes from a duration string or milliseconds; refuses negative values.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let signed = super::serde_signed_duration::deserialize(deserializer)?;
        if signed.is_negative() {
            return Err(serde::de::Error::custom("a duration must not be negative"));
        }
        Ok(signed.non_negative())
    }
}

/// Serde for a duration that may be negative (a send's `delay`): read from a duration string or milliseconds.
pub mod serde_signed_duration {
    use serde::Deserializer;
    use serde::de::{self, Visitor};

    use super::{SignedNanos, parse_duration};

    /// Deserializes from a duration string or a whole number of milliseconds.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SignedNanos, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = SignedNanos;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an ISO-8601 duration string such as PT5S or a number of milliseconds")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<SignedNanos, E> {
                parse_duration(v).map_err(E::custom)
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<SignedNanos, E> {
                Ok(SignedNanos(v as i128 * 1_000_000))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<SignedNanos, E> {
                Ok(SignedNanos(v as i128 * 1_000_000))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<SignedNanos, E> {
                if v.fract() == 0.0 && v.is_finite() && v.abs() < 9.0e15 {
                    Ok(SignedNanos(v as i128 * 1_000_000))
                } else {
                    Err(E::custom(format!("must be a whole number of milliseconds: {v}")))
                }
            }
        }
        deserializer.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_instants_like_the_contract() {
        let t: Timestamp = parse_instant("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(format_instant(t), "2026-01-01T00:00:00Z");
        assert_eq!(format_instant(from_micros(to_micros(t) + 1_000)), "2026-01-01T00:00:00.001Z");
        assert_eq!(format_instant(from_micros(to_micros(t) + 1)), "2026-01-01T00:00:00.000001Z");
        assert_eq!(format_instant(MAX_INSTANT), "9999-12-31T23:59:59.999999Z");
        assert_eq!(parse_instant("2026-01-01T01:00:00+01:00").unwrap(), t);
    }

    #[test]
    fn chrono_serde_writes_the_contract_format() {
        // States may hold plain `Timestamp` fields: chrono's serde form equals `format_instant`.
        for micros in [0, 1_000, 1, 1_767_225_600_000_000, 1_767_225_600_123_456] {
            let t = from_micros(micros);
            assert_eq!(serde_json::to_value(t).unwrap(), serde_json::Value::String(format_instant(t)));
        }
    }

    #[test]
    fn truncates_and_ceils() {
        let t = DateTime::from_timestamp(10, 1_500).unwrap();
        assert_eq!(truncate(t), DateTime::from_timestamp(10, 1_000).unwrap());
        assert_eq!(ceil(t), DateTime::from_timestamp(10, 2_000).unwrap());
        let negative = DateTime::from_timestamp(-1, 999_999_500).unwrap();
        assert!(truncate(negative) <= negative);
    }

    #[test]
    fn formats_durations_like_kotlin() {
        assert_eq!(format_duration(Duration::ZERO), "PT0S");
        assert_eq!(format_duration(Duration::from_secs(1)), "PT1S");
        assert_eq!(format_duration(Duration::from_secs(60)), "PT1M");
        assert_eq!(format_duration(Duration::from_secs(90)), "PT1M30S");
        assert_eq!(format_duration(Duration::from_secs(48 * 3600)), "PT48H");
        assert_eq!(format_duration(Duration::from_millis(500)), "PT0.500S");
        assert_eq!(format_duration(Duration::from_millis(1_500)), "PT1.500S");
        assert_eq!(format_duration(Duration::from_nanos(1_000_000_001)), "PT1.000000001S");
    }

    #[test]
    fn parses_durations() {
        let s = |t: &str| parse_duration(t).unwrap().0;
        assert_eq!(s("PT5S"), 5_000_000_000);
        assert_eq!(s("P1D"), 86_400_000_000_000);
        assert_eq!(s("P1DT2H"), 93_600_000_000_000);
        assert_eq!(s("PT1.5S"), 1_500_000_000);
        assert_eq!(s("-PT1M"), -60_000_000_000);
        assert_eq!(s("5s"), 5_000_000_000);
        assert_eq!(s("1h 30m"), 5_400_000_000_000);
        assert_eq!(s("500ms"), 500_000_000);
        assert_eq!(s("2d"), 172_800_000_000_000);
        assert_eq!(s("-(1h 30m)"), -5_400_000_000_000);
        for bad in ["", "P", "PT", "5", "5x", "PTS", "P1H", "PT1S1M", "abc"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }
}
