// Stock per SKU (not part of the checkout review). Omitted here: Reserve, Release and Restock handlers.

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub sku: String,
}

impl ActorRef for Inventory {
    const TYPE: &'static str = "inventory";
    type Message = InventoryMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum InventoryMsg {
    Reserve { order: String, count: u32 },
    Release { order: String },
    Restock { count: u32 },
}
