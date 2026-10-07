//! Example 2 — timer / timeout with a generation number.
//!
//! A timer is a delayed self-send. Instead of cancelling the message, a timer is "cancelled" by making it stale: the
//! [`Timer`] of `cicada-extras`, kept in state, gets a new generation on every arm and disarm, and the timeout carries
//! the generation it was armed with. A timeout that is not current is ignored. Because state and the self-send commit
//! atomically, the timer in state and the pending timeouts never disagree.

use std::time::Duration;

use cicada::prelude::*;
use cicada::time;
use cicada_extras::Timer;
use serde::{Deserialize, Serialize};

/// One checkout session waiting for its payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkout {
    /// The checkout id.
    pub id: String,
}

impl ActorRef for Checkout {
    const TEMPLATE: &'static str = "actor://checkouts/{id}";
    type Message = CheckoutMsg;
}

/// Messages of [`Checkout`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum CheckoutMsg {
    /// Starts waiting for the payment; the checkout expires after `timeout_seconds` (900 by default).
    Begin {
        /// Seconds until the checkout expires.
        #[serde(default = "default_timeout_seconds")]
        timeout_seconds: u64,
    },
    /// Gives the customer `seconds` more: re-arms the timer, which makes the pending timeout stale.
    Extend {
        /// Seconds added to the deadline.
        seconds: u64,
    },
    /// The payment arrived.
    Pay {
        /// The amount paid.
        amount_cents: i64,
    },
    /// The timer: sent by the checkout to itself, valid only while `generation` is the current one.
    PaymentTimeout {
        /// The timer generation it was armed with.
        generation: u32,
    },
}

fn default_timeout_seconds() -> u64 {
    900
}

/// Life cycle of a [`Checkout`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckoutStatus {
    /// Not begun.
    #[default]
    Open,
    /// Waiting for the payment, with the timer armed.
    AwaitingPayment,
    /// Paid in time.
    Paid,
    /// The timer fired first.
    Expired,
}

/// State of a [`Checkout`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct CheckoutState {
    /// Where the checkout is.
    pub status: CheckoutStatus,
    /// The payment timeout; only a `PaymentTimeout` carrying its current generation counts.
    pub timer: Timer,
    /// The amount paid.
    pub paid_cents: Option<i64>,
    /// How many timeouts were ignored as stale (for illustration).
    pub stale_timeouts: i32,
}

/// The checkout type.
pub struct CheckoutActor;

impl Actor for CheckoutActor {
    type Ref = Checkout;
    type State = CheckoutState;
    type Message = CheckoutMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: CheckoutMsg) -> HandlerResult {
        let status = turn.state().status;
        match message {
            CheckoutMsg::Begin { timeout_seconds } => {
                if status != CheckoutStatus::Open {
                    return Ok(()); // already begun: Begin is idempotent
                }
                let deadline = time::add(turn.now(), Duration::from_secs(timeout_seconds))?;
                let state = turn.state_mut();
                state.status = CheckoutStatus::AwaitingPayment;
                let generation = state.timer.arm(deadline);
                turn.send_self_with(CheckoutMsg::PaymentTimeout { generation }, SendOptions::new().at(deadline))?;
            }
            CheckoutMsg::Extend { seconds } => {
                let Some(deadline) = turn.state().timer.deadline() else { return Ok(()) };
                if status != CheckoutStatus::AwaitingPayment {
                    return Ok(());
                }
                // Re-arm: a new generation and a new timeout. The old one is still in the mailbox and will be ignored.
                let deadline = time::add(deadline, Duration::from_secs(seconds))?;
                let generation = turn.state_mut().timer.arm(deadline);
                turn.send_self_with(CheckoutMsg::PaymentTimeout { generation }, SendOptions::new().at(deadline))?;
            }
            CheckoutMsg::Pay { amount_cents } => {
                if status != CheckoutStatus::AwaitingPayment {
                    return Ok(());
                }
                // Disarm: a new generation is all it takes.
                let state = turn.state_mut();
                state.status = CheckoutStatus::Paid;
                state.timer.disarm();
                state.paid_cents = Some(amount_cents);
            }
            CheckoutMsg::PaymentTimeout { generation } => {
                let current = turn.state().timer.is_current(generation);
                let state = turn.state_mut();
                if !current || status != CheckoutStatus::AwaitingPayment {
                    state.stale_timeouts += 1;
                } else {
                    state.status = CheckoutStatus::Expired;
                    state.timer.disarm();
                }
            }
        }
        Ok(())
    }
}
