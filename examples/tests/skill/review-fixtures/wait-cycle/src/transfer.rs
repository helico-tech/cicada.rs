// Moves money between two accounts. The transfer holds the amount on the source account, then credits the target.

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transfer {
    pub id: String,
}

impl ActorRef for Transfer {
    const TYPE: &'static str = "transfers";
    type Message = TransferMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum TransferMsg {
    Start { from: String, to: String, amount_cents: i64 },
    Held,
    Credited,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Step {
    #[default]
    New,
    AwaitingHold,
    AwaitingCredit,
    Done,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TransferState {
    pub step: Step,
    pub from: String,
    pub to: String,
    pub amount_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
}

impl ActorRef for Account {
    const TYPE: &'static str = "accounts";
    type Message = AccountMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum AccountMsg {
    /// Asks to hold an amount for a transfer. The account answers Held once the transfer confirms it (Confirm).
    Hold { transfer: String, amount_cents: i64 },
    /// The transfer confirms that it still wants the hold.
    Confirm { transfer: String },
    Credit { transfer: String, amount_cents: i64 },
}

pub struct TransferActor;

impl Actor for TransferActor {
    type Ref = Transfer;
    type State = TransferState;

    async fn handle(&self, turn: &mut Turn<Self>, message: TransferMsg) -> HandlerResult {
        let transfer = turn.self_ref().id.clone();
        match (turn.state().step, message) {
            (Step::New, TransferMsg::Start { from, to, amount_cents }) => {
                turn.send(&Account { id: from.clone() }, AccountMsg::Hold { transfer, amount_cents })?;
                *turn.state_mut() = TransferState { step: Step::AwaitingHold, from, to, amount_cents };
            }
            (Step::AwaitingHold, TransferMsg::Held) => {
                let state = turn.state().clone();
                turn.send(&Account { id: state.from }, AccountMsg::Confirm { transfer: transfer.clone() })?;
                let credit = AccountMsg::Credit { transfer, amount_cents: state.amount_cents };
                turn.send(&Account { id: state.to }, credit)?;
                turn.state_mut().step = Step::AwaitingCredit;
            }
            (Step::AwaitingCredit, TransferMsg::Credited) => turn.state_mut().step = Step::Done,
            _ => {}
        }
        Ok(())
    }
}
