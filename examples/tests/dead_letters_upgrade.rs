//! Example 16: the upgrade of dead letters to 0.17.0, rehearsed on a disposable in-memory store on virtual time. The
//! store starts as 0.16 left it: the registration of 0.16 (`tests/fixtures/cicada.dead-letters-0.16.json`, the
//! reference of 0.16.17) and dead-letters actors whose letters hold raw payloads.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::runtime::{LeaseTurnsResult, RegisterContractResult, WatchResult};
use cicada::store::NodeId;
use cicada::{Store, TypeName, WatchTarget};
use cicada_actors::{DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, LetterPayload};
use cicada_examples::dead_letters_upgrade::{Export, UpgradeError, check, convert, export, reset, restore};
use cicada_test::{TokioClock, default_start};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// `worker/{id}`: every message fails; `Make` creates a dead-letters actor as its child.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Work {
    id: String,
}

impl ActorRef for Work {
    const TYPE: &'static str = "worker";
    type Message = WorkMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum WorkMsg {
    Fail,
    Make,
}

struct WorkActor;

impl Actor for WorkActor {
    type Ref = Work;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: WorkMsg) -> HandlerResult {
        match message {
            WorkMsg::Fail => Err(HandlerError::msg("boom")),
            WorkMsg::Make => Ok(turn.create_child(&DeadLetters::named("kid"), &DeadLettersState::default())?),
        }
    }
}

fn w() -> Work {
    Work { id: "w".into() }
}

fn at(text: &str) -> ActorAddress {
    text.parse().unwrap()
}

fn id(text: &str) -> MessageId {
    MessageId::new(text).unwrap()
}

/// The contract of `cicada.dead-letters` in 0.16.
fn contract_of_0_16() -> Contract {
    let json = serde_json::from_str(include_str!("fixtures/cicada.dead-letters-0.16.json")).unwrap();
    Contract::from_json(&json).unwrap()
}

/// Numbers that a parser without exact rounding, or a tool that uses one, changes.
fn numbers() -> Value {
    let floats = [1.0715660391465826e-75, -1.0715660391465826e-75, f64::from_bits(1), f64::MAX, -0.0];
    let mut all: Vec<Value> = floats.into_iter().map(Value::from).collect();
    all.extend([Value::from(i64::MIN), Value::from(u64::MAX)]);
    Value::Array(all)
}

/// The bits of every number of `value`, an array of [`numbers`].
fn bits(value: &Value) -> Vec<(Option<u64>, Option<i64>, Option<u64>)> {
    value.as_array().unwrap().iter().map(|n| (n.as_f64().map(f64::to_bits), n.as_i64(), n.as_u64())).collect()
}

/// A letter as 0.16 wrote it, for a `Fail` to `worker/w`: its payload raw.
fn letter_of_0_16(payload: Value, redriven_as: Option<&str>) -> Value {
    let mut letter = json!({
        "actor": "worker/w", "messageType": "Fail", "payload": payload, "correlationId": "c-1", "from": null,
        "error": "boom", "cause": "handler", "attempts": 1, "recordedAt": "2026-01-01T00:00:00Z", "seq": 7,
    });
    if let Some(sent) = redriven_as {
        letter["redrivenAs"] = json!(sent);
    }
    letter
}

/// The letters of `cicada.dead-letters/_` in 0.16: payloads that are null, a marker look-alike, numbers and `{}`
/// already redriven.
fn letters_of_underscore() -> Value {
    json!({"letters": {
        "m-null": letter_of_0_16(Value::Null, None),
        "m-marker": letter_of_0_16(json!({"payloadJson": "[1]"}), None),
        "m-numbers": letter_of_0_16(json!({"n": numbers()}), None),
        "m-done": letter_of_0_16(json!({}), Some("m-sent")),
    }})
}

