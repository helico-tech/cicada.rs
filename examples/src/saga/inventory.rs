use std::collections::BTreeMap;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use super::order::{Order, OrderMsg};
use super::traced;

/// Stock of one sku. A saga participant: it answers [`InventoryMsg::Reserve`] and [`InventoryMsg::Release`] to the
/// order that asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    /// The article.
    pub sku: String,
}

impl ActorRef for Inventory {
    const TEMPLATE: &'static str = "actor://inventory/{sku}";
    type Message = InventoryMsg;
}

/// Messages of [`Inventory`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum InventoryMsg {
    /// Reserves `quantity` for `reply_to`; answered with [`OrderMsg::Reserved`] or [`OrderMsg::OutOfStock`].
    Reserve {
        /// The order that asks.
        reply_to: Order,
        /// How many.
        quantity: i32,
    },
    /// Releases `reply_to`'s reservation; answered with [`OrderMsg::Released`]. Idempotent.
    Release {
        /// The order that asks.
        reply_to: Order,
    },
    /// Adds stock.
    Restock {
        /// How many.
        quantity: i32,
    },
}

/// State of an [`Inventory`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct InventoryState {
    /// Stock that is not reserved.
    pub available: i32,
    /// Reserved quantity per order id; reserving is idempotent per order.
    pub reservations: BTreeMap<String, i32>,
}

/// The inventory type.
pub struct InventoryActor;

impl Actor for InventoryActor {
    type Ref = Inventory;
    type State = InventoryState;
    type Message = InventoryMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: InventoryMsg) -> HandlerResult {
        match message {
            InventoryMsg::Reserve { reply_to, quantity } => {
                let available = turn.state().available;
                if turn.state().reservations.contains_key(&reply_to.id) {
                    turn.send_with(&reply_to, OrderMsg::Reserved, traced(turn))?; // asked again: same answer
                } else if quantity > available {
                    turn.send_with(&reply_to, OrderMsg::OutOfStock { available }, traced(turn))?;
                } else {
                    let state = turn.state_mut();
                    state.available -= quantity;
                    state.reservations.insert(reply_to.id.clone(), quantity);
                    turn.send_with(&reply_to, OrderMsg::Reserved, traced(turn))?;
                }
            }
            InventoryMsg::Release { reply_to } => {
                if let Some(reserved) = turn.state().reservations.get(&reply_to.id).copied() {
                    let state = turn.state_mut();
                    state.available += reserved;
                    state.reservations.remove(&reply_to.id);
                }
                turn.send_with(&reply_to, OrderMsg::Released, traced(turn))?;
            }
            InventoryMsg::Restock { quantity } => turn.state_mut().available += quantity,
        }
        Ok(())
    }
}
