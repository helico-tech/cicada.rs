// One account: owns its balance and the holds of open transfers.

use std::collections::BTreeMap;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use crate::transfer::{Account, AccountMsg, Transfer, TransferMsg};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AccountState {
    pub balance_cents: i64,
    /// Holds waiting for their transfer's Confirm, by transfer id.
    pub pending: BTreeMap<String, i64>,
    /// Confirmed holds, by transfer id.
    pub held: BTreeMap<String, i64>,
}

pub struct AccountActor;

impl Actor for AccountActor {
    type Ref = Account;
    type State = AccountState;

    async fn handle(&self, turn: &mut Turn<Self>, message: AccountMsg) -> HandlerResult {
        match message {
            AccountMsg::Hold { transfer, amount_cents } => {
                let state = turn.state_mut();
                if state.balance_cents >= amount_cents {
                    state.balance_cents -= amount_cents;
                    // Not answered yet: Held goes out when the transfer confirms.
                    state.pending.insert(transfer, amount_cents);
                }
            }
            AccountMsg::Confirm { transfer } => {
                if let Some(amount) = turn.state_mut().pending.remove(&transfer) {
                    turn.state_mut().held.insert(transfer.clone(), amount);
                    turn.send(&Transfer { id: transfer }, TransferMsg::Held)?;
                }
            }
            AccountMsg::Credit { transfer, amount_cents } => {
                turn.state_mut().balance_cents += amount_cents;
                turn.send(&Transfer { id: transfer }, TransferMsg::Credited)?;
            }
        }
        Ok(())
    }
}
