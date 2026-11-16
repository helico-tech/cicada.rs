// The card provider's client. Its HTTP API accepts an optional `Idempotency-Key` header, which this client does not send.

use std::future::Future;

pub trait CardGateway: Send + Sync + 'static {
    /// Charges `amount_cents` to `card` and returns the provider's receipt id.
    fn charge(&self, card: &str, amount_cents: i64) -> impl Future<Output = anyhow::Result<String>> + Send;
}
