use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use super::order::{Order, OrderMsg};

/// The payment service, a stateless singleton and saga participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payments;

impl ActorRef for Payments {
    const TEMPLATE: &'static str = "actor://payments";
    type Message = PaymentsMsg;
}

/// Messages of [`Payments`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum PaymentsMsg {
    /// Charges `amount_cents` for `reply_to`; answered with [`OrderMsg::Charged`] or [`OrderMsg::ChargeDeclined`].
    Charge {
        /// The order that asks.
        reply_to: Order,
        /// What to charge.
        amount_cents: i64,
    },
}

/// The external payment gateway.
pub trait PaymentGateway: Send + Sync + 'static {
    /// Charges `amount_cents` for `order_id`. Called at least once per charge, so `idempotency_key` is the same for
    /// every attempt of one charge (also across retries). Fails with [`GatewayUnavailable`] on a transient failure;
    /// Payments retries it.
    fn charge(
        &self,
        order_id: &str,
        amount_cents: i64,
        idempotency_key: &str,
    ) -> impl Future<Output = Result<ChargeResult, GatewayUnavailable>> + Send;
}

/// What the gateway answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChargeResult {
    /// Charged; `transaction_id` identifies the payment.
    Approved {
        /// Identifies the payment.
        transaction_id: String,
    },
    /// Refused for a business reason (no funds, fraud check, ...).
    Declined {
        /// Why.
        reason: String,
    },
}

/// A transient gateway failure, worth retrying.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct GatewayUnavailable(pub String);

/// Total attempts of one charge, including the first.
pub const PAYMENT_ATTEMPTS: u32 = 4;

/// How the Order saga has a charge retried: [`PAYMENT_ATTEMPTS`] runs, 1 s, 2 s, 4 s apart.
pub fn charge_retry() -> RetryPolicy {
    let backoff = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(30)).expect("a valid backoff");
    RetryPolicy::new(PAYMENT_ATTEMPTS, backoff).expect("a valid retry policy")
}

/// The payments type, charging through `gateway`.
///
/// Payments shows per-message retry. The *sender* decides how a charge is retried: the Order saga sends `Charge` with
/// [`charge_retry`]. A transient gateway error is returned as an error, and the dispatcher puts the same message back
/// into the mailbox after the backoff; the last attempt turns the error into a business answer (`ChargeDeclined`) so
/// the saga can compensate instead of waiting forever. A decline by the gateway is an answer, not an error, and is
/// never retried. An invalid charge is the receiver's veto: `turn.fail(reason)` ends the message `failed` at once,
/// whatever the policy.
pub struct PaymentsActor<G> {
    /// The gateway.
    pub gateway: G,
}

impl<G: PaymentGateway> Actor for PaymentsActor<G> {
    type Ref = Payments;
    type State = Stateless;
    type Message = PaymentsMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: PaymentsMsg) -> HandlerResult {
        match message {
            PaymentsMsg::Charge { reply_to, amount_cents } => {
                // Not worth retrying: the receiver's veto ends the message failed now, whatever the sender's policy.
                if amount_cents <= 0 {
                    return Err(turn.fail(format!("invalid amount {amount_cents}")));
                }
                // A retry runs the same message again, so its id is stable across attempts: the right idempotency key.
                let key = turn.message().id.as_str().to_owned();
                let result = match self.gateway.charge(&reply_to.id, amount_cents, &key).await {
                    Ok(result) => result,
                    Err(unavailable) => {
                        // message.failures: earlier attempts that failed; message.retry: the policy the sender attached.
                        let attempts = turn.message().retry.as_ref().map_or(1, RetryPolicy::max_attempts);
                        if turn.message().failures + 1 < attempts {
                            return Err(unavailable.into()); // retried by the dispatcher after the backoff
                        }
                        // Out of attempts: answer, so the saga compensates.
                        ChargeResult::Declined { reason: "payment gateway unavailable".into() }
                    }
                };
                let reply = match result {
                    ChargeResult::Approved { transaction_id } => OrderMsg::Charged { transaction_id },
                    ChargeResult::Declined { reason } => OrderMsg::ChargeDeclined { reason },
                };
                turn.send(&reply_to, reply)?;
            }
        }
        Ok(())
    }
}
