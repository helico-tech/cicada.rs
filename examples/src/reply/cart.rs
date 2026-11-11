use std::collections::BTreeMap;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use super::pricing::{Pricing, PricingMsg};

/// A shopping cart that prices its lines by asking [`Pricing`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cart {
    /// The cart id.
    pub id: String,
}

impl ActorRef for Cart {
    const TYPE: &'static str = "carts";
    type Message = CartMsg;
}

/// Messages of [`Cart`]: its own commands plus the replies of the pricing protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum CartMsg {
    /// Adds `quantity` × `sku`; the line is priced asynchronously.
    AddItem {
        /// The article.
        sku: String,
        /// How many.
        quantity: i32,
    },
    /// Reply of [`Pricing`] to a [`PricingMsg::QuoteRequest`].
    Quote {
        /// The article.
        sku: String,
        /// How many.
        quantity: i32,
        /// Price of one.
        unit_price_cents: i64,
        /// Price of all.
        total_cents: i64,
    },
    /// Reply of [`Pricing`] when it cannot price a [`PricingMsg::QuoteRequest`].
    QuoteRejected {
        /// The article.
        sku: String,
        /// Why.
        reason: String,
    },
}

/// A priced cart line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CartLine {
    /// The article.
    pub sku: String,
    /// How many.
    pub quantity: i32,
    /// Price of the line.
    pub total_cents: i64,
}

/// A line waiting for its quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingLine {
    /// The article.
    pub sku: String,
    /// How many.
    pub quantity: i32,
}

/// State of a [`Cart`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CartState {
    /// The priced lines.
    pub lines: Vec<CartLine>,
    /// Requests in flight, by correlation id.
    pub awaiting: BTreeMap<String, PendingLine>,
    /// Rejection reason per sku.
    pub rejected: BTreeMap<String, String>,
}

impl CartState {
    /// Sum of all priced lines.
    pub fn total_cents(&self) -> i64 {
        self.lines.iter().map(|l| l.total_cents).sum()
    }
}

/// The cart type.
pub struct CartActor;

impl Actor for CartActor {
    type Ref = Cart;
    type State = CartState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CartMsg) -> HandlerResult {
        match message {
            CartMsg::AddItem { sku, quantity } => {
                // The request's own message id is a unique correlation id; the reply carries it back.
                let correlation_id = turn.message().id.as_str().to_owned();
                turn.state_mut().awaiting.insert(correlation_id.clone(), PendingLine { sku: sku.clone(), quantity });
                turn.send_with(
                    &Pricing,
                    PricingMsg::QuoteRequest { sku, quantity },
                    SendOptions::new().correlation_id(correlation_id),
                )?;
            }
            CartMsg::Quote { sku, quantity, total_cents, .. } => {
                // Match the reply to its request; anything unknown (a duplicate, an unsolicited quote) is ignored.
                let Some(correlation_id) = awaited(turn) else { return Ok(()) };
                let state = turn.state_mut();
                state.lines.push(CartLine { sku, quantity, total_cents });
                state.awaiting.remove(&correlation_id);
            }
            CartMsg::QuoteRejected { sku, reason } => {
                let Some(correlation_id) = awaited(turn) else { return Ok(()) };
                let state = turn.state_mut();
                state.awaiting.remove(&correlation_id);
                state.rejected.insert(sku, reason);
            }
        }
        Ok(())
    }
}

/// The correlation id of the current reply when it answers a request in flight.
fn awaited(turn: &Turn<CartActor>) -> Option<String> {
    let correlation_id = turn.message().correlation_id.as_ref()?;
    turn.state().awaiting.contains_key(correlation_id).then(|| correlation_id.clone())
}
