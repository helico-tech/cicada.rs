//! Example 2 — timer / timeout.
//!
//! A timer is a delayed self-send. The state keeps the id of the pending timeout; to stop or move the timer, the
//! handler cancels that message with `turn.cancel`. State, the cancel and the new self-send commit atomically, and the
//! turns of one actor never overlap, so the timeout in state and the one in the mailbox never disagree.

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::MessageId;
use cicada::time::{self, Timestamp};
use serde::{Deserialize, Serialize};

/// One checkout session waiting for its payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkout {
    /// The checkout id.
    pub id: String,
}

impl ActorRef for Checkout {
    const TYPE: &'static str = "checkouts";
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
    /// Gives the customer `seconds` more: cancels the pending timeout and sends a later one.
    Extend {
        /// Seconds added to the deadline.
        seconds: u64,
    },
    /// The payment arrived.
    Pay {
        /// The amount paid.
        amount_cents: i64,
    },
    /// The timer: sent by the checkout to itself.
    PaymentTimeout,
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
    /// When the checkout expires; null while no timer runs.
    pub deadline: Option<Timestamp>,
    /// The id of the pending `PaymentTimeout`, so it can be cancelled; null while no timer runs.
    pub timeout: Option<MessageId>,
    /// The amount paid.
    pub paid_cents: Option<i64>,
}

/// The checkout type.
pub struct CheckoutActor;

impl Actor for CheckoutActor {
    type Ref = Checkout;
    type State = CheckoutState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CheckoutMsg) -> HandlerResult {
        let status = turn.state().status;
        match message {
            CheckoutMsg::Begin { timeout_seconds } => {
                if status != CheckoutStatus::Open {
                    return Ok(()); // already begun: Begin is idempotent
                }
                let deadline = time::add(turn.now(), Duration::from_secs(timeout_seconds))?;
                turn.state_mut().status = CheckoutStatus::AwaitingPayment;
                arm(turn, deadline)?;
            }
            CheckoutMsg::Extend { seconds } => {
                let Some(deadline) = turn.state().deadline else { return Ok(()) };
                if status != CheckoutStatus::AwaitingPayment {
                    return Ok(());
                }
                disarm(turn)?;
                arm(turn, time::add(deadline, Duration::from_secs(seconds))?)?;
            }
            CheckoutMsg::Pay { amount_cents } => {
                if status != CheckoutStatus::AwaitingPayment {
                    return Ok(());
                }
                disarm(turn)?;
                let state = turn.state_mut();
                state.status = CheckoutStatus::Paid;
                state.paid_cents = Some(amount_cents);
            }
            CheckoutMsg::PaymentTimeout => {
                if turn.state().timeout.as_ref() != Some(&turn.message().id) {
                    return Ok(()); // not the timer in state: sent by hand, not by this actor
                }
                let state = turn.state_mut();
                state.status = CheckoutStatus::Expired;
                state.deadline = None;
                state.timeout = None;
            }
        }
        Ok(())
    }
}

/// Starts the timer: a `PaymentTimeout` due at `deadline`, with its id kept in state.
fn arm(turn: &mut Turn<CheckoutActor>, deadline: Timestamp) -> HandlerResult {
    let timeout = turn.send_self_with(CheckoutMsg::PaymentTimeout, SendOptions::new().at(deadline))?;
    let state = turn.state_mut();
    state.deadline = Some(deadline);
    state.timeout = Some(timeout);
    Ok(())
}

/// Stops the timer: cancels the pending `PaymentTimeout` in this turn's commit.
fn disarm(turn: &mut Turn<CheckoutActor>) -> HandlerResult {
    if let Some(timeout) = turn.state_mut().timeout.take() {
        turn.cancel(&timeout)?;
    }
    turn.state_mut().deadline = None;
    Ok(())
}
