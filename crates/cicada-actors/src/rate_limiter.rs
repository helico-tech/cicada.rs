//! The rate limiter: an actor that sends each submitted message to its target at the next free slot of a rate.

use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp, format_duration, serde_duration};
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};

use crate::{EncodeError, StoredMessage};

/// A rate limiter, `cicada.rate-limiter/<name>`: one for each limit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RateLimiter {
    /// The name of the limit: the key of its address.
    pub name: String,
}

impl RateLimiter {
    /// The rate limiter named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for RateLimiter {
    const TYPE: &'static str = "cicada.rate-limiter";
    type Message = RateLimiterMsg;
}

/// The messages of a [`RateLimiter`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum RateLimiterMsg {
    /// Reserves the next free slot and sends the message to the target, due at that slot, with the limiter as its
    /// sender, the correlation id of the submission, and `retry` as its retry policy. Refused for good, reserving
    /// nothing, when the slot is more than the longest delay away or the target does not take the message.
    Submit {
        /// The target.
        to: ActorAddress,
        /// The message for the target.
        message: StoredMessage,
        /// The retry policy of the message at the target; none: one attempt. A retry there does not pass the limiter
        /// again.
        #[serde(default)]
        retry: Option<RetryPolicy>,
    },
}

impl RateLimiterMsg {
    /// A `Submit` of `message` for `to`, with `retry` as the retry policy at the target: the compiler checks that `to`
    /// takes the message, as for a send. Fails for a ref without an address, and for a value that is not a message
    /// family.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Mailer;
    /// # impl ActorRef for Mailer { const TYPE: &'static str = "mailer"; type Message = MailerMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum MailerMsg { Send { to: String } }
    /// use cicada_actors::RateLimiterMsg;
    ///
    /// let RateLimiterMsg::Submit { to, message, retry } =
    ///     RateLimiterMsg::submit(&Mailer, MailerMsg::Send { to: "a@b.c".into() }, None)?;
    /// assert_eq!((to.as_str(), message.message_type.as_str(), retry), ("mailer/_", "Send", None));
    /// # Ok::<(), cicada_actors::EncodeError>(())
    /// ```
    ///
    /// A message of another family does not compile:
    ///
    /// ```compile_fail
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Mailer;
    /// # impl ActorRef for Mailer { const TYPE: &'static str = "mailer"; type Message = MailerMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum MailerMsg { Send { to: String } }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum ReportsMsg { Build { full: bool } }
    /// use cicada_actors::RateLimiterMsg;
    ///
    /// let wrong = RateLimiterMsg::submit(&Mailer, ReportsMsg::Build { full: true }, None);
    /// ```
    pub fn submit<R: ActorRef>(
        to: &R,
        message: impl Into<R::Message>,
        retry: Option<RetryPolicy>,
    ) -> Result<Self, EncodeError> {
        let address = to.address().map_err(|e| EncodeError(e.to_string()))?;
        Ok(RateLimiterMsg::Submit { to: address, message: StoredMessage::for_ref(to, message)?, retry })
    }
}

/// Settings of a rate limiter out of their bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidRate(String);

/// The state of a [`RateLimiter`]: its rate, burst and longest delay, fixed at its create, and when its full burst is
/// free again. Build it with `RateLimiterState::new`; a decoded state is checked the same way.
///
/// On the wire: `{"count":2,"period":"PT1S","burst":2,"maxDelay":"PT1S"}`, with `idleAt` once it forwarded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct RateLimiterState {
    /// The messages per period.
    pub count: u32,
    /// The period of the rate.
    #[serde(serialize_with = "serde_duration::serialize")]
    pub period: Duration,
    /// The messages that may be due at once after enough idle time, the first included; 1 spaces every message.
    pub burst: u32,
    /// The longest time from handling a submission to its slot; zero takes only capacity that is free now.
    #[serde(serialize_with = "serde_duration::serialize")]
    pub max_delay: Duration,
    /// When the full burst is free again; none for a limiter that has all of it.
    pub idle_at: Option<Timestamp>,
}

