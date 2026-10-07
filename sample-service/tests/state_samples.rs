//! Port of StateSamplesTest: stored state of some of this service's types, against the samples in `tests/states`. A
//! state type change that would make existing instances undecodable fails here. Samples are recorded once and kept;
//! add one when a state type changes shape.
//!
//! The `kotlin-*.json` samples are copies of the Kotlin sample service's samples: state written by Kotlin nodes must
//! decode too, since both implementations share one store schema (ADR 0004).

use std::path::PathBuf;

use cicada::time::Timestamp;
use cicada_examples::cron::CronState;
use cicada_examples::timer::{CheckoutState, CheckoutStatus};
use cicada_extras::{DeadLetter, DeadLettersState, Schedule, Timer};
use cicada_test::{assert_state_decodes, record_state_sample};
use serde_json::json;

fn dir(name: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/states")).join(name)
}

fn t() -> Timestamp {
    "2026-10-03T12:00:00Z".parse().unwrap()
}

#[test]
fn checkout() {
    let dir = dir("checkout");
    let mut armed = Timer::new();
    armed.arm(t());
    let awaiting = CheckoutState { status: CheckoutStatus::AwaitingPayment, timer: armed, ..CheckoutState::default() };
    record_state_sample(&dir, "awaiting-payment", &awaiting);
    let mut disarmed = Timer::new();
    disarmed.arm(t());
    disarmed.disarm();
    let paid = CheckoutState {
        status: CheckoutStatus::Paid,
        timer: disarmed,
        paid_cents: Some(1250),
        ..CheckoutState::default()
    };
    record_state_sample(&dir, "paid", &paid);
    assert_state_decodes::<CheckoutState>(&dir);
}

#[test]
fn cron() {
    let dir = dir("cron");
    let state = CronState {
        next_run: Some(t()),
        runs: 4,
        last_run: Some(t()),
        ..CronState::new(Schedule::daily(2, 30).unwrap())
    };
    record_state_sample(&dir, "armed-daily", &state);
    assert_state_decodes::<CronState>(&dir);
}

#[test]
fn dead_letters() {
    let dir = dir("dead-letters");
    let letter = DeadLetter {
        failed_message_id: "m-1".into(),
        actor: "actor://mailers/m".into(),
        message_type: "SendMail".into(),
        payload: json!("x"),
        correlation_id: None,
        error: Some("smtp down".into()),
        failed_at: t(),
        redriven_as: None,
        redrive_error: None,
    };
    let state = DeadLettersState { letters: [("m-1".to_owned(), letter)].into() };
    record_state_sample(&dir, "one-letter", &state);
    assert_state_decodes::<DeadLettersState>(&dir);
}
