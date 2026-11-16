// The checkout of one order: waits for payment until a deadline, then tells shipping or gives up.

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::MessageId;
use serde::{Deserialize, Serialize};

use crate::shipping::{Shipping, ShippingMsg};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkout {
    pub order: String,
}

impl ActorRef for Checkout {
    const TYPE: &'static str = "checkouts";
    type Message = CheckoutMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CheckoutMsg {
    Begin,
    PaymentConfirmed { payment: String },
    PaymentTimeout,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Status {
    #[default]
    Open,
    AwaitingPayment,
    Paid,
    Expired,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CheckoutState {
    pub status: Status,
    pub timeout: Option<MessageId>,
    pub payment: Option<String>,
}

pub struct CheckoutActor;

impl Actor for CheckoutActor {
    type Ref = Checkout;
    type State = CheckoutState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CheckoutMsg) -> HandlerResult {
        match (turn.state().status, message) {
            (Status::Open, CheckoutMsg::Begin) => {
                let id = turn.send_self_with(CheckoutMsg::PaymentTimeout, SendOptions::new().delay(Duration::from_secs(900)))?;
                let state = turn.state_mut();
                state.status = Status::AwaitingPayment;
                state.timeout = Some(id);
            }
            (Status::AwaitingPayment, CheckoutMsg::PaymentConfirmed { payment }) => {
                if let Some(timeout) = turn.state_mut().timeout.take() {
                    turn.cancel(&timeout)?;
                }
                let order = turn.self_ref().order.clone();
                turn.send(&Shipping { order }, ShippingMsg::Ship)?;
                let state = turn.state_mut();
                state.status = Status::Paid;
                state.payment = Some(payment);
            }
            (Status::AwaitingPayment, CheckoutMsg::PaymentTimeout) => {
                if turn.state().timeout.as_ref() == Some(&turn.message().id) {
                    let state = turn.state_mut();
                    state.status = Status::Expired;
                    state.timeout = None;
                }
            }
            // A payment confirmed after expiry is dropped here; refunds are handled by the payments team's service.
            _ => {}
        }
        Ok(())
    }
}