/// The wire form of a [`RateLimiterState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RawState {
    /// The messages per period; positive.
    #[schemars(range(min = 1))]
    count: u32,
    /// The period of the rate, as an ISO-8601 duration; positive and at most `MAX_DELAY`.
    #[serde(deserialize_with = "serde_duration::deserialize")]
    #[schemars(with = "String")]
    period: Duration,
    /// The messages that may be due at once after enough idle time, the first included; positive. 1 spaces every
    /// message.
    #[schemars(range(min = 1))]
    burst: u32,
    /// The longest time from handling a submission to its slot, as an ISO-8601 duration, at most `MAX_DELAY`; zero
    /// takes only capacity that is free now. A submission whose slot is further away is refused for good.
    #[serde(deserialize_with = "serde_duration::deserialize")]
    #[schemars(with = "String")]
    max_delay: Duration,
    /// When the full burst is free again; absent or null for a limiter that has all of it.
    #[serde(default)]
    idle_at: Option<Timestamp>,
}

impl TryFrom<RawState> for RateLimiterState {
    type Error = InvalidRate;

    fn try_from(raw: RawState) -> Result<Self, InvalidRate> {
        let state = Self {
            count: raw.count,
            period: raw.period,
            burst: raw.burst,
            max_delay: raw.max_delay,
            idle_at: raw.idle_at,
        };
        state.bounds()?;
        Ok(state)
    }
}

impl RateLimiterState {
    /// A limiter of `count` messages per `period`, `burst` of them at once, that refuses a message whose slot is more
    /// than `max_delay` away; it has its full burst. Fails when `count` or `burst` is zero, `period` is zero or longer
    /// than `MAX_DELAY`, `max_delay` is longer than `MAX_DELAY`, or the burst spans more than `MAX_DELAY`.
    ///
    /// ```
    /// use std::time::Duration;
    /// use cicada_actors::RateLimiterState;
    ///
    /// let state = RateLimiterState::new(2, Duration::from_secs(1), 2, Duration::from_secs(1))?;
    /// assert_eq!(serde_json::to_value(&state)?, serde_json::json!(
    ///     {"count": 2, "period": "PT1S", "burst": 2, "maxDelay": "PT1S", "idleAt": null}));
    /// assert!(RateLimiterState::new(0, Duration::from_secs(1), 1, Duration::ZERO).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(count: u32, period: Duration, burst: u32, max_delay: Duration) -> Result<Self, InvalidRate> {
        let state = Self { count, period, burst, max_delay, idle_at: None };
        state.bounds()?;
        Ok(state)
    }

    /// Checks the settings and the timestamp; returns the interval between slots and the burst allowance, in
    /// microseconds.
    fn bounds(&self) -> Result<(i64, i64), InvalidRate> {
        if self.count == 0 {
            return Err(InvalidRate("count must be positive".to_owned()));
        }
        if self.burst == 0 {
            return Err(InvalidRate("burst must be positive".to_owned()));
        }
        if self.period.is_zero() || self.period > MAX_DELAY {
            return Err(InvalidRate(format!(
                "period must be positive and at most {}, was {}",
                format_duration(MAX_DELAY),
                format_duration(self.period)
            )));
        }
        if self.max_delay > MAX_DELAY {
            return Err(InvalidRate(format!(
                "maxDelay must not exceed {}, was {}",
                format_duration(MAX_DELAY),
                format_duration(self.max_delay)
            )));
        }
        // The period is at most MAX_DELAY, so the interval fits an i64 of microseconds.
        let interval = self.period.as_nanos().div_ceil(u128::from(self.count) * 1_000);
        let tau = interval.checked_mul(u128::from(self.burst - 1)).filter(|tau| *tau <= MAX_DELAY.as_micros());
        let (Ok(interval), Some(Ok(tau))) = (i64::try_from(interval), tau.map(i64::try_from)) else {
            return Err(InvalidRate(format!(
                "the burst of {} at {} per {} spans more than {}",
                self.burst,
                self.count,
                format_duration(self.period),
                format_duration(MAX_DELAY)
            )));
        };
        if self.idle_at.is_some_and(|t| t > MAX_INSTANT) {
            return Err(InvalidRate(format!("idleAt must not be after {}", time::format_instant(MAX_INSTANT))));
        }
        Ok((interval, tau))
    }

    /// The slot of a submission handled at `now`, and the new `idle_at`; or why it is refused. Microseconds, checked:
    /// no slot moves earlier by rounding, wrapping or saturating.
    fn reserve(&self, now: Timestamp) -> Result<(Timestamp, Timestamp), String> {
        let (interval, tau) = self.bounds().map_err(|e| e.0)?;
        let now = time::to_micros(time::ceil(now));
        let base = self.idle_at.map_or(now, |t| time::to_micros(time::ceil(t))).max(now);
        let overflow = || format!("the slot is after {}", time::format_instant(MAX_INSTANT));
        let slot = base.checked_sub(tau).ok_or_else(overflow)?.max(now);
        let idle_at = base.checked_add(interval).ok_or_else(overflow)?;
        let last = time::to_micros(MAX_INSTANT);
        if slot > last || idle_at > last {
            return Err(overflow());
        }
        let delay = Duration::from_micros((slot - now) as u64);
        if delay > self.max_delay {
            return Err(format!(
                "the next free slot is {} away, more than the longest delay of {}",
                format_duration(delay),
                format_duration(self.max_delay)
            ));
        }
        match (Timestamp::from_timestamp_micros(slot), Timestamp::from_timestamp_micros(idle_at)) {
            (Some(slot), Some(idle_at)) => Ok((slot, idle_at)),
            _ => Err(overflow()),
        }
    }
}

