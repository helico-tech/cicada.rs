//! The wire formats of the eight ready-made actors (ADR 0028): their contracts against the references in
//! `docs/reference/contracts`, the JSON their messages and states are written as, and sample checks for what the
//! helpers and the actors write: both the registered schemas and the decoders take it (ADR 0026). And the wire format
//! of a flow (ADR 0031): a test flow's contract, whose type name its host chooses, pinned in `tests/contracts`, with the
//! step and sleep entries of its journal.

mod common;

use std::path::{Path, PathBuf};

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::{Failed, FailureCause, StopCause, Stopped, WatchTarget};
use cicada_actors::{
    Batcher, BatcherActor, BatcherMsg, BatcherState, Child, DeadLetters, DeadLettersActor, DeadLettersMsg,
    DeadLettersState, Debouncer, DebouncerActor, DebouncerMsg, DebouncerState, Event, EventActor, EventMsg, EventState,
    Flow, FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, JournalEntry, RateLimiter, RateLimiterActor,
    RateLimiterMsg, RateLimiterState, Schedule, Scheduler, SchedulerActor, SchedulerMsg, SchedulerState, StoredMessage,
    Supervisor, SupervisorActor, SupervisorMsg, SupervisorState, Topic, TopicActor, TopicMsg, TopicState,
};
use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_test::{TestActors, UPDATE_CONTRACTS_ENV, describe_changes};
use common::*;
use serde::{Deserialize, Serialize};
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

fn the_eight_contracts() -> [Contract; 8] {
    [
        contract_of::<SupervisorActor>().unwrap(),
        contract_of::<DeadLettersActor>().unwrap(),
        contract_of::<TopicActor>().unwrap(),
        contract_of::<SchedulerActor>().unwrap(),
        contract_of::<RateLimiterActor>().unwrap(),
        contract_of::<DebouncerActor>().unwrap(),
        contract_of::<BatcherActor>().unwrap(),
        contract_of::<EventActor>().unwrap(),
    ]
}

