//! The wire formats of the twelve ready-made actors (ADR 0028): their contracts against the references in
//! `docs/reference/contracts`, the JSON their messages and states are written as, and sample checks for what the
//! helpers and the actors write: both the registered schemas and the decoders take it (ADR 0026). And the wire format
//! of a flow (ADR 0031, ADR 0044): a test flow's contract, whose type name its host chooses, pinned in `tests/contracts`,
//! with the step, sleep, send and wait entries of its journals, its replies, its four statuses and its three messages.

mod common;

use std::path::{Path, PathBuf};

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::{Failed, FailureCause, StopCause, Stopped, WatchTarget};
use cicada_actors::Job as GraphJob;
use cicada_actors::{
    Batcher, BatcherActor, BatcherMsg, BatcherState, Child, DeadLetters, DeadLettersActor, DeadLettersMsg,
    DeadLettersState, Debouncer, DebouncerActor, DebouncerMsg, DebouncerState, Event, EventActor, EventMsg, EventState,
    Flow, FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, HeartbeatStatus, HeartbeatWatch, HeartbeatWatchActor,
    HeartbeatWatchMsg, HeartbeatWatchState, JobGraph, JobGraphActor, JobGraphEnd, JobGraphMsg, JobGraphState,
    JobOutcome, JobStart, JournalEntry, NodeDown, NodeDownMonitor, NodeDownMonitorActor, NodeDownMonitorMsg,
    NodeDownMonitorState, NodeReadError, NodeSnapshot, NodeSnapshotSource, PermitNotice, PermitNoticeKind, Permits,
    PermitsActor, PermitsMsg, PermitsState, RateLimiter, RateLimiterActor, RateLimiterMsg, RateLimiterState, ReplyTo,
    Schedule, Scheduler, SchedulerActor, SchedulerMsg, SchedulerState, StoredMessage, Supervisor, SupervisorActor,
    SupervisorMsg, SupervisorState, Topic, TopicActor, TopicMsg, TopicState,
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

fn the_twelve_contracts() -> [Contract; 12] {
    [
        contract_of::<SupervisorActor>().unwrap(),
        contract_of::<DeadLettersActor>().unwrap(),
        contract_of::<TopicActor>().unwrap(),
        contract_of::<SchedulerActor>().unwrap(),
        contract_of::<RateLimiterActor>().unwrap(),
        contract_of::<DebouncerActor>().unwrap(),
        contract_of::<BatcherActor>().unwrap(),
        contract_of::<EventActor>().unwrap(),
        contract_of::<HeartbeatWatchActor>().unwrap(),
        contract_of::<NodeDownMonitorActor>().unwrap(),
        contract_of::<PermitsActor>().unwrap(),
        contract_of::<JobGraphActor>().unwrap(),
    ]
}

#[test]
fn the_contracts_of_the_twelve_types_match_their_references() {
    let contracts = the_twelve_contracts();
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
            "cicada.event",
            "cicada.heartbeat-watch",
            "cicada.node-down",
            "cicada.permits",
            "cicada.job-graph"
        ]
    );
    let update = std::env::var(UPDATE_CONTRACTS_ENV).is_ok_and(|v| v == "true");
    if let Err(e) = check_references(&contracts_dir(), &contracts, update) {
        panic!("{e}");
    }
}

/// The contract of the dead letters changed in 0.17.0 (ADR 0043): a letter holds `payload` or `payloadJson`, so its hash
/// is not the one of 0.16, and a node of 0.16 is refused against a registration of 0.17 and the other way round.
#[test]
fn the_contract_of_the_dead_letters_has_a_new_hash_since_0_17_0() {
    const HASH_OF_0_16: &str = "sha256:815160096f14f71d655450107b35a78fcc22e0f4fc774b77e5bfd7b040e2fb50";
    assert_ne!(contract_of::<DeadLettersActor>().unwrap().hash(), HASH_OF_0_16);
}

/// The event writes `FlowState::new` as raw JSON into the runs it creates, so the new defaults of the flow wire of 0.18.0
/// (ADR 0044) reach its runs, but its own contract holds no flow type: its hash is the one of 0.17.
#[test]
fn the_contract_of_the_event_is_unchanged_by_the_flow_wire_of_0_18_0() {
    const HASH_OF_0_17: &str = "sha256:5508ca9407e13322672e7e30aafe16eeb4fc04174c55c098a1a02b1d8486f48b";
    assert_eq!(contract_of::<EventActor>().unwrap().hash(), HASH_OF_0_17);
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

    let heartbeat = [
        (HeartbeatWatchMsg::Beat, json!({"type": "Beat"})),
        (HeartbeatWatchMsg::Rearm, json!({"type": "Rearm"})),
        (HeartbeatWatchMsg::Expire, json!({"type": "Expire"})),
    ];
    for (message, written) in heartbeat {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<HeartbeatWatchMsg>(&written);
    }
    assert!(cicada::message::decode::<HeartbeatWatchMsg>("Start", &json!({})).is_err(), "no Start message");

    for (message, written) in
        [(NodeDownMonitorMsg::Rearm, json!({"type": "Rearm"})), (NodeDownMonitorMsg::Poll, json!({"type": "Poll"}))]
    {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<NodeDownMonitorMsg>(&written);
    }
    assert!(cicada::message::decode::<NodeDownMonitorMsg>("Start", &json!({})).is_err(), "no Start message");
}

