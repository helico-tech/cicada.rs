use cicada::time::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A timer kept in an actor's state: a delayed self-send that can be re-armed and cancelled.
///
/// A timer is cancelled by making its message stale: every [`Self::arm`] and [`Self::disarm`] increments the
/// generation, and the timeout message carries the generation it was armed with. Only a timeout for which
/// [`Self::is_current`] is true counts; any other is a leftover and is ignored. State and the self-send commit
/// atomically, so the timer in state and the pending timeouts never disagree. (A handler may also cancel the pending
/// timeout with `turn.cancel(id)`; the generation check still guards against one that was already claimed.)
///
/// The timer does not send: the handler does, in the same turn.
///
/// ```
/// # use std::time::Duration;
/// # use cicada::prelude::*;
/// # use cicada_extras::Timer;
/// # use serde::{Deserialize, Serialize};
/// # #[derive(Debug, Clone, Serialize, Deserialize)]
/// # struct Checkout { id: String }
/// # impl ActorRef for Checkout { const TEMPLATE: &'static str = "actor://checkout/{id}"; type Message = CheckoutMsg; }
/// #[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// #[serde(tag = "type")]
/// enum CheckoutMsg {
///     Begin,
///     Pay,
///     PaymentTimeout { generation: u32 },
/// }
///
/// #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
/// struct CheckoutState {
///     timer: Timer,
///     expired: bool,
/// }
///
/// struct CheckoutActor;
///
/// impl Actor for CheckoutActor {
///     type Ref = Checkout;
///     type State = CheckoutState;
///     type Message = CheckoutMsg;
///     const UNHANDLED: Unhandled = Unhandled::Fail;
///
///     async fn handle(&self, turn: &mut Turn<Self>, msg: CheckoutMsg) -> HandlerResult {
///         match msg {
///             CheckoutMsg::Begin => {
///                 let deadline = turn.now() + Duration::from_secs(15 * 60);
///                 let generation = turn.state_mut().timer.arm(deadline);
///                 turn.send_self_with(CheckoutMsg::PaymentTimeout { generation }, SendOptions::new().at(deadline))?;
///             }
///             CheckoutMsg::Pay => turn.state_mut().timer.disarm(),
///             CheckoutMsg::PaymentTimeout { generation } => {
///                 if !turn.state().timer.is_current(generation) {
///                     return Ok(()); // stale: re-armed or disarmed since
///                 }
///                 let state = turn.state_mut();
///                 state.timer.disarm();
///                 state.expired = true;
///             }
///         }
///         Ok(())
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Timer {
    /// Incremented on every arm and disarm.
    #[serde(default)]
    generation: u32,
    /// When the armed timeout is due; null while disarmed.
    #[serde(default)]
    deadline: Option<Timestamp>,
}

impl Timer {
    /// A disarmed timer at generation 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The generation, incremented on every [`Self::arm`] and [`Self::disarm`].
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// When the armed timeout is due; `None` while disarmed.
    pub fn deadline(&self) -> Option<Timestamp> {
        self.deadline
    }

    /// True while a timeout is armed.
    pub fn is_armed(&self) -> bool {
        self.deadline.is_some()
    }

    /// Arms (or re-arms) the timer for `at`: a new generation, so any pending timeout becomes stale. Returns the new
    /// generation, for the timeout message.
    pub fn arm(&mut self, at: Timestamp) -> u32 {
        self.generation = self.generation.wrapping_add(1);
        self.deadline = Some(at);
        self.generation
    }

    /// Disarms the timer: a new generation, so any pending timeout becomes stale.
    pub fn disarm(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.deadline = None;
    }

    /// True when a timeout armed with `generation` is the live one: the timer is armed with that generation.
    pub fn is_current(&self, generation: u32) -> bool {
        self.is_armed() && generation == self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn a_new_timer_is_disarmed_and_nothing_is_current() {
        let timer = Timer::new();
        assert!(!timer.is_armed());
        assert!(!timer.is_current(0));
    }

    #[test]
    fn arming_makes_only_the_new_generation_current() {
        let mut timer = Timer::new();
        let generation = timer.arm(t("2026-10-03T12:00:00Z"));
        assert!(timer.is_armed());
        assert_eq!(timer.deadline(), Some(t("2026-10-03T12:00:00Z")));
        assert_eq!(generation, timer.generation());
        assert!(timer.is_current(generation));
        assert!(!timer.is_current(generation - 1));
    }

    #[test]
    fn re_arming_and_disarming_make_a_pending_timeout_stale() {
        let mut timer = Timer::new();
        let first = timer.arm(t("2026-10-03T12:00:00Z"));
        let second = timer.arm(t("2026-10-03T12:05:00Z"));
        assert!(!timer.is_current(first));
        assert!(timer.is_current(second));
        timer.disarm();
        assert_eq!(timer.deadline(), None);
        assert!(!timer.is_current(second));
        assert!(!timer.is_current(timer.generation()));
    }

    #[test]
    fn keeps_its_wire_form() {
        let mut timer = Timer::new();
        assert_eq!(serde_json::to_string(&timer).unwrap(), r#"{"generation":0,"deadline":null}"#);
        timer.arm(t("2026-10-03T12:00:00Z"));
        assert_eq!(serde_json::to_string(&timer).unwrap(), r#"{"generation":1,"deadline":"2026-10-03T12:00:00Z"}"#);
        assert_eq!(serde_json::from_str::<Timer>("{}").unwrap(), Timer::new());
        assert_eq!(
            serde_json::from_str::<Timer>(r#"{"generation":1,"deadline":"2026-10-03T12:00:00Z"}"#).unwrap(),
            timer
        );
    }
}
