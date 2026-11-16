// Shipping of one order: books a courier through the courier's API (not part of the checkout review).

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use crate::courier::Courier;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shipping {
    pub order: String,
}

impl ActorRef for Shipping {
    const TYPE: &'static str = "shipping";
    type Message = ShippingMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ShippingMsg {
    Ship,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ShippingState {
    pub booking: Option<String>,
}

pub struct ShippingActor {
    pub courier: Courier,
}

impl Actor for ShippingActor {
    type Ref = Shipping;
    type State = ShippingState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ShippingMsg) -> HandlerResult {
        match message {
            ShippingMsg::Ship => {
                let booking = self.courier.book(&turn.self_ref().order).await?;
                turn.state_mut().booking = Some(booking);
            }
        }
        Ok(())
    }
}
