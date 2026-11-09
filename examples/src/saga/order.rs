use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use super::inventory::{Inventory, InventoryMsg};
use super::payments::{Payments, PaymentsMsg, charge_retry};

/// One order, orchestrating inventory and payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Order {
    /// The order id.
    pub id: String,
}

impl ActorRef for Order {
    const TEMPLATE: &'static str = "actor://orders/{id}";
    type Message = OrderMsg;
}

/// Messages of [`Order`]: the command that starts it and the participants' replies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum OrderMsg {
    /// Places the order: reserve, then charge.
    PlaceOrder {
        /// The article.
        sku: String,
        /// How many.
        quantity: i32,
        /// What to charge.
        amount_cents: i64,
    },
    /// Inventory reserved the stock.
    Reserved,
    /// Inventory could not reserve the stock.
    OutOfStock {
        /// The stock that is available.
        available: i32,
    },
    /// Payments charged the customer.
    Charged {
        /// Identifies the payment.
        transaction_id: String,
    },
    /// Payments could not charge the customer; starts the compensation.
    ChargeDeclined {
        /// Why.
        reason: String,
    },
    /// Inventory released the reservation (the compensation of [`OrderMsg::Reserved`]).
    Released,
}

/// Steps of an [`Order`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderStatus {
    /// Not placed yet.
    #[default]
    New,
    /// Waiting for the inventory.
    Reserving,
    /// Waiting for the payment.
    Charging,
    /// Reserved and charged.
    Completed,
    /// Releasing the reservation after a declined payment.
    Compensating,
    /// Out of stock, or compensated.
    Cancelled,
}

/// State of an [`Order`]: the step machine plus what the steps produced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct OrderState {
    /// The current step.
    pub status: OrderStatus,
    /// The article.
    pub sku: Option<String>,
    /// How many.
    pub quantity: i32,
    /// What to charge.
    pub amount_cents: i64,
    /// The payment, once charged.
    pub transaction_id: Option<String>,
    /// Why the order was cancelled.
    pub failure: Option<String>,
}

/// The order type.
pub struct OrderActor;

impl Actor for OrderActor {
    type Ref = Order;
    type State = OrderState;
    type Message = OrderMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: OrderMsg) -> HandlerResult {
        let status = turn.state().status;
        let me = turn.self_ref().clone();
        // Every reply first checks that the saga is in the step it answers; anything else is a late or duplicate reply.
        match message {
            OrderMsg::PlaceOrder { sku, quantity, amount_cents } => {
                if status != OrderStatus::New {
                    return Ok(()); // placing twice is a no-op
                }
                turn.set_state(OrderState {
                    status: OrderStatus::Reserving,
                    sku: Some(sku.clone()),
                    quantity,
                    amount_cents,
                    ..OrderState::default()
                });
                turn.send(&Inventory { sku }, InventoryMsg::Reserve { reply_to: me, quantity })?;
            }
            OrderMsg::Reserved => {
                if status != OrderStatus::Reserving {
                    return Ok(());
                }
                turn.state_mut().status = OrderStatus::Charging;
                let amount_cents = turn.state().amount_cents;
                // The sender decides how the charge is retried: the policy travels with the message.
                turn.send_with(
                    &Payments,
                    PaymentsMsg::Charge { reply_to: me, amount_cents },
                    SendOptions::new().retry(charge_retry()),
                )?;
            }
            OrderMsg::OutOfStock { available } => {
                if status != OrderStatus::Reserving {
                    return Ok(());
                }
                // Nothing succeeded yet, so there is nothing to compensate.
                let state = turn.state_mut();
                state.status = OrderStatus::Cancelled;
                state.failure = Some(format!("out of stock ({available} available)"));
            }
            OrderMsg::Charged { transaction_id } => {
                if status != OrderStatus::Charging {
                    return Ok(());
                }
                let state = turn.state_mut();
                state.status = OrderStatus::Completed;
                state.transaction_id = Some(transaction_id);
            }
            OrderMsg::ChargeDeclined { reason } => {
                if status != OrderStatus::Charging {
                    return Ok(());
                }
                // Compensate the reservation, then finish as cancelled once it is confirmed.
                let state = turn.state_mut();
                state.status = OrderStatus::Compensating;
                state.failure = Some(format!("payment declined: {reason}"));
                let sku = state.sku.clone().ok_or_else(|| HandlerError::msg("a charging order has a sku"))?;
                turn.send(&Inventory { sku }, InventoryMsg::Release { reply_to: me })?;
            }
            OrderMsg::Released => {
                if status != OrderStatus::Compensating {
                    return Ok(());
                }
                turn.state_mut().status = OrderStatus::Cancelled;
            }
        }
        Ok(())
    }
}
