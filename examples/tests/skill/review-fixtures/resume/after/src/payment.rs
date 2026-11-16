// Charges the card of one order. The order sends Charge with handler retry (3 attempts, see src/order.rs).

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use crate::gateway::{CardGateway, ReceiptMailer};
use crate::ledger::{Ledger, LedgerMsg};
use crate::order::{Order, OrderMsg};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payment {
    pub order: String,
}

impl ActorRef for Payment {
    const TYPE: &'static str = "payments";
    type Message = PaymentMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum PaymentMsg {
    Charge { card: String, amount_cents: i64, ledger_month: String, email: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PaymentState {
    pub charged_cents: i64,
    pub receipts: Vec<String>,
}

pub struct PaymentActor<G, M> {
    pub gateway: G,
    pub mailer: M,
}

impl<G: CardGateway, M: ReceiptMailer> Actor for PaymentActor<G, M> {
    type Ref = Payment;
    type State = PaymentState;

    async fn handle(&self, turn: &mut Turn<Self>, message: PaymentMsg) -> HandlerResult {
        match message {
            PaymentMsg::Charge { card, amount_cents, ledger_month, email } => {
                // The message id names the charge across retries and crash recovery.
                let key = turn.message().id.to_string();
                let receipt = self.gateway.charge(&card, amount_cents, &key).await?;
                // Tell the customer right away.
                self.mailer.send_receipt(&email, &receipt).await?;
                let state = turn.state_mut();
                state.charged_cents += amount_cents;
                state.receipts.push(receipt.clone());
                let order = Order { id: turn.self_ref().order.clone() };
                turn.send(&order, OrderMsg::Charged { receipt: receipt.clone() })?;
                // The ledger of the month is created by a monthly job; early in the month it may not exist yet.
                turn.send(&Ledger { month: ledger_month }, LedgerMsg::Record { receipt, amount_cents })?;
            }
        }
        Ok(())
    }
}