/// A family of a holder of a permit: it declares `Permit` with the notice of the crate.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum BuildMsg {
    /// A notice of a permit pool.
    Permit(PermitNotice),
}

/// `build/{id}`: keeps nothing; it takes the notices of a pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Build {
    id: String,
}

impl ActorRef for Build {
    const TYPE: &'static str = "build";
    type Message = BuildMsg;
}

struct BuildActor;

impl Actor for BuildActor {
    type Ref = Build;
    type State = Value;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: BuildMsg) -> HandlerResult {
        Ok(())
    }
}

#[test]
fn the_permit_messages_and_notices_are_written_in_camel_case_and_pass_their_samples() {
    let build = Build { id: "b1".into() };
    let permits = [
        (
            PermitsMsg::acquire("b1-1", &build, "Permit").unwrap(),
            json!({"type": "Acquire", "request": "b1-1", "to": "build/b1", "messageType": "Permit"}),
        ),
        (
            PermitsMsg::Release { request: "b1-1".into(), grant: 4 },
            json!({"type": "Release", "request": "b1-1", "grant": 4}),
        ),
        (PermitsMsg::Cancel { request: "b1-1".into() }, json!({"type": "Cancel", "request": "b1-1"})),
        (PermitsMsg::Resize { capacity: 3 }, json!({"type": "Resize", "capacity": 3})),
        (PermitsMsg::Audit, json!({"type": "Audit"})),
        (
            PermitsMsg::Notify { request: "b1-1".into(), check: true },
            json!({"type": "Notify", "request": "b1-1", "check": true}),
        ),
    ];
    for (message, written) in permits {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<PermitsMsg>(&written);
    }
    for message in [PermitsMsg::Failed(failed()), PermitsMsg::Stopped(stopped())] {
        assert_message_sample::<PermitsMsg>(&json_of(message));
    }
    for (message_type, invalid) in [
        ("Acquire", json!({"request": "b1-1", "to": "build/b1"})),
        ("Acquire", json!({"request": "b1-1", "messageType": "Permit"})),
        ("Release", json!({"request": "b1-1"})),
        ("Release", json!({"request": "b1-1", "grant": -1})),
        ("Cancel", json!({})),
        ("Resize", json!({})),
        ("Notify", json!({"request": "b1-1"})),
    ] {
        assert!(cicada::message::decode::<PermitsMsg>(message_type, &invalid).is_err(), "{message_type} {invalid}");
    }

    let kinds = [
        (PermitNoticeKind::Granted, Some(4), "granted"),
        (PermitNoticeKind::Queued, None, "queued"),
        (PermitNoticeKind::Refused, None, "refused"),
        (PermitNoticeKind::Check, Some(4), "check"),
        (PermitNoticeKind::Check, None, "check"),
    ];
    for (kind, grant, name) in kinds {
        let notice = PermitNotice { pool: "builds".into(), request: "b1-1".into(), kind, grant };
        let written = json!({"type": "Permit", "pool": "builds", "request": "b1-1", "kind": name, "grant": grant});
        assert_eq!(json_of(BuildMsg::Permit(notice.clone())), written);
        assert_message_sample::<BuildMsg>(&written);
        let BuildMsg::Permit(read) = serde_json::from_value::<BuildMsg>(written).unwrap();
        assert_eq!(read, notice);
    }
    assert_message_sample::<BuildMsg>(
        &json!({"type": "Permit", "pool": "builds", "request": "b1-1", "kind": "queued"}),
    );
    let unknown = json!({"type": "Permit", "pool": "builds", "request": "b1-1", "kind": "revoked"});
    assert!(serde_json::from_value::<BuildMsg>(unknown).is_err(), "four kinds");
}

#[test]
fn a_permits_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_states() {
    let written = json_of(PermitsState::new(4, 100).unwrap());
    let settings = json!({"capacity": 4, "maxWaiting": 100});
    let with = |extra: Value| {
        let mut json = settings.clone();
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    };
    assert_eq!(written, with(json!({"nextGrant": 1, "holders": [], "waiting": []})));
    assert_state_sample::<PermitsState>(&written);
    assert_state_sample::<PermitsState>(&settings);
    let holder = |request: &str, grant: u64, notified: bool| json!({"request": request, "to": "build/b1", "messageType": "Permit", "grant": grant, "notified": notified});
    let waiter = |request: &str| json!({"request": request, "to": "build/b2", "messageType": "Permit"});
    let books = with(json!({"nextGrant": 6, "holders": [holder("a", 3, true), holder("b", 5, false)],
        "waiting": [waiter("c"), waiter("d")]}));
    assert_state_sample::<PermitsState>(&books);
    assert_eq!(json_of(serde_json::from_value::<PermitsState>(books.clone()).unwrap()), books);
    for invalid in [
        with(json!({"capacity": 1001})),
        with(json!({"capacity": -1})),
        with(json!({"maxWaiting": 1001})),
        with(json!({"nextGrant": 0})),
        with(json!({"nextGrant": 6, "holders": [holder("a", 3, true), holder("a", 5, true)]})),
        with(json!({"nextGrant": 6, "holders": [holder("a", 3, true)], "waiting": [waiter("a")]})),
        with(json!({"nextGrant": 3, "holders": [holder("a", 3, true)]})),
        with(
            json!({"nextGrant": 3, "holders": [{"request": "a", "to": "build/b1", "messageType": "Permit", "grant": 1}]}),
        ),
        with(json!({"waiting": [waiter("")]})),
        with(json!({"waiting": [waiter(&"r".repeat(201))]})),
        with(json!({"waiting": [{"request": "c", "to": "not an address", "messageType": "Permit"}]})),
        with(json!({"waiting": [{"request": "c", "to": "build/b2", "messageType": ""}]})),
        json!({"capacity": 1, "maxWaiting": 1, "waiting": [waiter("c"), waiter("d")]}),
        json!({"capacity": 4}),
    ] {
        assert!(serde_json::from_value::<PermitsState>(invalid.clone()).is_err(), "{invalid}");
    }
}

