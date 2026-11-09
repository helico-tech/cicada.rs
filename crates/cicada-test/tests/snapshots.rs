//! Contract snapshots and state samples.

use cicada::actor::contract_of;
use cicada::contract::{Contract, canonical_json};
use cicada::prelude::*;
use cicada_test::{check_contracts, check_state_decodes, record_state_sample};
use serde::{Deserialize, Serialize};

macro_rules! actor_ref {
    ($name:ident, $type_name:literal, $msg:ty) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct $name {
            id: String,
        }
        impl ActorRef for $name {
            const TYPE: &'static str = $type_name;
            type Message = $msg;
        }
    };
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SnapV1Msg {
    Ping { n: i32 },
    Reset,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SnapV2Msg {
    Ping { n: i32, note: String },
    Pong,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct SnapStateV1 {
    #[serde(default)]
    count: i32,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct SnapStateV2 {
    #[serde(default)]
    count: i64,
}

actor_ref!(SnapV1, "snap", SnapV1Msg);
actor_ref!(SnapV2, "snap", SnapV2Msg);

macro_rules! actor {
    ($name:ident, $ref:ty, $state:ty, $msg:ty) => {
        struct $name;
        impl Actor for $name {
            type Ref = $ref;
            type State = $state;
            type Message = $msg;
            const UNHANDLED: Unhandled = Unhandled::Fail;
            async fn handle(&self, _: &mut Turn<Self>, _: $msg) -> HandlerResult {
                Ok(())
            }
        }
    };
}

actor!(V1, SnapV1, SnapStateV1, SnapV1Msg);
actor!(V2, SnapV2, SnapStateV2, SnapV2Msg);
actor!(StatelessV1, SnapV1, Stateless, SnapV1Msg);

fn v1() -> Contract {
    contract_of::<V1>().unwrap()
}

#[test]
fn a_missing_snapshot_is_written_then_matches() {
    let dir = tempfile::tempdir().unwrap();
    check_contracts(dir.path(), &[v1()], false).unwrap();
    let file = dir.path().join("snap.json");
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(Contract::from_json(&serde_json::from_str(&text).unwrap()).unwrap(), v1());
    assert!(text.lines().count() > 3, "pretty printed");
    check_contracts(dir.path(), &[v1()], false).unwrap();
}

#[test]
fn formatting_does_not_matter() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("snap.json"), canonical_json(&v1().to_json())).unwrap();
    check_contracts(dir.path(), &[v1()], false).unwrap();
}

#[test]
fn a_changed_contract_fails_naming_every_change() {
    let dir = tempfile::tempdir().unwrap();
    check_contracts(dir.path(), &[v1()], false).unwrap();
    let text = check_contracts(dir.path(), &[contract_of::<V2>().unwrap()], false).unwrap_err();
    for expected in [
        "snap",
        "- state schema changed",
        "- message type added: Pong",
        "- message type removed: Reset",
        "- message type changed: Ping",
        "CICADA_UPDATE_CONTRACTS=true",
    ] {
        assert!(text.contains(expected), "{expected} in {text}");
    }
}

#[test]
fn a_type_that_became_stateless_is_named() {
    let dir = tempfile::tempdir().unwrap();
    check_contracts(dir.path(), &[v1()], false).unwrap();
    let text = check_contracts(dir.path(), &[contract_of::<StatelessV1>().unwrap()], false).unwrap_err();
    assert!(text.contains("- state removed (the type is now stateless)"), "{text}");
}

#[test]
fn update_mode_rewrites_a_snapshot_that_differs() {
    let dir = tempfile::tempdir().unwrap();
    let v2 = contract_of::<V2>().unwrap();
    check_contracts(dir.path(), &[v1()], false).unwrap();
    check_contracts(dir.path(), std::slice::from_ref(&v2), true).unwrap();
    check_contracts(dir.path(), &[v2], false).unwrap();
    assert!(check_contracts(dir.path(), &[v1()], false).is_err());
}

#[test]
fn a_snapshot_that_is_not_a_contract_fails() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("snap.json"), r#"{"nope": true}"#).unwrap();
    assert!(check_contracts(dir.path(), &[v1()], false).is_err());
}

#[test]
#[should_panic(expected = "share the snapshot file")]
fn two_contracts_for_one_type_name_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _ = check_contracts(dir.path(), &[v1(), contract_of::<V2>().unwrap()], false);
}

// ---- state samples ----

#[derive(Debug, Serialize, Deserialize)]
struct AddedWithDefault {
    #[serde(default)]
    count: i32,
    #[serde(default = "none")]
    label: String,
}

fn none() -> String {
    "none".into()
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct AddedRequired {
    #[serde(default)]
    count: i32,
    label: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct Renamed {
    total: i32,
}

#[test]
fn a_recorded_sample_decodes_and_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    assert!(record_state_sample(dir.path(), "v1", &SnapStateV1 { count: 3 }));
    assert!(!record_state_sample(dir.path(), "v1", &SnapStateV1 { count: 4 }));
    assert!(std::fs::read_to_string(dir.path().join("v1.json")).unwrap().contains('3'));
    check_state_decodes::<SnapStateV1>(dir.path()).unwrap();
}

#[test]
fn an_additive_change_with_a_default_keeps_old_samples_readable_unknown_keys_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    record_state_sample(dir.path(), "v1", &SnapStateV1 { count: 3 });
    std::fs::write(dir.path().join("extra.json"), r#"{"count": 1, "removedLater": true}"#).unwrap();
    check_state_decodes::<AddedWithDefault>(dir.path()).unwrap();
}

#[test]
fn a_new_required_property_or_a_rename_fails_naming_the_sample() {
    let dir = tempfile::tempdir().unwrap();
    record_state_sample(dir.path(), "v1", &SnapStateV1 { count: 3 });
    let required = check_state_decodes::<AddedRequired>(dir.path()).unwrap_err();
    assert!(required.contains("  - v1.json: "), "{required}");
    assert!(required.contains("label"), "{required}");
    let renamed = check_state_decodes::<Renamed>(dir.path()).unwrap_err();
    assert!(renamed.contains("v1.json"), "{renamed}");
}

#[test]
fn no_samples_is_a_failure_not_a_pass() {
    let dir = tempfile::tempdir().unwrap();
    assert!(check_state_decodes::<SnapStateV1>(dir.path()).is_err());
    assert!(check_state_decodes::<SnapStateV1>(&dir.path().join("missing")).is_err());
}

#[test]
fn odd_sample_names_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let result = std::panic::catch_unwind(|| record_state_sample(dir.path(), "../escape", &SnapStateV1::default()));
    assert!(result.is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
