use cicada::prelude::*;
use serde::{Deserialize, Serialize};

/// The pricing service, a stateless singleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pricing;

impl ActorRef for Pricing {
    const TEMPLATE: &'static str = "actor://pricing";
    type Message = PricingMsg;
}

/// Messages of [`Pricing`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum PricingMsg {
    /// Asks for the price of `quantity` × `sku`. Answered with [`PricingReply::Quote`] or
    /// [`PricingReply::QuoteRejected`] to the sender.
    QuoteRequest {
        /// The article.
        sku: String,
        /// How many.
        quantity: i32,
    },
}

/// The replies of the pricing protocol, as [`Pricing`] sends them. A requester's own message family contains messages
/// of the same names and fields (see [`crate::reply::CartMsg`]): on the wire a message is only its type name and
/// payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum PricingReply {
    /// The price of a [`PricingMsg::QuoteRequest`].
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
    /// The request cannot be priced.
    QuoteRejected {
        /// The article.
        sku: String,
        /// Why.
        reason: String,
    },
}

/// The price list Pricing reads from.
pub trait PriceList: Send + Sync + 'static {
    /// Unit price of `sku` in cents, or `None` when it is not sold.
    fn unit_price_cents(&self, sku: &str) -> Option<i64>;
}

/// The pricing type, reading `prices`.
pub struct PricingActor<P> {
    /// The price list.
    pub prices: P,
}

impl<P: PriceList> Actor for PricingActor<P> {
    type Ref = Pricing;
    type State = Stateless;
    type Message = PricingMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: PricingMsg) -> HandlerResult {
        match message {
            PricingMsg::QuoteRequest { sku, quantity } => {
                // Sent from outside an actor (HTTP, SDK): there is nobody to reply to.
                let Some(requester) = turn.message().from.clone() else { return Ok(()) };
                let reply = match self.prices.unit_price_cents(&sku) {
                    _ if quantity <= 0 => PricingReply::QuoteRejected { sku, reason: "quantity must be positive".into() },
                    None => PricingReply::QuoteRejected { sku, reason: "unknown sku".into() },
                    Some(unit) => PricingReply::Quote {
                        sku,
                        quantity,
                        unit_price_cents: unit,
                        total_cents: unit * i64::from(quantity),
                    },
                };
                let options = SendOptions { correlation_id: turn.message().correlation_id.clone(), ..SendOptions::new() };
                turn.send_to(&requester, &reply, options).await?;
            }
        }
        Ok(())
    }
}