#[test]
fn the_contracts_of_the_eight_types_match_their_references() {
    let contracts = the_eight_contracts();
    let names: Vec<&str> = contracts.iter().map(|c| c.type_name().as_str()).collect();
    assert_eq!(
        names,
        [
            "cicada.supervisor",
            "cicada.dead-letters",
            "cicada.topic",
            "cicada.scheduler",
            "cicada.rate-limiter",
            "cicada.debouncer",
            "cicada.batcher",
            "cicada.event"
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

    let batcher = [
        (BatcherMsg::add(json!({"row": 7})).unwrap(), json!({"type": "Add", "item": {"row": 7}})),
        (BatcherMsg::add(()).unwrap(), json!({"type": "Add", "item": null})),
        (BatcherMsg::add("a").unwrap(), json!({"type": "Add", "item": "a"})),
        (BatcherMsg::Flush, json!({"type": "Flush"})),
    ];
    for (message, written) in batcher {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<BatcherMsg>(&written);
    }
    assert!(cicada::message::decode::<BatcherMsg>("Add", &json!({})).is_err(), "an Add needs its item");

    let event = [
        (EventMsg::Subscribe { flow: "shop.email".into() }, json!({"type": "Subscribe", "flow": "shop.email"})),
        (EventMsg::Unsubscribe { flow: "shop.email".into() }, json!({"type": "Unsubscribe", "flow": "shop.email"})),
        (EventMsg::publish(json!({"order": 7})).unwrap(), json!({"type": "Publish", "input": {"order": 7}})),
        (EventMsg::publish(()).unwrap(), json!({"type": "Publish", "input": null})),
        (EventMsg::publish([1, 2]).unwrap(), json!({"type": "Publish", "input": [1, 2]})),
        (
            EventMsg::publish_batch([json!({"row": 1}), Value::Null]).unwrap(),
            json!({"type": "PublishBatch", "items": [{"row": 1}, null]}),
        ),
        (EventMsg::publish_batch(Vec::<Value>::new()).unwrap(), json!({"type": "PublishBatch", "items": []})),
    ];
    for (message, written) in event {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<EventMsg>(&written);
    }
    for (message_type, invalid) in [
        ("Publish", json!({})),
        ("PublishBatch", json!({})),
        ("PublishBatch", json!({"items": {"row": 1}})),
        ("Subscribe", json!({})),
        ("Unsubscribe", json!({"flow": 7})),
    ] {
        assert!(cicada::message::decode::<EventMsg>(message_type, &invalid).is_err(), "{message_type} {invalid}");
    }
}

#[test]
fn an_event_state_is_written_as_its_list_passes_its_samples_and_refuses_invalid_states() {
    let empty = json_of(EventState::default());
    assert_eq!(empty, json!({"flows": []}));
    assert_state_sample::<EventState>(&empty);
    assert_state_sample::<EventState>(&json!({}));
    let populated = json!({"flows": ["shop.email", "shop.fulfil"]});
    assert_state_sample::<EventState>(&populated);
    let held: EventState = serde_json::from_value(populated.clone()).unwrap();
    assert_eq!(json_of(&held), populated, "in subscription order");
    let full: Vec<String> = (0..100).map(|i| format!("shop.flow{i}")).collect();
    assert_state_sample::<EventState>(&json!({ "flows": full }));
    let over: Vec<String> = (0..101).map(|i| format!("shop.flow{i}")).collect();
    for invalid in [
        json!({ "flows": over }),
        json!({"flows": ["shop.email", "shop.email"]}),
        json!({"flows": ["Shop Email"]}),
        json!({"flows": [""]}),
        json!({"flows": "shop.email"}),
        json!({"flows": null}),
    ] {
        assert!(serde_json::from_value::<EventState>(invalid.clone()).is_err(), "{invalid}");
    }
}

#[test]
fn a_batcher_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_states() {
    let second = std::time::Duration::from_secs(1);
    let state = BatcherState::new(&worker("a"), "Items", 3, 10 * second).unwrap();
    let written = json_of(&state);
    let settings = json!({"target": "worker/a", "messageType": "Items", "maxItems": 3, "maxWait": "PT10S"});
    let mut full = settings.clone();
    full.as_object_mut()
        .unwrap()
        .extend(json!({"retry": null, "items": [], "firstAt": null, "armed": null}).as_object().unwrap().clone());
    assert_eq!(written, full);
    assert_state_sample::<BatcherState>(&written);
    assert_state_sample::<BatcherState>(&settings);
    let with = |extra: Value| {
        let mut json = settings.clone();
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    };
    let fixed = json!({"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT10S"}});
    let at = "2026-11-13T02:00:00Z";
    assert_state_sample::<BatcherState>(&with(
        json!({"retry": fixed, "items": [1, null], "firstAt": at, "armed": "f1"}),
    ));
    assert_state_sample::<BatcherState>(&with(json!({"items": [{"a": 1}], "firstAt": at})));
    for invalid in [
        with(json!({"maxItems": 0})),
        with(json!({"maxItems": 1001})),
        with(json!({"maxWait": "PT0S"})),
        with(json!({"maxWait": "P365001D"})),
        with(json!({"messageType": ""})),
        with(json!({"messageType": "I".repeat(201)})),
        with(json!({"target": "not an address"})),
        with(json!({"items": [1, 2, 3], "firstAt": at})),
        with(json!({"items": [1]})),
        with(json!({"firstAt": at})),
        with(json!({"armed": "f1"})),
        with(json!({"items": [1], "firstAt": "soon"})),
        with(json!({"retry": {"maxAttempts": 0, "backoff": fixed["backoff"]}})),
        json!({"target": "worker/a", "messageType": "Items", "maxItems": 3}),
    ] {
        assert!(serde_json::from_value::<BatcherState>(invalid.clone()).is_err(), "{invalid}");
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
                .host(BatcherActor)
                .host(EventActor)
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

    let bat = Batcher::named("b");
    let mut state = BatcherState::new(&worker("a"), "Items", 3, second).unwrap();
    state.retry = Some(RetryPolicy::new(2, Backoff::fixed(second).unwrap()).unwrap());
    test.create(&bat, &state).await;
    test.send(&bat, BatcherMsg::add(json!({"row": 1})).unwrap()).await;
    test.send(&bat, BatcherMsg::add(()).unwrap()).await;
    test.run_until_idle().await;
    let written = test.state_json(&bat).await.unwrap();
    for key in ["firstAt", "armed"] {
        assert!(written[key].is_string(), "{key} in {written}");
    }
    assert_eq!(written["items"], json!([{"row": 1}, null]), "{written}");
    assert_state_sample::<BatcherState>(&written);
    test.advance_time_by(second).await;
    let written = test.state_json(&bat).await.unwrap();
    assert_eq!((written["items"].clone(), written["armed"].clone()), (json!([]), Value::Null), "{written}");
    assert_state_sample::<BatcherState>(&written);

    let event = Event::named("e");
    test.create(&event, &EventState::default()).await;
    for flow in ["shop.email", "shop.fulfil", "shop.email"] {
        test.send(&event, EventMsg::Subscribe { flow: flow.into() }).await;
    }
    test.send(&event, EventMsg::Unsubscribe { flow: "shop.email".into() }).await;
    test.run_until_idle().await;
    let written = test.state_json(&event).await.unwrap();
    assert_eq!(written, json!({"flows": ["shop.fulfil"]}));
    assert_state_sample::<EventState>(&written);
}

/// The input of the test flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct SignUp {
    /// The user who signs up.
    user: String,
}

/// The test flow, `test.sign-up`: an account, a minute's sleep, then a welcome.
struct SignUpFlow;

impl Flow for SignUpFlow {
    const TYPE: &'static str = "test.sign-up";
    type Input = SignUp;

    async fn run(&self, step: &mut cicada_actors::Step, input: SignUp) -> HandlerResult {
        let account: String = step.run("account", |key| async move { Ok(format!("{}@{key}", input.user)) }).await?;
        step.sleep("cool-off", std::time::Duration::from_secs(60)).await?;
        step.run("welcome", |_key| async move { Ok(account.len()) }).await?;
        Ok(())
    }
}

/// A flow's contract is the host's: pinned with the actor tests, not in `docs/reference/contracts`. It holds the sleep
/// entry of the journal, so a flow that sleeps, as this one does since sleeps were added, has the fixture of before.
#[test]
fn the_contract_of_a_test_flow_matches_its_fixture() {
    let contract = contract_of::<FlowActor<SignUpFlow>>().unwrap();
    assert_eq!(contract.type_name().as_str(), "test.sign-up");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
    let update = std::env::var(UPDATE_CONTRACTS_ENV).is_ok_and(|v| v == "true");
    if let Err(e) = check_references(&fixtures, std::slice::from_ref(&contract), update) {
        panic!("{e}");
    }
    let state = contract.to_json()["state"].to_string();
    for word in ["\"sleep\"", "\"until\"", "\"step\"", "\"result\"", "\"running\"", "\"done\"", "\"armed\""] {
        assert!(state.contains(word), "{word} in {state}");
    }
    assert!(!contracts_dir().join("test.sign-up.json").exists(), "no fixed type name in the catalog");
}

#[test]
fn a_flow_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_states() {
    type State = FlowState<SignUp>;
    let input = json!({"user": "u1"});
    let initial = json_of(FlowState::new(SignUp { user: "u1".into() }));
    assert_eq!(initial, json!({"input": input, "journal": [], "status": "running", "armed": null}));
    assert_state_sample::<State>(&initial);
    assert_state_sample::<State>(&json!({"input": input}));
    let account = json!({"kind": "step", "id": "account", "result": "u1@test.sign-up/u1/account"});
    let progress = json!({"input": input, "journal": [account], "status": "running", "armed": "next-message"});
    assert_state_sample::<State>(&progress);
    let notify = json!({"kind": "step", "id": "notify", "result": null});
    let wait = json!({"kind": "sleep", "id": "wait", "until": "2026-11-14T00:00:00Z"});
    let slept = json!({"input": input, "journal": [notify, wait]});
    assert_state_sample::<State>(&slept);
    let decoded: State = serde_json::from_value(slept.clone()).unwrap();
    let until = "2026-11-14T00:00:00Z".parse().unwrap();
    assert_eq!(decoded.journal[1], JournalEntry::Sleep { id: "wait".into(), until });
    let written = json!({"input": input, "journal": slept["journal"], "status": "running", "armed": null});
    assert_eq!(json_of(&decoded), written, "the sleep round-trips");
    let welcome = json!({"kind": "step", "id": "welcome", "result": 23});
    let done = json!({"input": input, "journal": [account, welcome], "status": "done", "armed": null});
    assert_state_sample::<State>(&done);
    assert_eq!(serde_json::from_value::<State>(done).unwrap().status, FlowStatus::Done);

    let many: Vec<Value> = (0..1001).map(|i| json!({"kind": "step", "id": format!("s{i}"), "result": i})).collect();
    let entry = |entry: Value| json!({"input": input, "journal": [entry]});
    for invalid in [
        json!({}),
        json!({"input": {"user": 1}}),
        json!({"input": input, "journal": [account, account]}),
        json!({"input": input, "journal": [notify, {"kind": "sleep", "id": "notify", "until": "2026-11-14T00:00:00Z"}]}),
        entry(json!({"kind": "step", "id": "a/b", "result": 1})),
        entry(json!({"kind": "step", "id": "", "result": 1})),
        entry(json!({"kind": "step", "id": "x".repeat(101), "result": 1})),
        entry(json!({"kind": "step", "id": "a"})),
        entry(json!({"kind": "wait", "id": "a"})),
        entry(json!({"kind": "sleep", "id": "a", "until": "soon"})),
        entry(json!({"kind": "sleep", "id": "a", "until": "+10000-01-01T00:00:00Z"})),
        json!({"input": input, "journal": many}),
        json!({"input": input, "status": "paused"}),
        json!({"input": input, "status": "done", "armed": "m1"}),
    ] {
        assert!(serde_json::from_value::<State>(invalid.clone()).is_err(), "{invalid}");
    }
}

#[test]
fn a_flow_message_and_saved_results_read_back() {
    assert_eq!(json_of(FlowMsg::Continue), json!({"type": "Continue"}));
    assert_message_sample::<FlowMsg>(&json!({"type": "Continue"}));
    assert_eq!(FlowRun::<SignUpFlow>::named("u1").address().unwrap().as_str(), "test.sign-up/u1");
    let state: FlowState<SignUp> = serde_json::from_value(json!({"input": {"user": "u1"}, "journal": [
        {"kind": "step", "id": "account", "result": "a1"},
        {"kind": "step", "id": "notify", "result": null},
        {"kind": "sleep", "id": "wait", "until": "2026-11-14T00:00:00Z"},
    ]}))
    .unwrap();
    assert_eq!(state.result::<String>("account").unwrap().unwrap(), "a1");
    state.result::<()>("notify").unwrap().unwrap();
    assert_eq!(state.result::<Option<String>>("notify").unwrap().unwrap(), None);
    assert!(state.result::<u64>("account").unwrap().is_err(), "a bad type is an error");
    assert!(state.result::<Value>("wait").is_none(), "a sleep is no step");
    assert!(state.result::<Value>("absent").is_none());
}

/// The states a run writes on its way pass their samples.
#[tokio::test(start_paused = true)]
async fn the_states_a_flow_writes_pass_their_samples() {
    let test = TestActors::builder().with(|node| node.host(FlowActor::new(SignUpFlow))).build().await.unwrap();
    let run = FlowRun::<SignUpFlow>::named("u1");
    test.create(&run, &FlowState::new(SignUp { user: "u1".into() })).await;
    test.send(&run, FlowMsg::Continue).await;
    for _ in 0..3 {
        test.system().run_due_turns(1).await.unwrap();
        assert_state_sample::<FlowState<SignUp>>(&test.state_json(&run).await.unwrap());
    }
    let account = json!({"kind": "step", "id": "account", "result": "u1@test.sign-up/u1/account"});
    let sleep = json!({"kind": "sleep", "id": "cool-off", "until": "2026-01-01T00:01:00Z"});
    assert_eq!(test.state_json(&run).await.unwrap()["journal"], json!([account, sleep]));
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    for _ in 0..2 {
        test.system().run_due_turns(1).await.unwrap();
        assert_state_sample::<FlowState<SignUp>>(&test.state_json(&run).await.unwrap());
    }
    let written = test.state_json(&run).await.unwrap();
    assert_eq!(written["status"], "done");
    assert_eq!(written["journal"], json!([account, sleep, {"kind": "step", "id": "welcome", "result": 26}]));
}
