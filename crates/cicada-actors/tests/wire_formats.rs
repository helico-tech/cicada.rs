//! The wire formats of the six ready-made actors (ADR 0028): their contracts against the references in
//! `docs/reference/contracts`, the JSON their messages and states are written as, and sample checks for what the
//! helpers and the actors write: both the registered schemas and the decoders take it (ADR 0026).

mod common;

use std::path::{Path, PathBuf};

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::{Failed, FailureCause, StopCause, Stopped, WatchTarget};
use cicada_actors::{
    Child, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, Debouncer, DebouncerActor, DebouncerMsg,
    DebouncerState, RateLimiter, RateLimiterActor, RateLimiterMsg, RateLimiterState, Schedule, Scheduler,
    SchedulerActor, SchedulerMsg, SchedulerState, StoredMessage, Supervisor, SupervisorActor, SupervisorMsg,
    SupervisorState, Topic, TopicActor, TopicMsg, TopicState,
};
use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_test::{TestActors, UPDATE_CONTRACTS_ENV, describe_changes};
use common::*;
use serde_json::{Map, Value, json};

/// The references of the contracts: what a host depends on, so they live with the other references of the docs.
fn contracts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/contracts")
}

/// `value` with the keys of every object sorted, so that the text of a reference does not follow the order of a map.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&o[k]))).collect::<Map<_, _>>())
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// The text of the reference of `contract`: its JSON as `GET /types` serves it, pretty and with sorted keys.
fn reference_text(contract: &Contract) -> String {
    serde_json::to_string_pretty(&sorted(&contract.to_json())).unwrap() + "\n"
}

/// Compares each contract with its reference in `dir` as text, descriptions included, unlike
/// `cicada_test::assert_contracts_match`, which compares hashes and so leaves a reworded doc comment out. With
/// `update`, a reference that differs in any way, or is missing, is written instead. The error names each type and
/// what changed in its contract.
fn check_references(dir: &Path, contracts: &[Contract], update: bool) -> Result<(), String> {
    let mut problems = Vec::new();
    for contract in contracts {
        let file = dir.join(format!("{}.json", contract.type_name()));
        let current = reference_text(contract);
        let recorded = std::fs::read_to_string(&file).ok();
        if recorded.as_deref() == Some(current.as_str()) {
            continue;
        }
        if update {
            std::fs::write(&file, &current).map_err(|e| format!("Cannot write {}: {e}", file.display()))?;
            continue;
        }
        let changes = match recorded.as_deref().map(serde_json::from_str::<Value>) {
            None => vec!["the reference is missing".to_owned()],
            Some(Err(e)) => vec![format!("the reference is not JSON: {e}")],
            Some(Ok(json)) => match Contract::from_json(&json) {
                Err(e) => vec![format!("the reference is not a contract: {e}")],
                Ok(old) if old.hash() == contract.hash() => vec!["a description (doc comment) changed".to_owned()],
                Ok(old) => describe_changes(&old, contract),
            },
        };
        let lines: Vec<String> = changes.iter().map(|c| format!("  - {c}")).collect();
        problems.push(format!("{} ({}):\n{}", contract.type_name(), file.display(), lines.join("\n")));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Contracts differ from their references:\n{}\nIf the change is meant, rewrite them with \
             {UPDATE_CONTRACTS_ENV}=true and commit them.",
            problems.join("\n")
        ))
    }
}

fn the_six_contracts() -> [Contract; 6] {
    [
        contract_of::<SupervisorActor>().unwrap(),
        contract_of::<DeadLettersActor>().unwrap(),
        contract_of::<TopicActor>().unwrap(),
        contract_of::<SchedulerActor>().unwrap(),
        contract_of::<RateLimiterActor>().unwrap(),
        contract_of::<DebouncerActor>().unwrap(),
    ]
}

#[test]
fn the_contracts_of_the_six_types_match_their_references() {
    let contracts = the_six_contracts();
    let names: Vec<&str> = contracts.iter().map(|c| c.type_name().as_str()).collect();
    assert_eq!(
        names,
        [
            "cicada.supervisor",
            "cicada.dead-letters",
            "cicada.topic",
            "cicada.scheduler",
            "cicada.rate-limiter",
            "cicada.debouncer"
        ]
    );
    let update = std::env::var(UPDATE_CONTRACTS_ENV).is_ok_and(|v| v == "true");
    if let Err(e) = check_references(&contracts_dir(), &contracts, update) {
        panic!("{e}");
    }
}

/// A temporary folder that holds the reference of `contract`, as the main test writes it, so that the scenarios below
/// do not depend on the files in `docs/reference/contracts`, which a first run with the update variable writes.
fn folder_with(contract: &Contract) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(format!("{}.json", contract.type_name())), reference_text(contract)).unwrap();
    dir
}