/// A store as 0.16 left it, on virtual time: the registration of 0.16, `worker/w` and two dead-letters actors, `_`
/// watching every type and `ops`, paused, watching `worker`. Returns the store, a management node that hosts nothing
/// and a node of the workers, which starts against the old registration because it does not host the dead letters.
async fn as_0_16_left_it() -> (Store, ActorSystem, ActorSystem) {
    let store = Store::memory_on(Arc::new(TokioClock::new(default_start())));
    let manage = ActorSystem::builder(store.clone()).build();
    assert!(matches!(
        manage.register_contract(&contract_of_0_16()).await.unwrap(),
        RegisterContractResult::Registered(_)
    ));
    let workers = ActorSystem::builder(store.clone()).host(WorkActor).build();
    workers.initialize().await.unwrap();
    workers.create(&w(), &NoState {}).await.unwrap();
    manage.create_actor(&at("cicada.dead-letters/_"), letters_of_underscore()).await.unwrap();
    let ops = json!({"letters": {"m-ops": letter_of_0_16(json!({"to": "ops"}), None)}});
    manage.create_actor(&at("cicada.dead-letters/ops"), ops).await.unwrap();
    assert_eq!(manage.watch(&WatchTarget::Every, &at("cicada.dead-letters/_")).await.unwrap(), WatchResult::Created);
    let worker_type = WatchTarget::parse("worker").unwrap();
    assert_eq!(manage.watch(&worker_type, &at("cicada.dead-letters/ops")).await.unwrap(), WatchResult::Created);
    manage.pause_mailbox(&at("cicada.dead-letters/ops")).await.unwrap();
    (store, manage, workers)
}

/// A node of 0.17.0 that hosts the dead letters and the workers.
fn node_of_0_17(store: &Store) -> ActorSystem {
    ActorSystem::builder(store.clone()).host(DeadLettersActor::default()).host(WorkActor).build()
}

async fn run(system: &ActorSystem) {
    while system.run_due_turns(10).await.unwrap() > 0 {}
}

async fn letters(system: &ActorSystem, address: &str) -> DeadLettersState {
    serde_json::from_value(system.get_actor(&at(address)).await.unwrap().expect("the actor").state).unwrap()
}

/// Scenarios: Old raw letters survive restore; Old hosts and readers. The rehearsal of the operations guide: a node of
/// 0.17.0 is refused against the registration of 0.16; the export, written to text and read back, is converted, the
/// registration reset and the actors restored with their letters, pause flags and watches; the old contract is refused
/// then, by a registration and by a lease; the restored actors are new instances, record a new failure, and a letter
/// redriven before is not sent again.
#[tokio::test(start_paused = true)]
async fn the_upgrade_keeps_every_letter_pause_and_watch_and_the_new_actors_record_new_failures() {
    let (store, manage, _workers) = as_0_16_left_it().await;
    let before = manage.get_actor(&at("cicada.dead-letters/_")).await.unwrap().unwrap();
    assert!(node_of_0_17(&store).initialize().await.is_err(), "a node of 0.17.0 does not start against 0.16");

    let exported = export(&manage).await.unwrap();
    let file = serde_json::to_string_pretty(&exported).unwrap();
    let mut read: Export = serde_json::from_str(&file).unwrap();
    assert_eq!(read, exported, "the file holds the export exactly");
    convert(&mut read).unwrap();
    assert_eq!(read, exported, "no payload of these letters is too deep for a value");

    tokio::time::advance(Duration::from_secs(60)).await;
    reset(&manage, &read).await.unwrap();
    restore(&manage, &read).await.unwrap();

    let new_hash = contract_of::<DeadLettersActor>().unwrap().hash().to_owned();
    let refused = manage.register_contract(&contract_of_0_16()).await.unwrap();
    assert!(matches!(refused, RegisterContractResult::Conflict { .. }), "{refused:?}");
    let old_hash = contract_of_0_16().hash().to_owned();
    assert_ne!(old_hash, new_hash);
    let types = BTreeMap::from([(TypeName::parse("cicada.dead-letters").unwrap(), old_hash)]);
    let leased = manage.lease_turns(&NodeId::new("old-worker").unwrap(), &types, 1).await.unwrap();
    assert!(matches!(leased, LeaseTurnsResult::ContractMismatch { registered: Some(h), .. } if h == new_hash));

    let after = manage.get_actor(&at("cicada.dead-letters/_")).await.unwrap().unwrap();
    assert_eq!(after.state, before.state, "the letters as they were");
    assert_eq!(after.version, 0, "a new instance");
    assert!(after.created_at > before.created_at, "created again");
    let held = letters(&manage, "cicada.dead-letters/_").await;
    assert_eq!(held.letters[&id("m-null")].payload, LetterPayload::Value(Value::Null));
    assert_eq!(held.letters[&id("m-marker")].payload, LetterPayload::Value(json!({"payloadJson": "[1]"})));
    let restored = held.letters[&id("m-numbers")].payload.value().unwrap();
    assert_eq!(bits(&restored["n"]), bits(&numbers()), "every number with its bits");
    assert_eq!(held.letters[&id("m-done")].redriven_as, Some(id("m-sent")));
    assert_eq!(held.letters[&id("m-done")].seq, 7);
    let ops = manage.get_actor(&at("cicada.dead-letters/ops")).await.unwrap().unwrap();
    assert!(ops.paused, "the pause flag");
    let watches = manage.watches(None, None, None, 10).await.unwrap().items;
    let pairs: Vec<(String, String)> = watches.iter().map(|w| (w.target.to_string(), w.watcher.to_string())).collect();
    let expected = [("*", "cicada.dead-letters/_"), ("worker", "cicada.dead-letters/ops")];
    assert_eq!(pairs, expected.map(|(t, w)| (t.to_owned(), w.to_owned())));

    let node = node_of_0_17(&store);
    node.initialize().await.unwrap();
    let cicada::SendResult::Accepted(failed) = node.send(&w(), WorkMsg::Fail).await.unwrap() else { panic!() };
    run(&node).await;
    assert!(letters(&node, "cicada.dead-letters/_").await.letters.contains_key(&failed.id), "a new failure");
    assert!(!letters(&node, "cicada.dead-letters/ops").await.letters.contains_key(&failed.id), "ops is paused");
    node.resume_mailbox(&at("cicada.dead-letters/ops")).await.unwrap();
    run(&node).await;
    assert!(letters(&node, "cicada.dead-letters/ops").await.letters.contains_key(&failed.id), "and resumed");

    let underscore = DeadLetters::default();
    let pending = node.pending(&at("worker/w"), None, 10).await.unwrap().unwrap().items.len();
    node.pause_mailbox(&at("worker/w")).await.unwrap();
    node.send(&underscore, DeadLettersMsg::Redrive { message_id: id("m-done") }).await.unwrap();
    node.send(&underscore, DeadLettersMsg::Redrive { message_id: id("m-null") }).await.unwrap();
    run(&node).await;
    let sent = node.pending(&at("worker/w"), None, 10).await.unwrap().unwrap().items;
    assert_eq!(sent.len(), pending + 1, "m-done is not sent again");
    assert_eq!((sent[pending].payload.clone(), sent[pending].correlation_id.as_deref()), (Value::Null, Some("c-1")));
}

