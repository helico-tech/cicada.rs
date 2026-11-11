//! Actor type definitions: what a node checks at start, and the published contract.
//!
//! Checked by the compiler instead, so not tested here: a message without a handler and a duplicate handler (one exhaustive
//! `match`), a missing or doubled unhandled policy (`UNHANDLED` is a required associated constant), a missing type name
//! (`TYPE` is a required associated constant), and a message field named `type` (serde refuses it: "variant field
//! name `type` conflicts with internal tag"). A message family is one flat
//! enum.

use std::sync::Arc;
use std::time::Duration;

use cicada::contract::{Contract, canonical_json};
use cicada::prelude::*;
use cicada::store::{Store, TypeDef};
use cicada::{ActorSystem, StartError};
use cicada_test::{InMemoryStore, VirtualClock};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sync {
    tenant: String,
}

impl ActorRef for Sync {
    const TYPE: &'static str = "sync";
    type Message = SyncMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SyncMsg {
    Run,
    FetchPage { cursor: Option<String> },
    Pause,
    Resume,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SyncState {
    source: String,
    #[serde(default)]
    imported: i32,
}

struct SyncActor;

impl Actor for SyncActor {
    type Ref = Sync;
    type State = SyncState;
    type Message = SyncMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: SyncMsg) -> HandlerResult {
        Ok(())
    }
}

struct EmptySync;

impl Actor for EmptySync {
    type Ref = Sync;
    type State = NoState;
    type Message = SyncMsg;
    const UNHANDLED: Unhandled = Unhandled::Drop;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: SyncMsg) -> HandlerResult {
        Ok(())
    }
}

/// A ref whose type name is malformed.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BadRef {
    name: String,
}

impl ActorRef for BadRef {
    const TYPE: &'static str = "Bad Type";
    type Message = SyncMsg;
}

struct BadActor;

impl Actor for BadActor {
    type Ref = BadRef;
    type State = NoState;
    type Message = SyncMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;
    const MAX_TURN: Option<Duration> = Some(Duration::ZERO);

    async fn handle(&self, _turn: &mut Turn<Self>, _message: SyncMsg) -> HandlerResult {
        Ok(())
    }
}

fn store() -> Arc<dyn Store> {
    Arc::new(InMemoryStore::with_clock(Arc::new(VirtualClock::default())))
}

/// The registry entry a node registers for `actor`.
async fn registered<A: Actor>(actor: A) -> TypeDef {
    let store = store();
    ActorSystem::builder(store.clone()).host(actor).build().initialize().await.unwrap();
    store.list_types().await.unwrap().remove(0)
}

async fn problems<A: Actor>(actor: A) -> Vec<String> {
    let e: StartError = ActorSystem::builder(store()).host(actor).build().initialize().await.unwrap_err();
    e.problems
}

#[tokio::test]
async fn a_complete_definition_has_no_problems() {
    let def = registered(SyncActor).await;
    assert_eq!(def.name.as_str(), "sync");
    let messages: Vec<&String> = def.contract["messages"].as_object().unwrap().keys().collect();
    assert_eq!(messages, vec!["FetchPage", "Pause", "Resume", "Run"]);
}

#[tokio::test]
async fn a_bad_ref_and_a_zero_max_turn_are_reported() {
    let problems = problems(BadActor).await;
    assert!(problems.iter().any(|p| p.contains("Ref:")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("MAX_TURN must be positive")), "{problems:?}");
}

#[tokio::test]
async fn a_type_that_keeps_no_data_has_the_schema_of_an_empty_object() {
    let def = registered(EmptySync).await;
    let state = &def.contract["state"];
    assert_eq!(state["type"], "object");
    assert_eq!(state["title"], "NoState");
    assert!(state.get("properties").is_none_or(|p| p == &serde_json::json!({})), "no properties: {state}");
    assert_eq!(state["additionalProperties"], false, "NoState refuses unknown fields");
    assert_eq!(Contract::of(&def).unwrap().state_schema(), Some(state));
}

#[tokio::test]
async fn contract_snapshot_and_stable_hash() {
    let def = registered(SyncActor).await;
    // The schemas come from schemars.
    let expected = concat!(
        r#"{"key":["tenant"],"messages":{"#,
        r#""FetchPage":{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"cursor":{"type":["string","null"]}},"title":"FetchPage","type":"object"},"#,
        r#""Pause":{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{},"title":"Pause","type":"object"},"#,
        r#""Resume":{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{},"title":"Resume","type":"object"},"#,
        r#""Run":{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{},"title":"Run","type":"object"}},"#,
        r#""state":{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"imported":{"default":0,"format":"int32","type":"integer"},"source":{"type":"string"}},"required":["source"],"title":"SyncState","type":"object"},"#,
        r#""type":"sync"}"#
    );
    assert_eq!(canonical_json(&def.contract), expected);
    let contract = Contract::from_json(&def.contract).unwrap();
    assert_eq!(contract.hash(), def.contract_hash);
    assert_eq!(registered(SyncActor).await.contract_hash, def.contract_hash);
    assert_eq!(contract.to_type_def().contract_hash, def.contract_hash);
}

#[tokio::test]
async fn a_different_state_type_changes_the_hash() {
    let stateful = registered(SyncActor).await;
    let empty = registered(EmptySync).await;
    assert_ne!(stateful.contract_hash, empty.contract_hash);
    assert!(empty.contract.get("messages").is_some());
}
