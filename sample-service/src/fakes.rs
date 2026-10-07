//! Stand-ins for the external systems the examples talk to. They log and answer deterministically, with a few hooks
//! for manual experiments over HTTP:
//!
//! - payments decline amounts above 100.00 (10 000 cents); for an amount ending in `13` cents the gateway is
//!   "unavailable" on the first call of each charge *per node* (in memory), so it fails at most once per node and the
//!   saga's charge retry policy (4 attempts) always gets it through on a cluster of up to three nodes;
//! - onboarding fails to provision a workspace for users whose id contains `fail` (`actor://onboarding/fail-1`);
//! - the sync API serves three pages (10, 10 and 5 items).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use cicada::time::Timestamp;
use cicada_examples::cron::{CronTask, TaskError};
use cicada_examples::fanout::ItemProcessor;
use cicada_examples::reply::PriceList;
use cicada_examples::saga::{ChargeResult, GatewayUnavailable, PaymentGateway};
use cicada_examples::singleton::TempFiles;
use cicada_examples::steps::{OnboardingServices, ServiceError};
use cicada_examples::sync::{Page, SyncApi};
use md5::{Digest, Md5};

const LOG: &str = "cicada_sample_service::fakes";

/// The fakes of one node, one per external system.
#[derive(Debug, Default)]
pub struct Fakes {
    /// For the cleanup singleton.
    pub temp_files: FakeTempFiles,
    /// For the cron jobs.
    pub cron_task: FakeCronTask,
    /// For pricing.
    pub price_list: FakePriceList,
    /// For payments.
    pub payment_gateway: FakePaymentGateway,
    /// For onboarding.
    pub onboarding: FakeOnboarding,
    /// For the batch items.
    pub item_processor: FakeItemProcessor,
    /// For the sync.
    pub sync_api: FakeSyncApi,
}

/// Logs the purge and deletes nothing.
#[derive(Debug, Default)]
pub struct FakeTempFiles;

impl TempFiles for FakeTempFiles {
    async fn purge_older_than(&self, cutoff: Timestamp, idempotency_key: &str) -> i32 {
        tracing::info!(target: LOG, "cleanup: purging temp files older than {cutoff} (key {idempotency_key})");
        0
    }
}

/// Logs the run.
#[derive(Debug, Default)]
pub struct FakeCronTask;

impl CronTask for FakeCronTask {
    async fn run(&self, job: &str, scheduled_for: Timestamp) -> Result<(), TaskError> {
        tracing::info!(target: LOG, "cron: running {job} for {scheduled_for}");
        Ok(())
    }
}

/// Apples 50, pears 80, bananas 25 cents; any other sku is unknown.
#[derive(Debug, Default)]
pub struct FakePriceList;

impl PriceList for FakePriceList {
    fn unit_price_cents(&self, sku: &str) -> Option<i64> {
        match sku {
            "apple" => Some(50),
            "pear" => Some(80),
            "banana" => Some(25),
            _ => None,
        }
    }
}

/// Declines amounts above 10 000 cents; unavailable on the first call of each charge of an amount ending in `13`.
#[derive(Debug, Default)]
pub struct FakePaymentGateway {
    /// Calls per idempotency key.
    calls: Arc<Mutex<HashMap<String, u32>>>,
}

impl PaymentGateway for FakePaymentGateway {
    async fn charge(
        &self,
        order_id: &str,
        amount_cents: i64,
        idempotency_key: &str,
    ) -> Result<ChargeResult, GatewayUnavailable> {
        let call = {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            let count = calls.entry(idempotency_key.to_owned()).or_default();
            *count += 1;
            *count
        };
        tracing::info!(
            target: LOG,
            "payments: charge {amount_cents} cents for {order_id} (key {idempotency_key}, call {call})"
        );
        if amount_cents % 100 == 13 && call == 1 {
            return Err(GatewayUnavailable(format!("gateway unavailable (call {call})")));
        }
        Ok(if amount_cents > 10_000 {
            ChargeResult::Declined { reason: "amount above limit".into() }
        } else {
            ChargeResult::Approved { transaction_id: format!("tx-{idempotency_key}") }
        })
    }
}

/// Derives ids from the idempotency keys; provisioning fails for keys containing `fail`.
#[derive(Debug, Default)]
pub struct FakeOnboarding;

impl OnboardingServices for FakeOnboarding {
    async fn create_account(&self, _email: &str, idempotency_key: &str) -> Result<String, ServiceError> {
        Ok(format!("acc-{}", short_id(idempotency_key)))
    }

    async fn send_welcome(&self, email: &str, _idempotency_key: &str) -> Result<(), ServiceError> {
        tracing::info!(target: LOG, "onboarding: welcome mail to {email}");
        Ok(())
    }

    async fn provision_workspace(&self, _account_id: &str, idempotency_key: &str) -> Result<String, ServiceError> {
        if idempotency_key.contains("fail") {
            return Err(ServiceError("workspace provisioning failed".into()));
        }
        Ok(format!("ws-{}", short_id(idempotency_key)))
    }
}

/// An item's result is its input's length.
#[derive(Debug, Default)]
pub struct FakeItemProcessor;

impl ItemProcessor for FakeItemProcessor {
    async fn process(&self, input: &str) -> i64 {
        // Kotlin's String.length counts UTF-16 units.
        input.encode_utf16().count() as i64
    }
}

/// Three pages: 10 items (next `p2`), 10 items (next `p3`), 5 items (last).
#[derive(Debug, Default)]
pub struct FakeSyncApi;

impl SyncApi for FakeSyncApi {
    async fn fetch(&self, source: &str, cursor: Option<&str>, key: &str) -> Page {
        tracing::info!(target: LOG, "sync: fetching {source} page {} (key {key})", cursor.unwrap_or("null"));
        match cursor {
            None => Page { size: 10, next: Some("p2".into()) },
            Some("p2") => Page { size: 10, next: Some("p3".into()) },
            Some(_) => Page { size: 5, next: None },
        }
    }
}

/// The first 8 characters of the name-based UUID of `key`, as Kotlin's `UUID.nameUUIDFromBytes(key)`: the first 4
/// bytes of the key's MD5 digest (the version bits come later).
fn short_id(key: &str) -> String {
    let digest = Md5::digest(key.as_bytes());
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ids_match_kotlin() {
        // UUID.nameUUIDFromBytes("abc".toByteArray()) = 90015098-3cd2-3fb0-9696-3f7d28e17f72
        assert_eq!(short_id("abc"), "90015098");
    }
}