/// `contract` with `edit` applied to its JSON.
fn edited(contract: &Contract, edit: impl FnOnce(&mut Value)) -> Contract {
    let mut json = contract.to_json();
    edit(&mut json);
    Contract::from_json(&json).unwrap()
}

/// Scenario: A field of a ready-made actor changes.
#[test]
fn a_renamed_field_of_the_state_of_the_supervisor_names_the_change() {
    let supervisor = contract_of::<SupervisorActor>().unwrap();
    let renamed = edited(&supervisor, |json| {
        let properties = json["state"]["properties"].as_object_mut().unwrap();
        let children = properties.remove("children").expect("the state has children");
        properties.insert("kids".into(), children);
    });
    let dir = folder_with(&supervisor);
    let error = check_references(dir.path(), &[renamed], false).unwrap_err();
    assert!(error.contains("cicada.supervisor") && error.contains("state schema changed"), "{error}");
    assert!(error.contains("CICADA_UPDATE_CONTRACTS=true"), "{error}");
}

/// Scenario: Only a doc comment changes.
#[test]
fn a_reworded_doc_comment_of_the_topic_fails_and_the_update_writes_the_new_text() {
    let topic = contract_of::<TopicActor>().unwrap();
    let reworded = edited(&topic, |json| {
        json["messages"]["Publish"]["description"] = json!("Publishes a message, reworded.");
    });
    assert_eq!(reworded.hash(), topic.hash(), "a description is not in the hash");
    let dir = folder_with(&topic);
    let error = check_references(dir.path(), std::slice::from_ref(&reworded), false).unwrap_err();
    assert!(error.contains("cicada.topic") && error.contains("a description (doc comment) changed"), "{error}");

    check_references(dir.path(), std::slice::from_ref(&reworded), true).unwrap();
    let rewritten = std::fs::read_to_string(dir.path().join("cicada.topic.json")).unwrap();
    assert!(rewritten.contains("Publishes a message, reworded."), "{rewritten}");
    assert_eq!(check_references(dir.path(), &[reworded], false), Ok(()));
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

    let slot: cicada::time::Timestamp = "2026-11-13T02:00:00Z".parse().unwrap();
    let every = Schedule::every(60).unwrap();
    let policy = RetryPolicy::new(3, Backoff::fixed(std::time::Duration::from_secs(300)).unwrap()).unwrap();
    let scheduler = [
        (SchedulerMsg::Start, json!({"type": "Start"})),
        (SchedulerMsg::Run { slot: None }, json!({"type": "Run", "slot": null})),
        (SchedulerMsg::Run { slot: Some(slot) }, json!({"type": "Run", "slot": "2026-11-13T02:00:00Z"})),
        (SchedulerMsg::Tick { slot }, json!({"type": "Tick", "slot": "2026-11-13T02:00:00Z"})),
        (
            SchedulerMsg::Update { schedule: None, target: None, message: None, retry: None },
            json!({"type": "Update", "schedule": null, "target": null, "message": null, "retry": null}),
        ),
        (
            SchedulerMsg::Update {
                schedule: Some(every),
                target: Some(addr(&worker("b"))),
                message: Some(StoredMessage::for_ref(&worker("b"), WorkerMsg::Note { text: "hi".into() }).unwrap()),
                retry: Some(policy),
            },
            json!({
                "type": "Update",
                "schedule": {"type": "every", "seconds": 60},
                "target": "worker/b",
                "message": message,
                "retry": {"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT5M"}}
            }),
        ),
    ];
    for (message, written) in scheduler {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<SchedulerMsg>(&written);
    }
    for written in [json!({"type": "Run"}), json!({"type": "Update", "message": message})] {
        assert_message_sample::<SchedulerMsg>(&written);
    }

    let fixed = json!({"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT10S"}});
    let two = RetryPolicy::new(2, Backoff::fixed(std::time::Duration::from_secs(10)).unwrap()).unwrap();
    let rate_limiter = [
        (
            RateLimiterMsg::submit(&worker("b"), WorkerMsg::Note { text: "hi".into() }, None).unwrap(),
            json!({"type": "Submit", "to": "worker/b", "message": message, "retry": null}),
        ),
        (
            RateLimiterMsg::submit(&worker("b"), WorkerMsg::Note { text: "hi".into() }, Some(two)).unwrap(),
            json!({"type": "Submit", "to": "worker/b", "message": message, "retry": fixed}),
        ),
    ];
    for (message, written) in rate_limiter {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<RateLimiterMsg>(&written);
    }
    assert_message_sample::<RateLimiterMsg>(&json!({"type": "Submit", "to": "worker/b", "message": message}));
    for invalid in [
        json!({"message": message}),
        json!({"to": "worker/b"}),
        json!({"to": "worker/b", "message": message, "retry": {"maxAttempts": 0, "backoff": fixed["backoff"]}}),
    ] {
        assert!(cicada::message::decode::<RateLimiterMsg>("Submit", &invalid).is_err(), "{invalid}");
    }

    let two = RetryPolicy::new(2, Backoff::fixed(std::time::Duration::from_secs(10)).unwrap()).unwrap();
    let debouncer = [
        (
            DebouncerMsg::submit(&worker("b"), WorkerMsg::Note { text: "hi".into() }, None).unwrap(),
            json!({"type": "Submit", "to": "worker/b", "message": message, "retry": null}),
        ),
        (
            DebouncerMsg::submit(&worker("b"), WorkerMsg::Note { text: "hi".into() }, Some(two)).unwrap(),
            json!({"type": "Submit", "to": "worker/b", "message": message, "retry": fixed}),
        ),
        (DebouncerMsg::Flush, json!({"type": "Flush"})),
    ];
    for (message, written) in debouncer {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<DebouncerMsg>(&written);
    }
    assert_message_sample::<DebouncerMsg>(&json!({"type": "Submit", "to": "worker/b", "message": message}));
    for invalid in [
        json!({"message": message}),
        json!({"to": "worker/b"}),
        json!({"to": "worker/b", "message": message, "retry": {"maxAttempts": 0, "backoff": fixed["backoff"]}}),
    ] {
        assert!(cicada::message::decode::<DebouncerMsg>("Submit", &invalid).is_err(), "{invalid}");
    }
}

