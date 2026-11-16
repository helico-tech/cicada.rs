// The cart of one customer, which also owns its checkout (see docs/design/actors.md, Decisions).

use std::collections::BTreeMap;
use std::time::Duration;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use crate::stock::{Stock, StockMsg};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cart {
    pub customer: String,
}

impl ActorRef for Cart {
    const TYPE: &'static str = "carts";
    type Message = CartMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CartMsg {
    Add { sku: String, count: u32 },
    Reserved { request: String },
    Refused { request: String },
    Expire { request: String },
    Checkout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LineStatus {
    Pending,
    Reserved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Line {
    pub sku: String,
    pub count: u32,
    pub status: LineStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CartState {
    /// Lines by the request id of their reservation; at most 50.
    pub lines: BTreeMap<String, Line>,
    pub checked_out: bool,
}

pub struct CartActor;

impl Actor for CartActor {
    type Ref = Cart;
    type State = CartState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CartMsg) -> HandlerResult {
        if turn.state().checked_out {
            return Ok(()); // a finished cart changes no more; late replies are ignored
        }
        match message {
            CartMsg::Add { sku, count } => {
                if turn.state().lines.len() >= 50 {
                    return Ok(());
                }
                let request = turn.message().id.to_string();
                let customer = turn.self_ref().customer.clone();
                let line = Line { sku: sku.clone(), count, status: LineStatus::Pending };
                turn.state_mut().lines.insert(request.clone(), line);
                turn.send(&Stock { sku }, StockMsg::Reserve { request: request.clone(), customer, count })?;
                let expire = SendOptions::new().delay(Duration::from_secs(15 * 60));
                turn.send_self_with(CartMsg::Expire { request }, expire)?;
            }
            CartMsg::Reserved { request } => {
                if let Some(line) = turn.state_mut().lines.get_mut(&request) {
                    line.status = LineStatus::Reserved;
                }
            }
            CartMsg::Refused { request } => {
                turn.state_mut().lines.remove(&request);
            }
            CartMsg::Expire { request } => {
                if let Some(line) = turn.state_mut().lines.remove(&request) {
                    turn.send(&Stock { sku: line.sku }, StockMsg::Release { request })?;
                }
            }
            CartMsg::Checkout => {
                let all_reserved = turn.state().lines.values().all(|l| l.status == LineStatus::Reserved);
                if all_reserved && !turn.state().lines.is_empty() {
                    for (request, line) in turn.state().lines.clone() {
                        turn.send(&Stock { sku: line.sku }, StockMsg::Confirm { request })?;
                    }
                    turn.state_mut().checked_out = true;
                }
            }
        }
        Ok(())
    }
}
