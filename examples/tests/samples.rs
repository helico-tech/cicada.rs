//! Every message and state of the examples, as the types serialise them, passes both its registered schema and its
//! decoder: the HTTP door takes what the typed client sends (ADR 0026).

use std::collections::BTreeMap;

use cicada::store::MessageId;
use cicada::time::Timestamp;
use cicada_actors::Schedule;
use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_examples::cron::{CronMsg, CronState};
use cicada_examples::debounce::{DocumentMsg, DocumentState, IndexerMsg};
use cicada_examples::fanout::{BatchItemMsg, BatchItemState, BatchMsg, BatchState, BatchStatus};
use cicada_examples::rate_limit::{SignupMsg, SignupState, SmsMsg};
use cicada_examples::registry::{Device, DeviceMsg, DeviceState, RegistryMsg, RegistryState};
use cicada_examples::reply::{CartLine, CartMsg, CartState, PendingLine, PricingMsg, PricingReply};
use cicada_examples::saga::{InventoryMsg, InventoryState, Order, OrderMsg, OrderState, OrderStatus, PaymentsMsg};
use cicada_examples::singleton::CleanupMsg;
use cicada_examples::steps::{OnboardingMsg, OnboardingState, OnboardingStep};
use cicada_examples::supervision::{MailerMsg, MailerState};
use cicada_examples::sync::{ReportsMsg, ReportsState, SyncMsg, SyncState};
use cicada_examples::timer::{CheckoutMsg, CheckoutState, CheckoutStatus};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

fn messages<M: Serialize + DeserializeOwned + JsonSchema>(all: impl IntoIterator<Item = M>) {
    for message in all {
        assert_message_sample::<M>(&serde_json::to_value(message).unwrap());
    }
}

fn states<S: Serialize + DeserializeOwned + JsonSchema>(all: impl IntoIterator<Item = S>) {
    for state in all {
        assert_state_sample::<S>(&serde_json::to_value(state).unwrap());
    }
}

fn t() -> Timestamp {
    "2026-10-03T12:00:00Z".parse().unwrap()
}

fn id() -> MessageId {
    MessageId::new("0b9f6a52-5c1e-4a55-9d0c-3f1f8f0f2a11").unwrap()
}

#[test]
fn cron() {
    messages([CronMsg::Start, CronMsg::Tick { scheduled_for: t() }, CronMsg::RunNow, CronMsg::StopJob]);
    let armed = CronState {
        next_run: Some(t()),
        runs: 4,
        last_run: Some(t()),
        failed_runs: 1,
        last_error: Some("boom".into()),
        armed_tick: Some(id()),
        ..CronState::new(Schedule::daily(2, 30).unwrap())
    };
    states([CronState::new(Schedule::every(3600).unwrap()), armed]);
}

#[test]
fn fanout() {
    messages([
        BatchMsg::StartBatch { items: vec!["a".into(), "b".into()] },
        BatchMsg::ItemDone { index: 1, result: 7 },
    ]);
    messages([BatchItemMsg::ProcessItem]);
    let running = BatchState { status: BatchStatus::Running, total: 2, results: BTreeMap::from([(0, 5), (-1, 9)]) };
    states([BatchState::default(), running]);
    states([BatchItemState { input: "a".into() }]);
}

#[test]
fn registry() {
    messages([
        RegistryMsg::RegisterDevice { device_id: "d1".into(), model: "m".into() },
        RegistryMsg::UnregisterDevice { device_id: "d1".into() },
        RegistryMsg::BroadcastFirmware { version: "2.0".into() },
    ]);
    messages([DeviceMsg::UpdateFirmware { version: "2.0".into() }, DeviceMsg::Decommission]);
    states([
        RegistryState::default(),
        RegistryState { devices: BTreeMap::from([("d1".into(), Device { id: "d1".into() })]) },
    ]);
    states([DeviceState { model: "m".into(), registered_at: t(), firmware: Some("1.0".into()) }]);
}

#[test]
fn reply() {
    messages([
        CartMsg::AddItem { sku: "s".into(), quantity: 2 },
        CartMsg::Quote { sku: "s".into(), quantity: 2, unit_price_cents: 100, total_cents: 200 },
        CartMsg::QuoteRejected { sku: "s".into(), reason: "gone".into() },
    ]);
    messages([PricingMsg::QuoteRequest { sku: "s".into(), quantity: 2 }]);
    messages([
        PricingReply::Quote { sku: "s".into(), quantity: 2, unit_price_cents: 100, total_cents: 200 },
        PricingReply::QuoteRejected { sku: "s".into(), reason: "gone".into() },
    ]);
    let full = CartState {
        lines: vec![CartLine { sku: "s".into(), quantity: 2, total_cents: 200 }],
        awaiting: BTreeMap::from([("t".into(), PendingLine { sku: "t".into(), quantity: 1 })]),
        rejected: BTreeMap::from([("u".into(), "gone".into())]),
    };
    states([CartState::default(), full]);
}