#[test]
fn a_debouncer_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_states() {
    let second = std::time::Duration::from_secs(1);
    let state = DebouncerState::new(2 * second, 5 * second).unwrap();
    let written = json_of(&state);
    assert_eq!(written, json!({"quiet": "PT2S", "maxWait": "PT5S", "pending": null, "firstAt": null, "armed": null}));
    assert_state_sample::<DebouncerState>(&written);
    assert_state_sample::<DebouncerState>(&json!({"quiet": "PT2S", "maxWait": "PT5S"}));
    let pending = json!({
        "to": "worker/a",
        "message": {"type": "Note", "payload": {"text": "n"}},
        "retry": {"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT10S"}},
        "correlationId": "c"
    });
    let kept = json!({"quiet": "PT2S", "maxWait": "PT5S", "pending": pending, "firstAt": "2026-11-13T02:00:00Z",
        "armed": "f1"});
    assert_state_sample::<DebouncerState>(&kept);
    assert_state_sample::<DebouncerState>(&json!(
        {"quiet": "PT1S", "maxWait": "PT1S", "pending": {"to": "worker/a", "message": {"type": "Note", "payload": {}}},
            "firstAt": "2026-11-13T02:00:00Z"}
    ));
    for invalid in [
        json!({"quiet": "PT0S", "maxWait": "PT5S"}),
        json!({"quiet": "PT5S", "maxWait": "PT2S"}),
        json!({"quiet": "PT1S", "maxWait": "P365001D"}),
        json!({"quiet": "PT2S"}),
        json!({"quiet": 2, "maxWait": "PT5S"}),
        json!({"quiet": "PT2S", "maxWait": "PT5S", "pending": pending}),
        json!({"quiet": "PT2S", "maxWait": "PT5S", "firstAt": "2026-11-13T02:00:00Z"}),
        json!({"quiet": "PT2S", "maxWait": "PT5S", "armed": "f1"}),
        json!({"quiet": "PT2S", "maxWait": "PT5S", "pending": {"to": "worker/a"}, "firstAt": "2026-11-13T02:00:00Z"}),
        json!({"quiet": "PT2S", "maxWait": "PT5S", "pending": pending, "firstAt": "soon"}),
    ] {
        assert!(serde_json::from_value::<DebouncerState>(invalid.clone()).is_err(), "{invalid}");
    }
}