/// The state a pool writes with holders, one told and one not yet, and waiting requests.
#[tokio::test(start_paused = true)]
async fn the_state_a_pool_writes_passes_its_sample() {
    let test = TestActors::builder().with(|node| node.host(PermitsActor).host(BuildActor)).build().await.unwrap();
    let pool = Permits::named("builds");
    test.create(&pool, &PermitsState::new(1, 10).unwrap()).await;
    for id in ["b1", "b2", "b3", "b4"] {
        let build = Build { id: id.into() };
        test.create(&build, &json!({})).await;
        test.send(&pool, PermitsMsg::acquire(id, &build, "Permit").unwrap()).await;
    }
    test.run_until_idle().await;
    test.send(&pool, PermitsMsg::Resize { capacity: 2 }).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    let written = test.state_json(&pool).await.unwrap();
    assert_eq!(written["holders"][0]["notified"], true, "{written}");
    assert_eq!(written["holders"][1]["notified"], false, "{written}");
    assert_eq!(written["waiting"].as_array().unwrap().len(), 2, "{written}");
    assert_eq!(written["nextGrant"], 3, "{written}");
    assert_state_sample::<PermitsState>(&written);
    test.run_until_idle().await;
    let written = test.state_json(&pool).await.unwrap();
    assert_eq!(written["holders"][1]["notified"], true, "{written}");
    assert_state_sample::<PermitsState>(&written);
}

/// A family of a worker and report target of a job graph: it declares `Build` with the job message of the crate and
/// `GraphEnded` with its end report.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CiMsg {
    /// A job message.
    Build(JobStart),
    /// The end report of a run.
    GraphEnded(JobGraphEnd),
}

/// `ci/{id}`: keeps nothing; it takes the job messages and the end report of a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ci {
    id: String,
}

impl ActorRef for Ci {
    const TYPE: &'static str = "ci";
    type Message = CiMsg;
}

struct CiActor;

impl Actor for CiActor {
    type Ref = Ci;
    type State = Value;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: CiMsg) -> HandlerResult {
        Ok(())
    }
}

#[test]
fn the_job_graph_messages_job_message_and_end_report_are_written_in_camel_case_and_pass_their_samples() {
    let start = JobStart { run: "run-7".into(), job: "build".into(), attempt: 2, input: json!({"ref": "main"}) };
    let messages = [
        (JobGraphMsg::Start, json!({"type": "Start"})),
        (JobGraphMsg::Report, json!({"type": "Report"})),
        (
            JobGraphMsg::succeeded(&start, json!({"artifact": "app.tar"})).unwrap(),
            json!({"type": "Result", "job": "build", "attempt": 2, "outcome": "succeeded",
                "output": {"artifact": "app.tar"}, "reason": null}),
        ),
        (
            JobGraphMsg::failed(&start, "it broke"),
            json!({"type": "Result", "job": "build", "attempt": 2, "outcome": "failed", "output": null,
                "reason": "it broke"}),
        ),
        (
            JobGraphMsg::Dispatch { job: "build".into(), attempt: 2 },
            json!({"type": "Dispatch", "job": "build", "attempt": 2}),
        ),
    ];
    for (message, written) in messages {
        assert_eq!(json_of(&message), written);
        assert_message_sample::<JobGraphMsg>(&written);
    }
    assert_message_sample::<JobGraphMsg>(&json!({"type": "Result", "job": "build", "attempt": 2,
        "outcome": "succeeded"}));
    for message in [JobGraphMsg::Failed(failed()), JobGraphMsg::Stopped(stopped())] {
        assert_message_sample::<JobGraphMsg>(&json_of(message));
    }
    for (message_type, invalid) in [
        ("Result", json!({"job": "build", "attempt": 2})),
        ("Result", json!({"job": "build", "attempt": 2, "outcome": "cancelled"})),
        ("Result", json!({"job": "build", "attempt": -1, "outcome": "failed"})),
        ("Result", json!({"attempt": 2, "outcome": "failed"})),
        ("Dispatch", json!({"job": "build"})),
    ] {
        assert!(cicada::message::decode::<JobGraphMsg>(message_type, &invalid).is_err(), "{message_type} {invalid}");
    }

    let written = json!({"type": "Build", "run": "run-7", "job": "build", "attempt": 2, "input": {"ref": "main"}});
    assert_eq!(json_of(CiMsg::Build(start.clone())), written);
    assert_message_sample::<CiMsg>(&written);
    let CiMsg::Build(read) = serde_json::from_value::<CiMsg>(written).unwrap() else { panic!("a job message") };
    assert_eq!(read, start);

    let end = json!({"type": "GraphEnded", "run": "run-7", "status": "failed", "jobs": [
        {"name": "build", "status": "succeeded", "attempt": 1, "output": {"artifact": "app.tar"}, "reason": null},
        {"name": "test", "status": "failed", "attempt": 1, "output": null, "reason": "3 tests failed"},
        {"name": "deploy", "status": "waiting", "attempt": null, "output": null, "reason": null}]});
    assert_message_sample::<CiMsg>(&end);
    let CiMsg::GraphEnded(read) = serde_json::from_value::<CiMsg>(end.clone()).unwrap() else { panic!("a report") };
    assert_eq!(json_of(CiMsg::GraphEnded(read)), end);
    for status in ["running", "succeeded", "failed", "cancelled"] {
        assert_message_sample::<CiMsg>(&json!({"type": "GraphEnded", "run": "r", "status": status, "jobs": []}));
    }
    for status in ["waiting", "running", "succeeded", "failed", "skipped", "cancelled"] {
        let job = json!({"name": "a", "status": status, "attempt": null, "output": null, "reason": null});
        assert_message_sample::<CiMsg>(&json!({"type": "GraphEnded", "run": "r", "status": "failed", "jobs": [job]}));
    }
}

