//! The messages and states of the end-to-end types, as the types serialise them and as the end-to-end suite sends
//! them, pass both their registered schemas and their decoders: the HTTP door takes them (ADR 0026). The examples and
//! the dead letters have checks of their own crates.

use cicada::time::Timestamp;
use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_sample_service::e2e::{
    CounterMsg, CounterState, FlakyMsg, FlakyState, Recorded, RecorderMsg, RecorderState, SlowMsg, SlowRun, SlowState,
    StepEntry,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

fn message<M: Serialize + DeserializeOwned + JsonSchema>(message: M) {
    assert_message_sample::<M>(&serde_json::to_value(message).unwrap());
}

fn state<S: Serialize + DeserializeOwned + JsonSchema>(state: S) {
    assert_state_sample::<S>(&serde_json::to_value(state).unwrap());
}

fn t() -> Timestamp {
    "2026-10-03T12:00:00Z".parse().unwrap()
}

#[test]
fn slow() {
    message(SlowMsg::Sleep { millis: 100, label: Some("a".into()) });
    message(SlowMsg::Sleep { millis: 100, label: None });
    assert_message_sample::<SlowMsg>(&json!({"type": "Sleep", "millis": 100}));
    let run = SlowRun {
        message_id: "m".into(),
        label: Some("a".into()),
        attempt: 2,
        node: "n1".into(),
        claimed_at: t(),
        finished_at: t(),
    };
    state(SlowState::default());
    state(SlowState { runs: vec![run] });
}

#[test]
fn counter() {
    message(CounterMsg::Step { seq: 1, hold_millis: 0 });
    assert_message_sample::<CounterMsg>(&json!({"type": "Step", "seq": 2}));
    let entry = StepEntry { seq: 1, node: "n1".into(), attempt: 1, claimed_at: t(), finished_at: t() };
    state(CounterState::default());
    state(CounterState { count: 1, last_seq: 1, out_of_order: 0, overlaps: 0, entries: vec![entry] });
}

#[test]
fn recorder() {
    message(RecorderMsg::Record { label: "a".into(), sleep_millis: 10 });
    assert_message_sample::<RecorderMsg>(&json!({"type": "Record", "label": "b"}));
    let record = Recorded { label: "a".into(), message_id: "m".into(), attempt: 1, node: "n1".into() };
    state(RecorderState::default());
    state(RecorderState { records: vec![record] });
}

#[test]
fn flaky() {
    message(FlakyMsg::Attempt { times: 2 });
    state(FlakyState::default());
    state(FlakyState { succeeded: 1, last_attempt: 3 });
}
