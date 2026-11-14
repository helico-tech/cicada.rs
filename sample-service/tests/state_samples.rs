//! Stored state of some of this service's types, against the samples in `tests/states`. A
//! state type change that would make existing instances undecodable fails here. Samples are recorded once and kept;
//! add one when a state type changes shape.

use std::path::PathBuf;

use cicada::FailureCause;
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::MessageId;
use cicada::time::Timestamp;
use cicada_actors::{
    BatcherState, CreateKind, DeadLetter, DeadLettersState, DebouncedMessage, DebouncerState, EventState, FlowReply,
    FlowState, JournalEntry, LetterPayload, RateLimiterState, Schedule, SchedulerState, StoredMessage, WaitMode,
    WaitOutcome,
};
use cicada_examples::cron::CronState;
use cicada_examples::flow_onboarding::OnboardingInput;
use cicada_examples::timer::{CheckoutState, CheckoutStatus};
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
    let awaiting = CheckoutState {
        status: CheckoutStatus::AwaitingPayment,
        deadline: Some(t()),
        timeout: Some(MessageId::new("0b9f6a52-5c1e-4a55-9d0c-3f1f8f0f2a11").unwrap()),
        paid_cents: None,
    };
    record_state_sample(&dir, "awaiting-payment", &awaiting);
    let paid = CheckoutState { status: CheckoutStatus::Paid, paid_cents: Some(1250), ..CheckoutState::default() };
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
fn scheduler() {
    let dir = dir("scheduler");
    let state = SchedulerState {
        schedule: Schedule::daily(2, 0).unwrap(),
        target: "reports/_".parse().unwrap(),
        message: StoredMessage { message_type: "Build".into(), payload: json!({"full": true}) },
        retry: Some(RetryPolicy::new(3, Backoff::fixed(std::time::Duration::from_secs(300)).unwrap()).unwrap()),
        next_run: Some(t()),
        armed_tick: Some(MessageId::new("0b9f6a52-5c1e-4a55-9d0c-3f1f8f0f2a12").unwrap()),
        runs: 4,
        last_run: Some(t()),
    };
    record_state_sample(&dir, "armed-daily", &state);
    assert_state_decodes::<SchedulerState>(&dir);
}

#[test]
fn rate_limiter() {
    let dir = dir("rate-limiter");
    let second = std::time::Duration::from_secs(1);
    let state = RateLimiterState { idle_at: Some(t()), ..RateLimiterState::new(3, second, 2, 10 * second).unwrap() };
    record_state_sample(&dir, "reserved", &state);
    assert_state_decodes::<RateLimiterState>(&dir);
}

#[test]
fn debouncer() {
    let dir = dir("debouncer");
    let second = std::time::Duration::from_secs(1);
    let message = StoredMessage { message_type: "Note".into(), payload: json!({"text": "n"}) };
    let pending = DebouncedMessage {
        to: cicada::ActorAddress::parse("worker/a").unwrap(),
        message,
        retry: None,
        correlation_id: Some("c".into()),
    };
    let state = DebouncerState {
        pending: Some(pending),
        first_at: Some(t()),
        armed: Some(MessageId::new("f1").unwrap()),
        ..DebouncerState::new(2 * second, 5 * second).unwrap()
    };
    record_state_sample(&dir, "pending", &state);
    assert_state_decodes::<DebouncerState>(&dir);
}

#[test]
fn batcher() {
    let dir = dir("batcher");
    let state = BatcherState {
        target: cicada::ActorAddress::parse("worker/a").unwrap(),
        message_type: "Items".into(),
        max_items: 3,
        max_wait: std::time::Duration::from_secs(10),
        retry: Some(RetryPolicy::new(2, Backoff::fixed(std::time::Duration::from_secs(10)).unwrap()).unwrap()),
        items: vec![json!({"row": 1}), json!(null)],
        first_at: Some(t()),
        armed: Some(MessageId::new("f1").unwrap()),
    };
    record_state_sample(&dir, "pending", &state);
    assert_state_decodes::<BatcherState>(&dir);
}