#[test]
fn a_job_graph_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_states() {
    let ci = Ci { id: "run-7".into() };
    let jobs = vec![
        GraphJob::new("build", &ci, "Build", json!({})).unwrap(),
        GraphJob::new("test", &ci, "Build", json!({"suite": "all"})).unwrap().with_needs(["build"]),
    ];
    let written = json_of(JobGraphState::new(jobs, 2, &ci, "GraphEnded").unwrap());
    let job = |name: &str, needs: Value, input: Value| json!({"name": name, "needs": needs, "to": "ci/run-7", "messageType": "Build", "input": input});
    let created = json!({"maxInFlight": 2, "reportTo": "ci/run-7", "reportType": "GraphEnded",
        "jobs": [job("build", json!([]), json!({})), job("test", json!(["build"]), json!({"suite": "all"}))]});
    let mut full = created.clone();
    full["status"] = json!("running");
    full["endedAt"] = Value::Null;
    for entry in full["jobs"].as_array_mut().unwrap() {
        entry["status"] = json!("waiting");
        entry["output"] = Value::Null;
        entry["attempts"] = json!([]);
    }
    assert_eq!(written, full);
    assert_state_sample::<JobGraphState>(&written);
    assert_state_sample::<JobGraphState>(&created);
    let mid_run = json!({"maxInFlight": 2, "reportTo": "ci/run-7", "reportType": "GraphEnded", "status": "running",
        "endedAt": null, "jobs": [
            {"name": "build", "needs": [], "to": "ci/run-7", "messageType": "Build", "input": {},
             "status": "succeeded", "output": {"artifact": "app.tar"}, "attempts": [{"attempt": 1,
             "dispatchedAt": "2026-01-01T00:00:00Z", "endedAt": "2026-01-01T00:00:05Z", "outcome": "succeeded",
             "reason": null}]},
            {"name": "test", "needs": ["build"], "to": "ci/run-7", "messageType": "Build", "input": {},
             "status": "running", "output": null, "attempts": [{"attempt": 1, "dispatchedAt": null,
             "endedAt": null, "outcome": null, "reason": null}]}]});
    assert_state_sample::<JobGraphState>(&mid_run);
    assert_eq!(json_of(serde_json::from_value::<JobGraphState>(mid_run.clone()).unwrap()), mid_run);
    let with = |edit: &dyn Fn(&mut Value)| {
        let mut json = mid_run.clone();
        edit(&mut json);
        json
    };
    for invalid in [
        with(&|j| j["maxInFlight"] = json!(0)),
        with(&|j| j["maxInFlight"] = json!(101)),
        with(&|j| j["reportType"] = json!("")),
        with(&|j| j["reportTo"] = json!("not an address")),
        with(&|j| j["status"] = json!("done")),
        with(&|j| j["jobs"] = json!([])),
        with(&|j| j["jobs"][1]["needs"] = json!(["test"])),
        with(&|j| j["jobs"][1]["needs"] = json!(["deploy"])),
        with(&|j| j["jobs"][0]["needs"] = json!(["test"])),
        with(&|j| j["jobs"][1]["name"] = json!("build")),
        with(&|j| j["jobs"][1]["status"] = json!("waiting")),
        with(&|j| j["jobs"][0]["attempts"][0]["outcome"] = json!("cancelled")),
        with(&|j| j["jobs"][1]["attempts"][0]["attempt"] = json!(0)),
        with(&|j| j.as_object_mut().unwrap().remove("jobs").map(drop).unwrap()),
    ] {
        assert!(serde_json::from_value::<JobGraphState>(invalid.clone()).is_err(), "{invalid}");
    }
}

/// The states a run writes: mid-run with a job dispatched and one not yet, and ended.
#[tokio::test(start_paused = true)]
async fn the_states_a_run_writes_pass_their_sample() {
    let test = TestActors::builder().with(|node| node.host(JobGraphActor).host(CiActor)).build().await.unwrap();
    let ci = Ci { id: "run-7".into() };
    test.create(&ci, &json!({})).await;
    let jobs = vec![
        GraphJob::new("build", &ci, "Build", json!({})).unwrap(),
        GraphJob::new("lint", &ci, "Build", json!({})).unwrap(),
        GraphJob::new("test", &ci, "Build", json!({})).unwrap().with_needs(["build"]),
    ];
    let run = JobGraph::named("run-7");
    test.create(&run, &JobGraphState::new(jobs, 2, &ci, "GraphEnded").unwrap()).await;
    test.send(&run, JobGraphMsg::Start).await;
    for _ in 0..2 {
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the start, then the first dispatch");
    }
    let written = test.state_json(&run).await.unwrap();
    assert!(written["jobs"][0]["attempts"][0]["dispatchedAt"].is_string(), "{written}");
    assert!(written["jobs"][1]["attempts"][0]["dispatchedAt"].is_null(), "{written}");
    assert_state_sample::<JobGraphState>(&written);
    test.run_until_idle().await;
    for job in ["build", "lint", "test"] {
        let attempt = 1;
        let result = JobGraphMsg::Result {
            job: job.into(),
            attempt,
            outcome: JobOutcome::Succeeded,
            output: Some(json!({"job": job})),
            reason: None,
        };
        test.send(&run, result).await;
        test.run_until_idle().await;
    }
    let written = test.state_json(&run).await.unwrap();
    assert_eq!(written["status"], "succeeded", "{written}");
    assert_state_sample::<JobGraphState>(&written);
}

