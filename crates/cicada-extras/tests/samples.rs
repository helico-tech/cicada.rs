//! The messages and the state of the dead letters, as the types serialise them, pass both their registered schemas and
//! their decoders: the HTTP door takes what the typed client sends (ADR 0026).

use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_extras::{DeadLetter, DeadLettersMsg, DeadLettersState};
use serde_json::json;

fn letter() -> DeadLetter {
    DeadLetter {
        failed_message_id: "m-1".into(),
        actor: "mailers/m".into(),
        message_type: "SendMail".into(),
        payload: json!({"to": "a@b.c"}),
        correlation_id: Some("c-1".into()),
        error: Some("smtp down".into()),
        failed_at: "2026-10-03T12:00:00Z".parse().unwrap(),
        redriven_as: Some("m-2".into()),
        redrive_error: None,
    }
}

#[test]
fn dead_letters() {
    for message in [
        DeadLettersMsg::RecordFailure { letter: letter() },
        DeadLettersMsg::Redrive { failed_message_id: "m-1".into() },
        DeadLettersMsg::Discard { failed_message_id: "m-1".into() },
    ] {
        assert_message_sample::<DeadLettersMsg>(&serde_json::to_value(message).unwrap());
    }
    for state in [DeadLettersState::default(), DeadLettersState { letters: [("m-1".to_owned(), letter())].into() }] {
        assert_state_sample::<DeadLettersState>(&serde_json::to_value(state).unwrap());
    }
}
