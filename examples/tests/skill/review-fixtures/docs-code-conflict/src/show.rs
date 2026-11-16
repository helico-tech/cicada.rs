// The seats of one show.

use std::collections::BTreeMap;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Show {
    pub id: String,
}

impl ActorRef for Show {
    const TYPE: &'static str = "shows";
    type Message = ShowMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ShowMsg {
    Hold { seat: String, customer: String },
    Pay { seat: String, customer: String },
    Release { seat: String, customer: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Seat {
    Held { customer: String },
    Sold { customer: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ShowState {
    /// Seats that are held or sold; a free seat has no entry.
    pub seats: BTreeMap<String, Seat>,
}

pub struct ShowActor;

impl Actor for ShowActor {
    type Ref = Show;
    type State = ShowState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ShowMsg) -> HandlerResult {
        let seats = &mut turn.state_mut().seats;
        match message {
            ShowMsg::Hold { seat, customer } => {
                seats.entry(seat).or_insert(Seat::Held { customer });
            }
            ShowMsg::Pay { seat, customer } => {
                if seats.get(&seat) == Some(&Seat::Held { customer: customer.clone() }) {
                    seats.insert(seat, Seat::Sold { customer });
                }
            }
            // The web client sends Release when the customer leaves the checkout page.
            ShowMsg::Release { seat, customer } => {
                if seats.get(&seat) == Some(&Seat::Held { customer }) {
                    seats.remove(&seat);
                }
            }
        }
        Ok(())
    }
}