/// A family of a subscriber of the node-down topic: it declares `NodeDown` with the payload of the crate.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum OnCallMsg {
    /// A node row was seen dead.
    NodeDown(NodeDown),
}

#[test]
fn a_node_down_is_written_in_camel_case_without_absent_details_and_passes_a_receivers_samples() {
    let all = json!({
        "type": "NodeDown", "node": "web-1", "startedAt": "2026-11-16T08:00:00Z", "lastSeen": "2026-11-16T09:00:00.500Z",
        "detectedAt": "2026-11-16T09:02:00.500Z", "hostname": "web-1.internal", "cicadaVersion": "0.20.2", "label": "shop"
    });
    let OnCallMsg::NodeDown(down) = serde_json::from_value::<OnCallMsg>(all.clone()).unwrap();
    assert_eq!(json_of(OnCallMsg::NodeDown(down.clone())), all);
    assert_message_sample::<OnCallMsg>(&all);
    let mut bare = down;
    (bare.hostname, bare.cicada_version, bare.label) = (None, None, None);
    let written = json_of(OnCallMsg::NodeDown(bare));
    assert_eq!(
        written.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["type", "node", "startedAt", "lastSeen", "detectedAt"]
    );
    assert_message_sample::<OnCallMsg>(&written);
    let mut blank = all.clone();
    blank["node"] = json!("");
    assert!(serde_json::from_value::<OnCallMsg>(blank).is_err(), "a node id is never blank");
    let mut late = all;
    late["detectedAt"] = json!("soon");
    assert!(serde_json::from_value::<OnCallMsg>(late).is_err());
}

#[test]
fn a_node_down_monitor_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_states() {
    let topic = json!({"topic": {"name": "nodes"}});
    let with = |extra: Value| {
        let mut json = topic.clone();
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    };
    let written = json_of(NodeDownMonitorState::new(Topic::named("nodes")).unwrap());
    let empty = json!({"alerted": {}, "armed": null, "nextPollAt": null, "lastCheckedAt": null, "lastProblem": null});
    assert_eq!(written, with(empty));
    assert_state_sample::<NodeDownMonitorState>(&written);
    assert_state_sample::<NodeDownMonitorState>(&topic);
    let at = "2026-11-16T09:00:00Z";
    let mark = json!({"startedAt": at, "lastSeen": at});
    let full = with(json!({"alerted": {"web-1": mark}, "armed": "p1", "nextPollAt": at, "lastCheckedAt": at,
                            "lastProblem": "timeout"}));
    assert_state_sample::<NodeDownMonitorState>(&full);
    for invalid in [
        json!({}),
        json!({"topic": "cicada.topic/nodes"}),
        with(json!({"armed": "p1"})),
        with(json!({"nextPollAt": at})),
        with(json!({"alerted": {"web-1": {"startedAt": "2026-11-16T10:00:00Z", "lastSeen": at}}})),
        with(json!({"alerted": {"web-1": {"startedAt": at}}})),
        with(json!({"lastProblem": "down"})),
        with(json!({"lastCheckedAt": "soon"})),
    ] {
        assert!(serde_json::from_value::<NodeDownMonitorState>(invalid.clone()).is_err(), "{invalid}");
    }
}