/// Scenario: Maintenance safeguards. The export refuses an actor that another actor watches, one with a pending
/// message, and one that is the child of another actor; nothing is deleted.
#[tokio::test(start_paused = true)]
async fn the_export_refuses_a_watched_a_busy_and_a_child_dead_letters_actor() {
    let (_, manage, _) = as_0_16_left_it().await;
    let on = WatchTarget::Actor(at("cicada.dead-letters/_"));
    manage.watch(&on, &at("cicada.dead-letters/ops")).await.unwrap();
    let refused = export(&manage).await.unwrap_err();
    assert!(matches!(&refused, UpgradeError::Watched { watcher, .. } if *watcher == at("cicada.dead-letters/ops")));
    manage.unwatch(&on, &at("cicada.dead-letters/ops")).await.unwrap();

    let discard = DeadLettersMsg::Discard { message_id: id("m-ops") };
    let cicada::SendResult::Accepted(m) = manage.send(&DeadLetters::named("ops"), discard).await.unwrap() else {
        panic!()
    };
    assert!(matches!(export(&manage).await.unwrap_err(), UpgradeError::Busy(a) if a == at("cicada.dead-letters/ops")));
    manage.cancel(&m.id).await.unwrap();
    assert!(export(&manage).await.is_ok(), "idle again");

    let (_, manage, workers) = as_0_16_left_it().await;
    workers.send(&w(), WorkMsg::Make).await.unwrap();
    run(&workers).await;
    assert!(matches!(export(&manage).await.unwrap_err(), UpgradeError::Tree(a) if a == at("cicada.dead-letters/kid")));
    assert_eq!(manage.list_actors("cicada.dead-letters/", None, 10).await.unwrap().items.len(), 3, "nothing deleted");
}

/// Scenario: Incomplete export or unquiesced work. The reset refuses an export that misses an actor created after it,
/// and one whose actor is gone, before it deletes anything.
#[tokio::test(start_paused = true)]
async fn the_reset_refuses_an_incomplete_or_stale_export_and_deletes_nothing() {
    let (_, manage, _) = as_0_16_left_it().await;
    let exported = export(&manage).await.unwrap();
    manage.create_actor(&at("cicada.dead-letters/late"), json!({"letters": {}})).await.unwrap();
    assert!(matches!(reset(&manage, &exported).await.unwrap_err(), UpgradeError::Stale(why) if why.contains("late")));
    assert_eq!(manage.list_actors("cicada.dead-letters/", None, 10).await.unwrap().items.len(), 3, "nothing deleted");

    manage.delete_actor(&at("cicada.dead-letters/late")).await.unwrap();
    manage.delete_actor(&at("cicada.dead-letters/ops")).await.unwrap();
    assert!(matches!(reset(&manage, &exported).await.unwrap_err(), UpgradeError::Stale(why) if why.contains("ops")));
    assert!(manage.get_actor(&at("cicada.dead-letters/_")).await.unwrap().is_some(), "nothing deleted");
}