#[test]
fn a_rate_limiter_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_settings() {
    let second = std::time::Duration::from_secs(1);
    let state = RateLimiterState::new(2, second, 2, second).unwrap();
    let written = json_of(&state);
    assert_eq!(written, json!({"count": 2, "period": "PT1S", "burst": 2, "maxDelay": "PT1S", "idleAt": null}));
    assert_state_sample::<RateLimiterState>(&written);
    assert_state_sample::<RateLimiterState>(&json!({"count": 2, "period": "PT1S", "burst": 2, "maxDelay": "PT1S"}));
    assert_state_sample::<RateLimiterState>(&json!(
        {"count": 3, "period": "PT1M", "burst": 1, "maxDelay": "PT0S", "idleAt": "2026-11-13T02:00:00.333334Z"}
    ));
    for invalid in [
        json!({"count": 0, "period": "PT1S", "burst": 1, "maxDelay": "PT1S"}),
        json!({"count": 1, "period": "PT1S", "burst": 0, "maxDelay": "PT1S"}),
        json!({"count": 1, "period": "PT0S", "burst": 1, "maxDelay": "PT1S"}),
        json!({"count": 1, "period": "PT1S", "burst": 1, "maxDelay": "P365001D"}),
        json!({"count": 1, "period": "PT1S", "burst": 1, "maxDelay": "PT1S", "idleAt": "soon"}),
        json!({"count": 1, "period": 1000, "burst": 1, "maxDelay": "PT1S"}),
        json!({"period": "PT1S", "burst": 1, "maxDelay": "PT1S"}),
    ] {
        assert!(serde_json::from_value::<RateLimiterState>(invalid.clone()).is_err(), "{invalid}");
    }
}

#[test]
fn a_scheduler_state_built_by_new_is_written_in_camel_case_and_passes_its_sample() {
    let mut state =
        SchedulerState::new(Schedule::daily(2, 0).unwrap(), &worker("a"), WorkerMsg::Note { text: "n".into() })
            .unwrap();
    let written = json_of(&state);
    assert_eq!(
        written,
        json!({
            "schedule": {"type": "daily", "hour": 2, "minute": 0},
            "target": "worker/a",
            "message": {"type": "Note", "payload": {"text": "n"}},
            "retry": null,
            "nextRun": null,
            "armedTick": null,
            "runs": 0,
            "lastRun": null
        })
    );
    assert_state_sample::<SchedulerState>(&written);
    state.retry = Some(RetryPolicy::new(2, Backoff::fixed(std::time::Duration::from_secs(10)).unwrap()).unwrap());
    assert_state_sample::<SchedulerState>(&json_of(&state));
    assert_state_sample::<SchedulerState>(&json!({
        "schedule": {"type": "every", "seconds": 60},
        "target": "worker/a",
        "message": {"type": "Note", "payload": {"text": "n"}}
    }));
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

/// The states that the actors write in a run, with a restart, a given-up child, a redriven letter, subscribers and a
/// scheduler that delivered.
#[tokio::test(start_paused = true)]
async fn the_states_the_actors_write_pass_their_samples() {
    let test = TestActors::builder()
        .with(|node| {
            node.host(SupervisorActor)
                .host(DeadLettersActor::default())
                .host(TopicActor)
                .host(SchedulerActor)
                .host(RateLimiterActor)
                .host(DebouncerActor)
                .host(WorkerActor)
        })
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

    let nightly = Scheduler::named("nightly");
    let state =
        SchedulerState::new(Schedule::every(60).unwrap(), &worker("a"), WorkerMsg::Note { text: "n".into() }).unwrap();
    test.create(&nightly, &state).await;
    test.send(&nightly, SchedulerMsg::Start).await;
    test.advance_time_by(std::time::Duration::from_secs(60)).await;
    let written = test.state_json(&nightly).await.unwrap();
    for key in ["nextRun", "armedTick", "lastRun"] {
        assert!(written[key].is_string(), "{key} in {written}");
    }
    assert_eq!(written["runs"], 1, "{written}");
    assert_state_sample::<SchedulerState>(&written);

    let api = RateLimiter::named("api");
    let second = std::time::Duration::from_secs(1);
    test.create(&api, &RateLimiterState::new(3, second, 1, second).unwrap()).await;
    test.send(&api, RateLimiterMsg::submit(&worker("a"), WorkerMsg::Note { text: "n".into() }, None).unwrap()).await;
    test.run_until_idle().await;
    let written = test.state_json(&api).await.unwrap();
    assert!(written["idleAt"].is_string(), "{written}");
    assert_state_sample::<RateLimiterState>(&written);

    let deb = Debouncer::named("d");
    test.create(&deb, &DebouncerState::new(second, 5 * second).unwrap()).await;
    let submit = DebouncerMsg::submit(&worker("a"), WorkerMsg::Note { text: "n".into() }, None).unwrap();
    test.send_with(&deb, submit, SendOptions::new().correlation_id("c")).await;
    test.run_until_idle().await;
    let written = test.state_json(&deb).await.unwrap();
    for key in ["firstAt", "armed"] {
        assert!(written[key].is_string(), "{key} in {written}");
    }
    assert_eq!(written["pending"]["correlationId"], "c", "{written}");
    assert_state_sample::<DebouncerState>(&written);
    test.advance_time_by(second).await;
    let written = test.state_json(&deb).await.unwrap();
    assert_eq!(written, json!({"quiet": "PT1S", "maxWait": "PT5S", "pending": null, "firstAt": null, "armed": null}));
    assert_state_sample::<DebouncerState>(&written);
}