#[test]
fn a_heartbeat_watch_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_states() {
    let mut state = HeartbeatWatchState::new(
        std::time::Duration::from_secs(90_000),
        &worker("a"),
        WorkerMsg::Note { text: "n".into() },
    )
    .unwrap();
    let settings =
        json!({"silence": "PT25H", "target": "worker/a", "alarm": {"type": "Note", "payload": {"text": "n"}}});
    let with = |extra: Value| {
        let mut json = settings.clone();
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    };
    let written = json_of(&state);
    assert_eq!(written, with(json!({"retry": null, "status": "waiting", "lastBeatAt": null, "armed": null})));
    assert_state_sample::<HeartbeatWatchState>(&written);
    assert_state_sample::<HeartbeatWatchState>(&settings);
    state.retry = Some(RetryPolicy::new(2, Backoff::fixed(std::time::Duration::from_secs(10)).unwrap()).unwrap());
    state.status = HeartbeatStatus::Alive;
    state.last_beat_at = Some("2026-11-13T02:00:00Z".parse().unwrap());
    state.armed = Some(MessageId::new("e1").unwrap());
    let fixed = json!({"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT10S"}});
    let at = "2026-11-13T02:00:00Z";
    assert_eq!(json_of(&state), with(json!({"retry": fixed, "status": "alive", "lastBeatAt": at, "armed": "e1"})));
    assert_state_sample::<HeartbeatWatchState>(&json_of(&state));
    assert_state_sample::<HeartbeatWatchState>(&with(json!({"status": "silent", "lastBeatAt": at})));
    for invalid in [
        with(json!({"silence": "PT0S"})),
        with(json!({"silence": "P365001D"})),
        with(json!({"silence": 90000})),
        with(json!({"target": "not an address"})),
        with(json!({"alarm": {"type": "Note"}})),
        with(json!({"status": "asleep"})),
        with(json!({"lastBeatAt": at})),
        with(json!({"armed": "e1"})),
        with(json!({"status": "alive", "lastBeatAt": at})),
        with(json!({"status": "silent", "lastBeatAt": at, "armed": "e1"})),
        with(json!({"status": "silent"})),
        with(json!({"status": "silent", "lastBeatAt": "soon"})),
        with(json!({"status": "silent", "lastBeatAt": "9999-12-31T23:00:00Z"})),
        with(json!({"retry": {"maxAttempts": 0, "backoff": fixed["backoff"]}})),
        json!({"silence": "PT25H", "target": "worker/a"}),
    ] {
        assert!(serde_json::from_value::<HeartbeatWatchState>(invalid.clone()).is_err(), "{invalid}");
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
                .host(HeartbeatWatchActor)
                .host(NodeDownMonitorActor::new(FixedSample::default()))
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

    let watch = HeartbeatWatch::named("import");
    let mut state = HeartbeatWatchState::new(second, &worker("a"), WorkerMsg::Note { text: "silent".into() }).unwrap();
    state.retry = Some(RetryPolicy::new(2, Backoff::fixed(second).unwrap()).unwrap());
    test.create(&watch, &state).await;
    test.send(&watch, HeartbeatWatchMsg::Beat).await;
    test.run_until_idle().await;
    let written = test.state_json(&watch).await.unwrap();
    assert_eq!(written["status"], "alive", "{written}");
    for key in ["lastBeatAt", "armed"] {
        assert!(written[key].is_string(), "{key} in {written}");
    }
    assert_state_sample::<HeartbeatWatchState>(&written);
    test.advance_time_by(second).await;
    let written = test.state_json(&watch).await.unwrap();
    assert_eq!((written["status"].clone(), written["armed"].clone()), (json!("silent"), Value::Null), "{written}");
    assert!(written["lastBeatAt"].is_string(), "{written}");
    assert_state_sample::<HeartbeatWatchState>(&written);

    // A monitor that alerts a dead row, then reads nothing: the second check keeps `alerted` and adds `lastProblem`.
    let nodes = Topic::named("nodes");
    test.create(&nodes, &TopicState::default()).await;
    test.create(&NodeDownMonitor, &NodeDownMonitorState::new(nodes).unwrap()).await;
    test.send(&NodeDownMonitor, NodeDownMonitorMsg::Rearm).await;
    test.run_until_idle().await;
    let written = test.state_json(&NodeDownMonitor).await.unwrap();
    assert_eq!(written["alerted"]["web-1"]["lastSeen"], "2025-12-31T23:50:00Z", "{written}");
    for key in ["armed", "nextPollAt", "lastCheckedAt"] {
        assert!(written[key].is_string(), "{key} in {written}");
    }
    assert_state_sample::<NodeDownMonitorState>(&written);
    test.advance_time_by(10 * second).await;
    let written = test.state_json(&NodeDownMonitor).await.unwrap();
    assert_eq!(written["lastProblem"], "unavailable", "{written}");
    assert!(written["alerted"]["web-1"].is_object(), "{written}");
    assert_state_sample::<NodeDownMonitorState>(&written);
}

/// A source that gives one sample with a dead row `web-1`, then fails.
#[derive(Default)]
struct FixedSample(std::sync::Mutex<usize>);

