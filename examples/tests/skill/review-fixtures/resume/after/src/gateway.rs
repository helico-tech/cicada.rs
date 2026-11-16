// The card provider's client. It sends the `Idempotency-Key` header; the provider keeps keys for 24 hours.

use std::future::Future;

pub trait CardGateway: Send + Sync + 'static {
    /// Charges `amount_cents` to `card` under `key` and returns the provider's receipt id. A repeated key returns the
    /// first charge's receipt without charging again.
    fn charge(&self, card: &str, amount_cents: i64, key: &str) -> impl Future<Output = anyhow::Result<String>> + Send;
}

pub trait ReceiptMailer: Send + Sync + 'static {
    /// Emails the receipt to the customer. The mail service has no idempotency key.
    fn send_receipt(&self, email: &str, receipt: &str) -> impl Future<Output = anyhow::Result<()>> + Send;
}