#[test]
fn saga() {
    let order = || Order { id: "o1".into() };
    messages([
        InventoryMsg::Reserve { reply_to: order(), quantity: 2 },
        InventoryMsg::Release { reply_to: order() },
        InventoryMsg::Restock { quantity: 5 },
    ]);
    states([InventoryState { available: 3, reservations: BTreeMap::from([("o1".into(), 2)]) }]);
    messages([
        OrderMsg::PlaceOrder { sku: "s".into(), quantity: 2, amount_cents: 500 },
        OrderMsg::Reserved,
        OrderMsg::OutOfStock { available: 1 },
        OrderMsg::Charged { transaction_id: "tx".into() },
        OrderMsg::ChargeDeclined { reason: "no".into() },
        OrderMsg::Released,
    ]);
    let cancelled = OrderState {
        status: OrderStatus::Cancelled,
        sku: Some("s".into()),
        quantity: 2,
        amount_cents: 500,
        transaction_id: Some("tx".into()),
        failure: Some("no".into()),
    };
    states([OrderState::default(), cancelled]);
    messages([PaymentsMsg::Charge { reply_to: order(), amount_cents: 500 }]);
}

#[test]
fn singleton() {
    messages([CleanupMsg::Purge { retention_hours: 24 }]);
}

#[test]
fn steps() {
    messages([
        OnboardingMsg::BeginOnboarding { email: "a@b.c".into() },
        OnboardingMsg::RunStep { step: OnboardingStep::SendWelcome },
        OnboardingMsg::Resume,
    ]);
    let done = OnboardingState {
        step: OnboardingStep::Done,
        email: Some("a@b.c".into()),
        account_id: Some("acc".into()),
        workspace_id: Some("ws".into()),
    };
    states([OnboardingState::default(), done]);
}

#[test]
fn supervision() {
    messages([MailerMsg::SendMail { to: "a@b.c".into() }]);
    states([MailerState::default(), MailerState { sent: vec!["a@b.c".into()] }]);
}

#[test]
fn rate_limit() {
    messages([SignupMsg::Register { phone: "+31 6 1".into() }, SignupMsg::Texted]);
    states([SignupState::default(), SignupState { phone: Some("+31 6 1".into()), texted: true }]);
    let reply_to = "signups/1".parse().unwrap();
    messages([SmsMsg::Send { phone: "+31 6 1".into(), text: "Welcome!".into(), reply_to }]);
}

#[test]
fn debounce() {
    messages([DocumentMsg::Edit { text: "abc".into() }, DocumentMsg::Indexed { version: 3 }]);
    states([DocumentState::default(), DocumentState { text: "abc".into(), version: 3, indexed: Some(3) }]);
    let reply_to = "documents/d1".parse().unwrap();
    messages([IndexerMsg::Reindex { doc: "d1".into(), version: 3, text: "abc".into(), reply_to }]);
}

#[test]
fn sync() {
    messages([SyncMsg::Run, SyncMsg::FetchPage { cursor: None }, SyncMsg::FetchPage { cursor: Some("c".into()) }]);
    states([SyncState { source: "erp".into(), imported: 3 }]);
    messages([ReportsMsg::SyncFinished { imported: 3 }]);
    states([ReportsState::default(), ReportsState { runs: vec![1, 2] }]);
}

#[test]
fn timer() {
    messages([
        CheckoutMsg::Begin { timeout_seconds: 900 },
        CheckoutMsg::Extend { seconds: 60 },
        CheckoutMsg::Pay { amount_cents: 1250 },
        CheckoutMsg::PaymentTimeout,
    ]);
    let awaiting = CheckoutState {
        status: CheckoutStatus::AwaitingPayment,
        deadline: Some(t()),
        timeout: Some(id()),
        paid_cents: None,
    };
    let paid = CheckoutState { status: CheckoutStatus::Paid, paid_cents: Some(1250), ..CheckoutState::default() };
    states([CheckoutState::default(), awaiting, paid]);
}