/// The rate limiter type. It has no settings: the rate, burst and longest delay are in the state of each limiter,
/// fixed at its create; a create of a limiter that exists changes nothing.
///
/// Each `Submit` reserves a slot by the generic cell rate algorithm (GCRA): the interval between slots is the period
/// divided by the count, rounded up to a whole microsecond; the state keeps only the time its full burst is free
/// again. The slot and the forwarded message commit in one turn, so a submission that fails reserves nothing: a slot
/// beyond the longest delay, or a message the target does not take, is refused for good (the cause `refused`); a target
/// that is gone rejects the commit (`commit`); a read of the store that fails is tried by the submission's own retry
/// policy. A failure at the target after the forward, or a cancel of the forwarded message, frees no slot.
///
/// Slots are due times, not run times: messages that wait past their slots, behind a paused or slow target, after an
/// outage or a slow commit of the limiter, can run close together. It does not enforce an outside API's quota.
#[derive(Debug, Clone, Copy, Default)]
pub struct RateLimiterActor;

impl Actor for RateLimiterActor {
    type Ref = RateLimiter;
    type State = RateLimiterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: RateLimiterMsg) -> HandlerResult {
        let RateLimiterMsg::Submit { to, message, retry } = message;
        let (slot, idle_at) = turn.state().reserve(turn.now()).map_err(HandlerError::permanent)?;
        let mut options = SendOptions::new().at(slot);
        options.retry = retry;
        options.correlation_id = turn.message().correlation_id.clone();
        match turn.send_raw(&to, &message.message_type, message.payload, options).await {
            Ok(_) => {}
            // Tried again by the retry policy of the submission.
            Err(e @ TurnError::Store(_)) => return Err(e.into()),
            // No registered type, a type that does not take the message, or a payload out of bounds.
            Err(e) => return Err(HandlerError::permanent_from(e)),
        }
        turn.state_mut().idle_at = Some(idle_at);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn t(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn micros(us: i64) -> Duration {
        Duration::from_micros(us as u64)
    }

    /// Reserves `n` slots at `now`, keeping each new `idle_at`; the offset of each slot from `now`, or the refusal.
    fn reserve_n(state: &mut RateLimiterState, now: Timestamp, n: usize) -> Vec<Result<Duration, String>> {
        (0..n)
            .map(|_| {
                let (slot, idle_at) = state.reserve(now)?;
                state.idle_at = Some(idle_at);
                Ok(time::between(now, slot))
            })
            .collect()
    }

    #[test]
    fn invalid_settings_are_refused_by_the_constructor_and_the_decoder() {
        let ok = RateLimiterState::new(2, SECOND, 2, SECOND).unwrap();
        assert_eq!(ok.idle_at, None);
        let past = MAX_DELAY + Duration::from_nanos(1);
        for (count, period, burst, max_delay, why) in [
            (0, SECOND, 1, SECOND, "count must be positive"),
            (1, SECOND, 0, SECOND, "burst must be positive"),
            (1, Duration::ZERO, 1, SECOND, "period must be positive"),
            (1, past, 1, SECOND, "period must be positive and at most"),
            (1, SECOND, 1, past, "maxDelay must not exceed"),
            (1, MAX_DELAY, 3, SECOND, "spans more than"),
            (1, SECOND, u32::MAX, SECOND, ""),
        ] {
            let built = RateLimiterState::new(count, period, burst, max_delay);
            if why.is_empty() {
                assert!(built.is_ok(), "a burst of u32::MAX seconds is within MAX_DELAY");
                continue;
            }
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
            let json = serde_json::json!({"count": count, "period": format_duration(period), "burst": burst,
                "maxDelay": format_duration(max_delay)});
            let decoded = serde_json::from_value::<RateLimiterState>(json);
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {why}");
        }
        // A burst allowance of exactly MAX_DELAY, and one interval more.
        let period = Duration::from_secs(1_000);
        let burst = (MAX_DELAY.as_micros() / period.as_micros()) as u32 + 1;
        assert!(RateLimiterState::new(1, period, burst, SECOND).is_ok());
        assert!(RateLimiterState::new(1, period, burst + 1, SECOND).is_err());
        for bad in [
            serde_json::json!({"count": -1, "period": "PT1S", "burst": 1, "maxDelay": "PT1S"}),
            serde_json::json!({"count": 1, "period": "PT1S", "burst": 1}),
        ] {
            assert!(serde_json::from_value::<RateLimiterState>(bad.clone()).is_err(), "{bad}");
        }
        let decoded: RateLimiterState = serde_json::from_value(serde_json::json!(
            {"count": 2, "period": "PT1S", "burst": 2, "maxDelay": "PT1S", "idleAt": null, "other": 1}
        ))
        .unwrap();
        assert_eq!(decoded, ok);
    }

    #[test]
    fn an_idle_at_after_max_instant_is_refused() {
        let past = time::add(MAX_INSTANT, micros(1)).unwrap();
        let json = serde_json::json!({"count": 1, "period": "PT1S", "burst": 1, "maxDelay": "PT1S",
            "idleAt": time::format_instant(past)});
        let decoded = serde_json::from_value::<RateLimiterState>(json);
        assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains("idleAt")), "{decoded:?}");
        let mut state = RateLimiterState::new(1, SECOND, 1, MAX_DELAY).unwrap();
        state.idle_at = Some(past);
        assert!(state.reserve(t("2026-01-01T00:00:00Z")).unwrap_err().contains("idleAt"));
    }

    #[test]
    fn a_rate_that_needs_rounding_spaces_its_slots_by_the_rounded_interval() {
        let mut state = RateLimiterState::new(3, SECOND, 1, SECOND).unwrap();
        let now = t("2026-01-01T00:00:00Z");
        assert_eq!(reserve_n(&mut state, now, 3), [Ok(Duration::ZERO), Ok(micros(333_334)), Ok(micros(666_668))]);
    }

    #[test]
    fn a_sub_microsecond_period_spaces_slots_one_microsecond_apart() {
        let mut state = RateLimiterState::new(1, Duration::from_nanos(1), 1, SECOND).unwrap();
        let now = t("2026-01-01T00:00:00Z");
        assert_eq!(reserve_n(&mut state, now, 3), [Ok(Duration::ZERO), Ok(micros(1)), Ok(micros(2))]);
    }

    #[test]
    fn a_fractional_idle_at_rounds_the_slot_up() {
        let now = t("2026-01-01T00:00:00Z");
        let mut state = RateLimiterState::new(1, SECOND, 1, SECOND).unwrap();
        state.idle_at = Some(time::add(now, Duration::from_nanos(500)).unwrap());
        let (slot, idle_at) = state.reserve(now).unwrap();
        assert_eq!(time::between(now, slot), micros(1));
        assert_eq!(time::between(now, idle_at), micros(1_000_001));
    }

    #[test]
    fn burst_counts_the_first_and_the_longest_delay_is_inclusive() {
        let now = t("2026-01-01T00:00:00Z");
        let mut state = RateLimiterState::new(2, SECOND, 2, SECOND).unwrap();
        let slots = reserve_n(&mut state, now, 5);
        assert_eq!(slots[..4], [Ok(Duration::ZERO), Ok(Duration::ZERO), Ok(SECOND / 2), Ok(SECOND)]);
        assert!(slots[4].as_ref().is_err_and(|e| e.contains("longest delay")), "{:?}", slots[4]);
        assert_eq!(state.idle_at, Some(time::add(now, 2 * SECOND).unwrap()), "the refusal reserved nothing");
    }

    #[test]
    fn a_zero_longest_delay_takes_only_capacity_that_is_free_now() {
        let now = t("2026-01-01T00:00:00Z");
        let mut state = RateLimiterState::new(1, SECOND, 1, Duration::ZERO).unwrap();
        let slots = reserve_n(&mut state, now, 2);
        assert_eq!(slots[0], Ok(Duration::ZERO));
        assert!(slots[1].is_err());
    }

    #[test]
    fn idle_time_replenishes_the_burst() {
        let now = t("2026-01-01T00:00:00Z");
        let mut state = RateLimiterState::new(2, SECOND, 2, Duration::ZERO).unwrap();
        assert_eq!(reserve_n(&mut state, now, 2), [Ok(Duration::ZERO), Ok(Duration::ZERO)]);
        let later = time::add(now, SECOND).unwrap();
        assert_eq!(reserve_n(&mut state, later, 2), [Ok(Duration::ZERO), Ok(Duration::ZERO)]);
    }

    #[test]
    fn a_sub_microsecond_longest_delay_refuses_a_slot_one_microsecond_away() {
        let now = t("2026-01-01T00:00:00Z");
        let mut state = RateLimiterState::new(1, Duration::from_nanos(1), 1, Duration::from_nanos(500)).unwrap();
        state.idle_at = Some(time::add(now, micros(1)).unwrap());
        assert!(state.reserve(now).unwrap_err().contains("longest delay"));
    }

    #[test]
    fn a_slot_or_idle_at_after_max_instant_is_refused() {
        // The slot is free, but the new idle_at would pass MAX_INSTANT.
        let mut state = RateLimiterState::new(1, SECOND, 1, MAX_DELAY).unwrap();
        state.idle_at = Some(MAX_INSTANT);
        let now = t("9500-01-01T00:00:00Z");
        assert!(state.reserve(now).unwrap_err().contains("after 9999-12-31"));
        // At MAX_INSTANT itself.
        let state = RateLimiterState::new(1, SECOND, 1, MAX_DELAY).unwrap();
        assert!(state.reserve(MAX_INSTANT).unwrap_err().contains("after 9999-12-31"));
        // The last microsecond that still fits.
        let state = RateLimiterState::new(1, Duration::from_micros(1), 1, MAX_DELAY).unwrap();
        let before = time::sub(MAX_INSTANT, micros(1));
        assert_eq!(state.reserve(before), Ok((before, MAX_INSTANT)));
    }
}