/// Scenario: Readable legacy deep export. A raw payload of 70 levels in an export, too deep for the state of 0.17.0,
/// is converted to `payloadJson` and restored; its redrive is refused for its depth, and it can be discarded. An export
/// that the state type of 0.17.0 cannot read is refused by the conversion.
#[tokio::test(start_paused = true)]
async fn a_deep_legacy_payload_is_converted_to_text_restored_refused_by_a_redrive_and_discarded() {
    let deep = (0..70).fold(json!(1), |inner, _| json!([inner]));
    let state = json!({"letters": {"m-deep": letter_of_0_16(deep.clone(), None)}});
    let text = json!({"actors": [{
        "address": "cicada.dead-letters/_", "state": state, "version": 3, "paused": false, "watches": []
    }]})
    .to_string();
    let mut read: Export = serde_json::from_str(&text).unwrap();
    convert(&mut read).unwrap();
    let converted = &read.actors[0].state["letters"]["m-deep"];
    assert!(converted.get("payload").is_none());
    assert_eq!(converted["payloadJson"], json!(deep.to_string()));

    let store = Store::memory_on(Arc::new(TokioClock::new(default_start())));
    let node = node_of_0_17(&store);
    node.initialize().await.unwrap();
    node.create(&w(), &NoState {}).await.unwrap();
    restore(&node, &read).await.unwrap();
    node.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: id("m-deep") }).await.unwrap();
    run(&node).await;
    let held = letters(&node, "cicada.dead-letters/_").await;
    let letter = &held.letters[&id("m-deep")];
    assert_eq!(letter.redrive_error.as_deref(), Some("The payload is nested deeper than 64 levels"));
    assert_eq!(letter.payload.value().unwrap(), deep, "kept as text");
    node.send(&DeadLetters::default(), DeadLettersMsg::Discard { message_id: id("m-deep") }).await.unwrap();
    run(&node).await;
    assert!(letters(&node, "cicada.dead-letters/_").await.letters.is_empty());

    for broken in [
        json!({"letters": {"m": letter_of_0_16(json!(1), None).as_object().unwrap().clone().into_iter()
            .chain([("payloadJson".to_owned(), json!("1"))]).collect::<serde_json::Map<_, _>>()}}),
        json!({"letters": []}),
    ] {
        let text = json!({"actors": [{
            "address": "cicada.dead-letters/_", "state": broken, "version": 0, "paused": false, "watches": []
        }]});
        let mut read: Export = serde_json::from_value(text).unwrap();
        assert!(matches!(convert(&mut read).unwrap_err(), UpgradeError::Invalid { .. }), "{broken}");
    }
}

/// Scenario: Incomplete export or unquiesced work. A state that `restore` would refuse only for a NUL character, in a
/// value or in a key, is refused by the check of `convert` and by `reset`, which deletes nothing and leaves the
/// registration of 0.16.
#[tokio::test(start_paused = true)]
async fn a_state_with_a_nul_character_is_refused_before_anything_is_deleted() {
    let (_, manage, _) = as_0_16_left_it().await;
    let before = export(&manage).await.unwrap();
    for (what, extra) in [("value", json!({"extra": "a\u{0}b"})), ("key", json!({"a\u{0}b": 1}))] {
        let mut broken = before.clone();
        broken.actors[0].state.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let refused = convert(&mut broken.clone()).unwrap_err();
        assert!(matches!(&refused, UpgradeError::Invalid { why, .. } if why.contains("NUL")), "{what}: {refused}");
        assert!(matches!(check(&broken), Err(UpgradeError::Invalid { .. })), "{what}");
        let refused = reset(&manage, &broken).await.unwrap_err();
        assert!(matches!(&refused, UpgradeError::Invalid { why, .. } if why.contains("NUL")), "{what}: {refused}");

        assert_eq!(export(&manage).await.unwrap(), before, "{what}: every actor as it was");
        let registered = manage.register_contract(&contract_of_0_16()).await.unwrap();
        assert!(matches!(registered, RegisterContractResult::Registered(_)), "{what}: still 0.16: {registered:?}");
    }
}
