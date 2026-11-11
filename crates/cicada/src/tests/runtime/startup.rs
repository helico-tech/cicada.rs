//! Node start: validation of hosted types and configuration, contract registration.
//!
//! A missing handler and a missing unhandled policy are not tested here: a handler is one exhaustive
//! `match` and `UNHANDLED` is a required associated constant, so both are checked by the compiler (ADR 0003).

use std::sync::Arc;
use std::time::Duration;

use super::common::*;
use crate::contract::Contract;
use crate::prelude::*;
use crate::runtime::{NodeConfig, NodeState};
use crate::schema;
use crate::store::{NodeId, Store};
use crate::testing::{InMemoryStore, VirtualClock};
use crate::{ActorSystem, StartError};
use serde::{Deserialize, Serialize};

fn store() -> Arc<dyn Store> {
    Arc::new(InMemoryStore::with_clock(Arc::new(VirtualClock::default())))
}

/// Another type for `counter`, with a different contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OtherCounter {
    id: String,
}

impl ActorRef for OtherCounter {
    const TYPE: &'static str = "counter";
    type Message = OtherMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum OtherMsg {
    Add {
        n: i64,
        #[serde(default)]
        note: String,
    },
}

/// The same shape as `counter`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SameShape {
    x: String,
}

impl ActorRef for SameShape {
    const TYPE: &'static str = "counter";
    type Message = OtherMsg;
}

macro_rules! other_type {
    ($name:ident, $ref:ty) => {
        struct $name;

        impl Actor for $name {
            type Ref = $ref;
            type State = NoState;
            type Message = OtherMsg;
            const UNHANDLED: Unhandled = Unhandled::Fail;

            async fn handle(&self, _turn: &mut Turn<Self>, _message: OtherMsg) -> HandlerResult {
                Ok(())
            }
        }
    };
}

other_type!(OtherActor, OtherCounter);
other_type!(SameShapeActor, SameShape);

counter_type!(SameCounter, Unhandled::Fail);

/// A `documented` type with state and one message, all in the calling module: `Doc`, `DocState`, `DocMsg`.
macro_rules! documented_type {
    () => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Doc {
            id: String,
        }

        impl ActorRef for Doc {
            const TYPE: &'static str = "documented";
            type Message = DocMsg;
        }

        pub struct DocActor;

        impl Actor for DocActor {
            type Ref = Doc;
            type State = DocState;
            type Message = DocMsg;
            const UNHANDLED: Unhandled = Unhandled::Fail;

            async fn handle(&self, _turn: &mut Turn<Self>, _message: DocMsg) -> HandlerResult {
                Ok(())
            }
        }
    };
}

/// The first version of `documented`.
mod doc_old {
    use super::*;

    /// What a document holds.
    #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
    pub struct DocState {
        /// The text.
        text: String,
    }

    /// Messages of a document.
    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    pub enum DocMsg {
        /// Replaces the text.
        Write {
            /// The new text.
            text: String,
        },
    }

    documented_type!();
}

/// `doc_old` with every doc comment reworded.
mod doc_reworded {
    use super::*;

    /// The state of one document, reworded.
    #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
    pub struct DocState {
        /// The whole text of the document.
        text: String,
    }

    /// What a document accepts.
    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    pub enum DocMsg {
        /// Sets the text, dropping the old one.
        Write {
            /// The text to keep from now on.
            text: String,
        },
    }

    documented_type!();
}

/// `doc_old` with a property named `description` in its message.
mod doc_with_description {
    use super::*;

    /// What a document holds.
    #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
    pub struct DocState {
        /// The text.
        text: String,
    }

    /// Messages of a document.
    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    pub enum DocMsg {
        /// Replaces the text.
        Write {
            /// The new text.
            text: String,
            /// A summary of the change.
            #[serde(default)]
            description: String,
        },
    }

    documented_type!();
}

fn single(e: &StartError) -> &str {
    assert_eq!(e.problems.len(), 1, "{:?}", e.problems);
    &e.problems[0]
}

#[tokio::test]
async fn same_contract_registers_on_many_nodes_a_different_contract_for_the_type_name_fails_start() {
    let store = store();
    ActorSystem::builder(store.clone()).host(CounterActor).build().initialize().await.unwrap();
    ActorSystem::builder(store.clone()).host(SameCounter).build().initialize().await.unwrap(); // equal contract: fine
    let e = ActorSystem::builder(store.clone()).host(OtherActor).build().initialize().await.unwrap_err();
    assert!(single(&e).contains("different contract"), "{:?}", e.problems);
}

