//! The wire formats of the three ready-made actors (ADR 0028): their contracts against the snapshots in
//! `tests/contracts`, the JSON their messages and states are written as, and sample checks for what the helpers and
//! the actors write: both the registered schemas and the decoders take it (ADR 0026).

mod common;

use cicada::actor::contract_of;
use cicada::prelude::*;
use cicada::{Failed, FailureCause, StopCause, Stopped, WatchTarget};
use cicada_actors::{
    Child, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, StoredMessage, Supervisor, SupervisorActor,
    SupervisorMsg, SupervisorState, Topic, TopicActor, TopicMsg, TopicState,
};
use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_test::TestActors;
use common::*;
use serde_json::{Value, json};

#[test]
fn the_contracts_of_the_three_types_match_their_snapshots() {
    let contracts = [
        contract_of::<SupervisorActor>().unwrap(),
        contract_of::<DeadLettersActor>().unwrap(),
        contract_of::<TopicActor>().unwrap(),
    ];
    let names: Vec<&str> = contracts.iter().map(|c| c.type_name().as_str()).collect();
    assert_eq!(names, ["cicada.supervisor", "cicada.dead-letters", "cicada.topic"]);
    cicada_test::assert_contracts_match(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/contracts"), &contracts);
}

fn failed() -> Failed {
    Failed {
        actor: addr(&worker("a")),
        message_id: MessageId::new("m-1").unwrap(),
        message_type: "Fail".into(),
        payload: json!({}),
        correlation_id: Some("c-1".into()),
        from: None,
        error: "boom".into(),
        attempts: 1,
        cause: FailureCause::Handler,
    }
}

fn stopped() -> Stopped {
    Stopped { actor: addr(&worker("a")), cause: StopCause::Deleted, by: Some(addr(&Supervisor::named("s"))) }
}

fn json_of(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn the_messages_are_written_in_camel_case_and_pass_their_samples() {
    let start = StoredMessage::for_ref(&worker("a"), WorkerMsg::Begin).unwrap();
    assert_eq!(json_of(&start), json!({"type": "Begin", "payload": {}}));
    let a = addr(&worker("a"));
    let supervisor = [
        (SupervisorMsg::Start, json!({"type": "Start"})),
        (SupervisorMsg::StartChild { actor: a.clone() }, json!({"type": "StartChild", "actor": "worker/a"})),
        (SupervisorMsg::Restarted { actor: a.clone() }, json!({"type": "Restarted", "actor": "worker/a"})),
        (
            SupervisorMsg::Escalate { actor: a.clone(), error: "e".into(), restart: true },
            json!({"type": "Escalate", "actor": "worker/a", "error": "e", "restart": true}),
        ),
    ];
    for (message, written) in supervisor {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<SupervisorMsg>(&written);
    }
    for message in [SupervisorMsg::Failed(failed()), SupervisorMsg::Stopped(stopped())] {
        assert_message_sample::<SupervisorMsg>(&json_of(message));
    }

    let id = MessageId::new("m-1").unwrap();
    let dead_letters = [
        (DeadLettersMsg::Redrive { message_id: id.clone() }, json!({"type": "Redrive", "messageId": "m-1"})),
        (DeadLettersMsg::Discard { message_id: id.clone() }, json!({"type": "Discard", "messageId": "m-1"})),
    ];
    for (message, written) in dead_letters {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<DeadLettersMsg>(&written);
    }
    for message in [DeadLettersMsg::Failed(failed()), DeadLettersMsg::Stopped(stopped())] {
        assert_message_sample::<DeadLettersMsg>(&json_of(message));
    }

    let note = TopicMsg::publish(WorkerMsg::Note { text: "hi".into() }).unwrap();
    let message = json!({"type": "Note", "payload": {"text": "hi"}});
    let topic = [
        (TopicMsg::Subscribe { subscriber: a.clone() }, json!({"type": "Subscribe", "subscriber": "worker/a"})),
        (TopicMsg::Unsubscribe { subscriber: a.clone() }, json!({"type": "Unsubscribe", "subscriber": "worker/a"})),
        (note.clone(), json!({"type": "Publish", "message": message})),
    ];
    for (message, written) in topic {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<TopicMsg>(&written);
    }
    let TopicMsg::Publish { message: stored } = note else { unreachable!() };
    let deliver = TopicMsg::Deliver { subscriber: a, message: stored };
    assert_eq!(json_of(&deliver), json!({"type": "Deliver", "subscriber": "worker/a", "message": message}));
    for message in [deliver, TopicMsg::Failed(failed()), TopicMsg::Stopped(stopped())] {
        assert_message_sample::<TopicMsg>(&json_of(message));
    }
}

#[test]
fn a_supervisor_state_built_by_the_helpers_is_written_in_camel_case_and_passes_its_sample() {
    let child = Child::new(&worker("a"), &WorkerState::default()).unwrap().start(WorkerMsg::Begin).unwrap();
    let state = SupervisorState::new().child(child).child(Child::new(&worker("b"), &WorkerState::default()).unwrap());
    let written = json_of(&state);
    assert_eq!(
        written,
        json!({
            "children": {
                "worker/a": {
                    "state": {"n": 0, "log": []},
                    "start": {"type": "Begin", "payload": {}},
                    "running": false,
                    "givenUp": false,
                    "restarts": []
                },
                "worker/b": {"state": {"n": 0, "log": []}, "running": false, "givenUp": false, "restarts": []}
            },
            "maxRestarts": 3,
            "withinSeconds": 60
        })
    );
    assert_state_sample::<SupervisorState>(&written);
    assert_state_sample::<SupervisorState>(&json!({"children": {}}));
    assert_state_sample::<DeadLettersState>(&json!({"letters": {}}));
    assert_state_sample::<TopicState>(&json!({"subscribers": []}));
}

/// The states that the actors write in a run, with a restart, a given-up child, a redriven letter and subscribers.
#[tokio::test(start_paused = true)]
async fn the_states_the_actors_write_pass_their_samples() {
    let test = TestActors::builder()
        .with(|node| node.host(SupervisorActor).host(DeadLettersActor::default()).host(TopicActor).host(WorkerActor))
        .build()
        .await
        .unwrap();
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    test.system().watch(&WatchTarget::Every, &addr(&DeadLetters::default())).await.unwrap();
    let sup = Supervisor::named("s");
    let children = SupervisorState::new()
        .child(Child::new(&worker("a"), &WorkerState::default()).unwrap().start(WorkerMsg::Begin).unwrap())
        .child(Child::new(&worker("b"), &WorkerState::default()).unwrap());
    test.create(&sup, &children).await;
    test.send(&sup, SupervisorMsg::Start).await;
    test.run_until_idle().await;
    test.send(&worker("a"), WorkerMsg::Fail).await;
    test.run_until_idle().await;
    test.send(&worker("b"), WorkerMsg::Poison).await;
    test.send(&worker("b"), WorkerMsg::Work).await;
    test.run_until_idle().await;
    let written = test.state_json(&sup).await.unwrap();
    assert_eq!(written["children"]["worker/a"]["restarts"].as_array().unwrap().len(), 1, "{written}");
    assert_eq!(written["children"]["worker/b"]["givenUp"], true, "{written}");
    assert!(written["children"]["worker/b"]["lastError"].is_string(), "{written}");
    assert_state_sample::<SupervisorState>(&written);

    let letters: DeadLettersState = test.state(&DeadLetters::default()).await;
    let first = letters.letters.keys().next().unwrap().clone();
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: first.clone() }).await;
    test.run_until_idle().await;
    let written = test.state_json(&DeadLetters::default()).await.unwrap();
    let letter = &written["letters"][first.as_str()];
    for key in ["actor", "messageType", "payload", "correlationId", "from", "error", "cause", "attempts", "recordedAt"]
    {
        assert!(letter.get(key).is_some(), "{key} in {letter}");
    }
    assert!(letter.get("redrivenAs").is_some() || letter.get("redriveError").is_some(), "{letter}");
    assert_state_sample::<DeadLettersState>(&written);

    let topic = Topic::named("orders");
    test.create(&topic, &TopicState::default()).await;
    test.send(&topic, TopicMsg::Subscribe { subscriber: addr(&worker("a")) }).await;
    test.run_until_idle().await;
    let written = test.state_json(&topic).await.unwrap();
    assert_eq!(written, json!({"subscribers": ["worker/a"]}));
    assert_state_sample::<TopicState>(&written);
}
