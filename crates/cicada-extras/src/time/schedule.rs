use std::borrow::Cow;

use cicada::time::Timestamp;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

const DAY: i64 = 86_400;

/// A schedule that cannot exist: a non-positive interval or an invalid time of day.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ScheduleError(String);

/// When a recurring job runs: the next time after a given one. Times are UTC, computed with plain epoch arithmetic.
///
/// Serializable, so it can live in an actor's state. A cron actor's `Tick` handler sends the next tick at
/// `schedule.next(max(turn.now(), tick.scheduled_for))` (from `now`, so slots missed while no node was up are skipped
/// instead of run in a burst).
///
/// On the wire: `{"type":"every","seconds":3600}` or `{"type":"daily","hour":2,"minute":30}`. A decoded schedule is
/// checked like a constructed one.
///
/// ```
/// use cicada_extras::Schedule;
///
/// let hourly = Schedule::every(3600).unwrap();
/// let t = "2026-10-02T10:15:30Z".parse().unwrap();
/// assert_eq!(hourly.next(t), "2026-10-02T11:00:00Z".parse::<cicada::time::Timestamp>().unwrap());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Spec", into = "Spec")]
pub struct Schedule(Spec);

/// The wire form, unchecked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Spec {
    /// Every `seconds`, aligned to multiples of the interval since the epoch.
    Every { seconds: i64 },
    /// Once a day at `hour`:`minute` UTC.
    Daily {
        hour: u32,
        #[serde(default)]
        minute: u32,
    },
}

impl Schedule {
    /// Every `seconds`, aligned to multiples of the interval since the epoch (`3600` = on the hour).
    pub fn every(seconds: i64) -> Result<Self, ScheduleError> {
        Spec::Every { seconds }.try_into()
    }

    /// Once a day at `hour`:`minute` UTC.
    pub fn daily(hour: u32, minute: u32) -> Result<Self, ScheduleError> {
        Spec::Daily { hour, minute }.try_into()
    }

    /// The first scheduled time strictly after `after` (whole seconds).
    pub fn next(&self, after: Timestamp) -> Timestamp {
        let secs = after.timestamp();
        let next = match self.0 {
            Spec::Every { seconds } => (secs.div_euclid(seconds) + 1) * seconds,
            Spec::Daily { hour, minute } => {
                let today = secs.div_euclid(DAY) * DAY + i64::from(hour) * 3600 + i64::from(minute) * 60;
                if today > secs { today } else { today + DAY }
            }
        };
        Timestamp::from_timestamp(next, 0).unwrap_or(cicada::time::MAX_INSTANT)
    }
}

impl TryFrom<Spec> for Schedule {
    type Error = ScheduleError;

    fn try_from(spec: Spec) -> Result<Self, ScheduleError> {
        match spec {
            Spec::Every { seconds } if seconds <= 0 => Err(ScheduleError("seconds must be positive".to_owned())),
            Spec::Daily { hour, minute } if hour > 23 || minute > 59 => {
                Err(ScheduleError(format!("invalid time of day {hour}:{minute}")))
            }
            spec => Ok(Schedule(spec)),
        }
    }
}

impl From<Schedule> for Spec {
    fn from(schedule: Schedule) -> Spec {
        schedule.0
    }
}

impl JsonSchema for Schedule {
    fn schema_name() -> Cow<'static, str> {
        "Schedule".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        Spec::json_schema(generator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn schedules_compute_the_next_slot_in_utc() {
        let now = t("2026-10-02T10:15:30Z");
        assert_eq!(Schedule::every(3600).unwrap().next(now), t("2026-10-02T11:00:00Z"));
        // A slot boundary is not "after" itself.
        assert_eq!(Schedule::every(3600).unwrap().next(t("2026-10-02T11:00:00Z")), t("2026-10-02T12:00:00Z"));
        assert_eq!(Schedule::every(900).unwrap().next(now), t("2026-10-02T10:30:00Z"));
        assert_eq!(Schedule::daily(2, 30).unwrap().next(now), t("2026-10-03T02:30:00Z"));
        assert_eq!(Schedule::daily(23, 0).unwrap().next(now), t("2026-10-02T23:00:00Z"));
        assert_eq!(Schedule::daily(10, 15).unwrap().next(t("2026-10-02T10:15:00Z")), t("2026-10-03T10:15:00Z"));
        // Fractions of a second count as the whole second, as in Kotlin.
        assert_eq!(Schedule::every(60).unwrap().next(t("2026-10-02T10:14:59.900Z")), t("2026-10-02T10:15:00Z"));
    }

    #[test]
    fn invalid_schedules_are_refused() {
        assert_eq!(Schedule::every(0).unwrap_err().to_string(), "seconds must be positive");
        assert_eq!(Schedule::daily(24, 0).unwrap_err().to_string(), "invalid time of day 24:0");
        assert_eq!(Schedule::daily(1, 60).unwrap_err().to_string(), "invalid time of day 1:60");
        assert!(serde_json::from_str::<Schedule>(r#"{"type":"every","seconds":-5}"#).is_err());
    }

    #[test]
    fn schedules_keep_their_wire_form() {
        let every = Schedule::every(60).unwrap();
        assert_eq!(serde_json::to_string(&every).unwrap(), r#"{"type":"every","seconds":60}"#);
        assert_eq!(
            serde_json::from_str::<Schedule>(r#"{"type":"daily","hour":2,"minute":30}"#).unwrap(),
            Schedule::daily(2, 30).unwrap()
        );
        assert_eq!(
            serde_json::from_str::<Schedule>(r#"{"type":"daily","hour":2}"#).unwrap(),
            Schedule::daily(2, 0).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&Schedule::daily(2, 0).unwrap()).unwrap(),
            r#"{"type":"daily","hour":2,"minute":0}"#
        );
    }
}