#[tokio::test]
async fn two_hosted_types_with_one_type_name_or_the_same_type_twice_fail_start() {
    let e = ActorSystem::builder(store()).host(CounterActor).host(SameShapeActor).build().start().await.unwrap_err();
    assert!(single(&e).contains("hosted twice"), "{:?}", e.problems);
    let twice = ActorSystem::builder(store()).host(CounterActor).host(CounterActor).build().start().await.unwrap_err();
    assert!(single(&twice).contains("hosted twice"), "{:?}", twice.problems);
}

#[tokio::test]
async fn invalid_configuration_fails_start() {
    let e = ActorSystem::builder(store())
        .config(NodeConfig {
            lease_ttl: Duration::from_secs(3),
            heartbeat_interval: Some(Duration::from_secs(5)),
            concurrency: 0,
            ..NodeConfig::default()
        })
        .build()
        .start()
        .await
        .unwrap_err();
    assert_eq!(e.problems.len(), 2, "{:?}", e.problems);
}

#[tokio::test]
async fn the_heartbeat_must_leave_room_for_a_retry_within_the_lease() {
    let e = ActorSystem::builder(store())
        .config(NodeConfig {
            lease_ttl: Duration::from_secs(30),
            heartbeat_interval: Some(Duration::from_secs(20)), // > TTL / 2: one failed renewal would lose the lease
            ..NodeConfig::default()
        })
        .build()
        .start()
        .await
        .unwrap_err();
    assert!(single(&e).contains("lease_ttl / 2"), "{:?}", e.problems);
    ActorSystem::builder(store())
        .config(NodeConfig {
            lease_ttl: Duration::from_secs(30),
            heartbeat_interval: Some(Duration::from_secs(15)),
            ..NodeConfig::default()
        })
        .build()
        .initialize()
        .await
        .unwrap();
}

#[tokio::test]
async fn a_node_keeps_its_own_copy_of_the_configuration() {
    let mut config = NodeConfig { node_id: NodeId::new("first").unwrap(), ..NodeConfig::default() };
    let system = ActorSystem::builder(store()).config(config.clone()).host(CounterActor).build();
    config.node_id = NodeId::new("changed").unwrap();
    config.lease_ttl = Duration::ZERO;
    system.initialize().await.unwrap(); // the invalid TTL set afterwards does not affect it
    assert_eq!(system.node_id().as_str(), "first");
    assert_eq!(system.hosted_types().into_iter().collect::<Vec<_>>(), vec![type_name::<Counter>()]);
}

#[tokio::test]
async fn start_registers_contracts_and_a_failed_start_leaves_the_node_unstarted() {
    let store = store();
    let system = ActorSystem::builder(store.clone()).host(CounterActor).host(EchoActor).build();
    system.initialize().await.unwrap();
    let types = store.list_types().await.unwrap();
    assert_eq!(types.iter().map(|t| t.name.to_string()).collect::<Vec<_>>(), vec!["counter", "echo"]);
    let expected = Contract::new(
        type_name::<Counter>(),
        vec!["id".into()],
        schema::root_schema::<CounterState>(),
        schema::message_schemas::<CounterMsg>().unwrap(),
    );
    assert_eq!(types[0].contract_hash, expected.hash());
    assert_eq!(system.state(), NodeState::New);

    let conflicting = ActorSystem::builder(store).host(OtherActor).build();
    assert!(conflicting.start().await.is_err());
    assert_eq!(conflicting.state(), NodeState::New);
}

#[tokio::test]
async fn a_reworded_doc_comment_keeps_the_contract_and_the_registry_keeps_the_first_descriptions() {
    let store = store();
    ActorSystem::builder(store.clone()).host(doc_old::DocActor).build().initialize().await.unwrap();
    ActorSystem::builder(store.clone()).host(doc_reworded::DocActor).build().initialize().await.unwrap();
    let registered = store.list_types().await.unwrap().remove(0);
    let state = &registered.contract["state"];
    assert_eq!(state["description"], "What a document holds.");
    assert_eq!(state["properties"]["text"]["description"], "The text.");
    assert_eq!(registered.contract["messages"]["Write"]["properties"]["text"]["description"], "The new text.");

    let e = ActorSystem::builder(store).host(doc_with_description::DocActor).build().initialize().await.unwrap_err();
    assert!(single(&e).contains("different contract"), "{:?}", e.problems);
}