#[test]
fn event() {
    let dir = dir("event");
    let state = EventState { flows: vec!["flows.onboarding".into(), "shop.fulfil".into()] };
    record_state_sample(&dir, "subscribed", &state);
    assert_state_decodes::<EventState>(&dir);
}

#[test]
fn flow_onboarding() {
    let dir = dir("flow-onboarding");
    let state = FlowState {
        journal: vec![
            JournalEntry::Step { id: "create-account".into(), result: json!("acc-1") },
            JournalEntry::Step { id: "send-welcome".into(), result: json!(null) },
        ],
        armed: Some(MessageId::new("0b9f6a52-5c1e-4a55-9d0c-3f1f8f0f2a13").unwrap()),
        ..FlowState::new(OnboardingInput { email: "bob@example.test".into() })
    };
    record_state_sample(&dir, "two-steps-saved", &state);
    let mut sleeping = state.clone();
    let until = "2026-11-14T00:01:00Z".parse().unwrap();
    sleeping.journal.push(JournalEntry::Sleep { id: "cool-off".into(), until });
    record_state_sample(&dir, "sleeping", &sleeping);
    // The wire of 0.18.0 (ADR 0044): a send, a request answered with a null value, and the wait that selected it.
    // The two samples above, written by 0.17, have neither `replies` nor `cleanup`, and still decode.
    let request = MessageId::new("5d1c2b8e-1f0a-4e6b-9a7c-2b3d4e5f6a7b").unwrap();
    let mut replied = sleeping.clone();
    replied.journal.extend([
        JournalEntry::Send {
            id: "notify".into(),
            to: "mailers/bob".parse().unwrap(),
            message: MessageId::new("7e2d3c9f-2a1b-4f7c-8b8d-3c4e5f6a7b8c").unwrap(),
            create: Some(CreateKind::Child),
            reply: false,
        },
        JournalEntry::Send {
            id: "review".into(),
            to: "reviews/bob".parse().unwrap(),
            message: request.clone(),
            create: None,
            reply: true,
        },
        JournalEntry::Wait {
            id: "reviewed".into(),
            requests: vec![request.clone()],
            mode: WaitMode::All,
            until: None,
            outcome: Some(WaitOutcome::Received),
            received: vec![request.clone()],
        },
    ]);
    replied.replies.push(FlowReply { request, value: json!(null) });
    record_state_sample(&dir, "sent-and-replied", &replied);
    assert_state_decodes::<FlowState<OnboardingInput>>(&dir);
}

/// The samples of the dead letters start again with 0.14.0: their contract changed, and the letters of 0.13.0 are lost
/// in the upgrade (the operations guide, "Upgrading to 0.14.0"). From 0.17.0 a letter holds its payload as a value, as
/// before, or as JSON text in `payloadJson` (ADR 0043).
#[test]
fn dead_letters() {
    let dir = dir("dead-letters");
    let letter = DeadLetter {
        actor: "mailers/m".parse().unwrap(),
        message_type: "SendMail".into(),
        payload: LetterPayload::Value(json!({"to": "a@example.test"})),
        correlation_id: None,
        from: None,
        error: "smtp down".into(),
        cause: FailureCause::Handler,
        attempts: 1,
        recorded_at: t(),
        seq: 17,
        redriven_as: Some(MessageId::new("m-2").unwrap()),
        redrive_error: None,
    };
    let state = DeadLettersState { letters: [(MessageId::new("m-1").unwrap(), letter)].into() };
    record_state_sample(&dir, "one-letter", &state);
    let mut text = state.clone();
    for letter in text.letters.values_mut() {
        letter.payload = LetterPayload::Json(r#"{"to":"a@example.test"}"#.into());
    }
    record_state_sample(&dir, "payload-json", &text);
    assert_state_decodes::<DeadLettersState>(&dir);
}