impl NodeSnapshotSource for FixedSample {
    async fn snapshot(&self) -> Result<NodeSnapshot, NodeReadError> {
        let mut reads = self.0.lock().unwrap();
        *reads += 1;
        if *reads > 1 {
            return Err(NodeReadError("down".into()));
        }
        let at = |text: &str| text.parse().unwrap();
        let row = cicada::runtime::NodeInfo {
            id: cicada::store::NodeId::new("web-1").unwrap(),
            hosted: Vec::new(),
            started_at: at("2025-12-31T23:00:00Z"),
            last_seen: at("2025-12-31T23:50:00Z"),
            hostname: None,
            cicada_version: None,
            label: Some("shop".into()),
            status: cicada::runtime::NodeHealth::Dead,
            running_turns: Vec::new(),
        };
        Ok(NodeSnapshot { as_of: at("2026-01-01T00:00:00Z"), nodes: vec![row] })
    }
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

/// The hash of the test flow's contract since 0.18.0 (ADR 0044): the releases that enabled replies, deadlines and joins
/// changed descriptions only.
const FLOW_HASH_OF_0_18_0: &str = "sha256:6ab4427f7456de8a94083af3f452df1aecbb70ac68c6e3702ad758bb280d5d13";

/// A flow's contract is the host's: pinned with the actor tests, not in `docs/reference/contracts`. It holds the sleep
/// entry of the journal, so a flow that sleeps, as this one does since sleeps were added, has the fixture of before.
/// Scenario: Rollout and contract equality: its hash is that of 0.18.0, with no new wire field.
#[test]
fn the_contract_of_a_test_flow_matches_its_fixture() {
    let contract = contract_of::<FlowActor<SignUpFlow>>().unwrap();
    assert_eq!(contract.type_name().as_str(), "test.sign-up");
    assert_eq!(contract.hash(), FLOW_HASH_OF_0_18_0);
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
    let update = std::env::var(UPDATE_CONTRACTS_ENV).is_ok_and(|v| v == "true");
    if let Err(e) = check_references(&fixtures, std::slice::from_ref(&contract), update) {
        panic!("{e}");
    }
    let json = contract.to_json();
    let state = json["state"].to_string();
    for word in
        ["sleep", "until", "step", "result", "running", "done", "armed", "send", "wait", "cancelling", "cancelled"]
    {
        assert!(state.contains(&format!("\"{word}\"")), "{word} in {state}");
    }
    // Scenario: Required nullable values; and Schema bounds, in the schema a host registers.
    let required = |schema: &Value| schema["required"].as_array().unwrap().clone();
    assert_eq!(required(&json["messages"]["Reply"]), [json!("request"), json!("value")]);
    assert!(json["messages"]["Cancel"]["properties"].as_object().unwrap().is_empty());
    let defs = &json["state"]["$defs"];
    assert_eq!(required(&defs["FlowReply"]), [json!("request"), json!("value")]);
    let fields = &json["state"]["properties"];
    for (field, most) in [("journal", 1000), ("cleanup", 1000), ("replies", 2000)] {
        assert_eq!(fields[field]["maxItems"], most, "{field}");
    }
    let entries = defs["JournalEntry"]["oneOf"].as_array().unwrap();
    let kind = |name: &str| entries.iter().find(|e| e["properties"]["kind"]["const"] == name).unwrap().clone();
    let (send, wait) = (kind("send"), kind("wait"));
    assert_eq!(required(&send), [json!("kind"), json!("id"), json!("to"), json!("message")]);
    assert_eq!(send["properties"]["reply"]["default"], false);
    assert_eq!(required(&wait), [json!("kind"), json!("id"), json!("requests"), json!("mode")]);
    let requests = &wait["properties"]["requests"];
    assert_eq!((requests["minItems"].clone(), requests["maxItems"].clone()), (json!(1), json!(100)));
    assert_eq!(wait["properties"]["received"]["maxItems"], 100);
    for entry in [&send, &wait] {
        assert_eq!(entry["properties"]["id"]["pattern"], "^[A-Za-z0-9._~:-]+$");
        assert_eq!(entry["properties"]["id"]["maxLength"], 100);
    }
    let names = |def: &str| -> Vec<Value> {
        defs[def]["oneOf"].as_array().unwrap().iter().map(|v| v["const"].clone()).collect()
    };
    assert_eq!(names("FlowStatus"), [json!("running"), json!("cancelling"), json!("done"), json!("cancelled")]);
    assert_eq!(names("CreateKind"), [json!("independent"), json!("child")]);
    assert_eq!(names("WaitMode"), [json!("all"), json!("first")]);
    assert_eq!(names("WaitOutcome"), [json!("received"), json!("timedOut")]);
    assert!(!contracts_dir().join("test.sign-up.json").exists(), "no fixed type name in the catalog");
}

#[test]
fn a_flow_state_is_written_in_camel_case_passes_its_samples_and_refuses_invalid_states() {
    type State = FlowState<SignUp>;
    let input = json!({"user": "u1"});
    let initial = json_of(FlowState::new(SignUp { user: "u1".into() }));
    let empty =
        json!({"input": input, "journal": [], "status": "running", "armed": null, "replies": [], "cleanup": []});
    assert_eq!(initial, empty);
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
    let written = json!({"input": input, "journal": slept["journal"], "status": "running", "armed": null,
        "replies": [], "cleanup": []});
    assert_eq!(json_of(&decoded), written, "the sleep round-trips");
    let welcome = json!({"kind": "step", "id": "welcome", "result": 23});
    let done = json!({"input": input, "journal": [account, welcome], "status": "done", "armed": null});
    assert_state_sample::<State>(&done);
    assert_eq!(serde_json::from_value::<State>(done).unwrap().status, FlowStatus::Done);
    let sent = json!({"kind": "send", "id": "start", "to": "jobs/j1", "message": "m1", "create": "child"});
    let asked = json!({"kind": "send", "id": "ask", "to": "jobs/j1", "message": "m2", "create": null, "reply": true});
    let waited = json!({"kind": "wait", "id": "answer", "requests": ["m2"], "mode": "all", "until": null,
        "outcome": "received", "received": ["m2"]});
    let replies = json!([{"request": "m2", "value": null}]);
    assert_state_sample::<State>(&json!({"input": input, "journal": [sent, asked, waited], "replies": replies}));
    let open = json!({"kind": "wait", "id": "answer", "requests": ["m2"], "mode": "first"});
    let interrupted = json!({"input": input, "journal": [asked, open], "status": "cancelling", "armed": "m3",
        "cleanup": [{"kind": "wait", "id": "answer", "requests": ["m2"], "mode": "all", "until": "2026-11-14T00:00:00Z",
            "outcome": "timedOut"}]});
    assert_state_sample::<State>(&interrupted);

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
        json!({"input": input, "journal": [asked], "replies": [{"request": "m2"}]}),
        entry(json!({"kind": "wait", "id": "w", "requests": ["m2"]})),
        entry(json!({"kind": "send", "id": "s", "to": "jobs/j1"})),
        json!({"input": input, "cleanup": [notify]}),
    ] {
        assert!(serde_json::from_value::<State>(invalid.clone()).is_err(), "{invalid}");
    }
}

