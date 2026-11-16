// Excerpt: how an order asks for its charge.

use std::time::Duration;

use cicada::prelude::*;

use crate::payment::{Payment, PaymentMsg};

pub(crate) fn request_charge(turn: &mut Turn<OrderActor>, card: String, amount_cents: i64, email: String) -> HandlerResult {
    let payment = Payment { order: turn.self_ref().id.clone() };
    let ledger_month = turn.state().month.clone();
    let backoff = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(30))?;
    let options = SendOptions::new().retry(RetryPolicy::new(3, backoff)?);
    turn.send_with(&payment, PaymentMsg::Charge { card, amount_cents, ledger_month, email }, options)?;
    Ok(())
}
