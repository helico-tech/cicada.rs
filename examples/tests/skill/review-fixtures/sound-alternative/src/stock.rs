// Stock of one SKU: the only owner of its available count and its reservations.

use std::collections::BTreeMap;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cart::{Cart, CartMsg};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stock {
    pub sku: String,
}

impl ActorRef for Stock {
    const TYPE: &'static str = "stock";
    type Message = StockMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum StockMsg {
    Reserve { request: String, customer: String, count: u32 },
    Release { request: String },
    Restock { count: u32 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct StockState {
    pub available: u32,
    /// Open reservations by request id; at most 10 000, refused beyond.
    pub reserved: BTreeMap<String, u32>,
}

pub struct StockActor;

impl Actor for StockActor {
    type Ref = Stock;
    type State = StockState;

    async fn handle(&self, turn: &mut Turn<Self>, message: StockMsg) -> HandlerResult {
        match message {
            StockMsg::Reserve { request, customer, count } => {
                let cart = Cart { customer };
                if turn.state().reserved.contains_key(&request) {
                    // A repeated request: answer again, reserve nothing new.
                    turn.send(&cart, CartMsg::Reserved { request })?;
                    return Ok(());
                }
                let state = turn.state_mut();
                if count > state.available || state.reserved.len() >= 10_000 {
                    turn.send(&cart, CartMsg::Refused { request })?;
                    return Ok(());
                }
                state.available -= count;
                state.reserved.insert(request.clone(), count);
                turn.send(&cart, CartMsg::Reserved { request })?;
            }
            StockMsg::Release { request } => {
                if let Some(count) = turn.state_mut().reserved.remove(&request) {
                    turn.state_mut().available += count;
                }
            }
            StockMsg::Restock { count } => turn.state_mut().available += count,
        }
        Ok(())
    }
}