#[test]
fn a_flow_message_and_saved_results_read_back() {
    assert_eq!(json_of(FlowMsg::Continue), json!({"type": "Continue"}));
    assert_message_sample::<FlowMsg>(&json!({"type": "Continue"}));
    assert_message_sample::<FlowMsg>(&json!({"type": "Reply", "request": "m1", "value": null}));
    assert_message_sample::<FlowMsg>(&json!({"type": "Reply", "request": "m1", "value": {"ok": [1]}}));
    assert_message_sample::<FlowMsg>(&json!({"type": "Cancel"}));
    assert!(cicada::message::decode::<FlowMsg>("Reply", &json!({"request": "m1"})).is_err(), "a missing value");
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

/// The result a job replies with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct JobResult {
    /// What the job made.
    artifact: String,
}

/// `test.job/<id>`: a target of flow requests, whose message carries the route of its reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct Job(String);

impl ActorRef for Job {
    const TYPE: &'static str = "test.job";
    type Message = JobMsg;
}

/// The messages of a job.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum JobMsg {
    /// Starts the job; its result goes to `reply`.
    Start {
        /// The size of the job.
        size: u32,
        /// Where the result goes.
        reply: ReplyTo<JobResult>,
    },
}

struct JobActor;

impl Actor for JobActor {
    type Ref = Job;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: JobMsg) -> HandlerResult {
        let JobMsg::Start { size, reply } = message;
        reply.send(turn, &JobResult { artifact: format!("a{size}") }).await?;
        Ok(())
    }
}

/// Scenario: Stable schema. A target that carries a `ReplyTo` exposes the route in its contract, pinned in
/// `tests/contracts`: two required fields, the address and the message id, the same for every reply type. Adding the
/// route to a target changes its contract like any new field.
#[test]
fn a_reply_route_is_pinned_in_the_contract_of_its_target() {
    let contract = contract_of::<JobActor>().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
    let update = std::env::var(UPDATE_CONTRACTS_ENV).is_ok_and(|v| v == "true");
    if let Err(e) = check_references(&fixtures, std::slice::from_ref(&contract), update) {
        panic!("{e}");
    }
    let json = contract.to_json();
    let start = &json["messages"]["Start"];
    assert_eq!(start["required"], json!(["size", "reply"]));
    assert_eq!(start["properties"]["reply"]["$ref"], "#/$defs/ReplyTo");
    let route = &start["$defs"]["ReplyTo"];
    assert_eq!(route["type"], "object");
    assert_eq!(route["required"], json!(["run", "request"]));
    let flow = contract_of::<FlowActor<SignUpFlow>>().unwrap().to_json();
    let entries = flow["state"]["$defs"]["JournalEntry"]["oneOf"].as_array().unwrap().clone();
    let send = entries.iter().find(|e| e["properties"]["kind"]["const"] == "send").unwrap();
    let without = |schema: &Value| {
        let mut schema = schema.clone();
        schema.as_object_mut().unwrap().remove("description");
        schema
    };
    let (run, request) = (&route["properties"]["run"], &route["properties"]["request"]);
    assert_eq!(without(run), without(&send["properties"]["to"]), "the run is an actor address");
    assert_eq!(without(request), without(&send["properties"]["message"]), "the request is a message id");

    let schema = |s: schemars::Schema| serde_json::to_value(s).unwrap();
    assert_eq!(schema(schemars::schema_for!(ReplyTo<String>)), schema(schemars::schema_for!(ReplyTo<u64>)));
    let text: ReplyTo<String> = serde_json::from_value(json!({"run": "test.sign-up/u1", "request": "m1"})).unwrap();
    let number: ReplyTo<u64> = serde_json::from_value(json_of(&text)).unwrap();
    assert_eq!(json_of(&text), json_of(&number), "one wire form for every reply type");
    assert_message_sample::<JobMsg>(&json!({"type": "Start", "size": 3, "reply": {"run": "f/k", "request": "m1"}}));
    assert!(cicada::message::decode::<JobMsg>("Start", &json!({"size": 3, "reply": {"run": "f/k"}})).is_err());
}

/// Scenario: Owned encoded value, through a node: the reply a target sends is a `FlowMsg::Reply` the run's family takes.
#[tokio::test(start_paused = true)]
async fn the_reply_a_target_sends_is_a_flow_message() {
    let test =
        TestActors::builder().with(|node| node.host(JobActor).host(FlowActor::new(SignUpFlow))).build().await.unwrap();
    let run = FlowRun::<SignUpFlow>::named("u1");
    test.create(&run, &FlowState::new(SignUp { user: "u1".into() })).await;
    test.create(&Job("j1".into()), &NoState {}).await;
    let reply: ReplyTo<JobResult> = serde_json::from_value(json!({"run": "test.sign-up/u1", "request": "m1"})).unwrap();
    test.send(&Job("j1".into()), JobMsg::Start { size: 3, reply }).await;
    test.system().run_due_turns(1).await.unwrap();
    let sent = test.pending(&run).await;
    assert_eq!(sent[0].message_type, "Reply");
    assert_eq!(sent[0].payload, json!({"request": "m1", "value": {"artifact": "a3"}}));
    assert_message_sample::<FlowMsg>(&json!({"type": "Reply", "request": "m1", "value": sent[0].payload["value"]}));
}
