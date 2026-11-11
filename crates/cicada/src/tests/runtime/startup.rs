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
use crate::store::{Backend, NodeId, Store};
use crate::testing::VirtualClock;
use crate::{ActorSystem, StartError};
use serde::{Deserialize, Serialize};

fn store() -> Store {
    Store::memory_on(Arc::new(VirtualClock::default()))
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

/// Refs of each shape of ADR 0021, with `OtherMsg`.
mod shapes {
    use std::marker::PhantomData;

    use super::*;

    macro_rules! shape {
        ($ref:ty, $actor:ident, $name:literal) => {
            impl ActorRef for $ref {
                const TYPE: &'static str = $name;
                type Message = OtherMsg;
            }

            pub(super) struct $actor;

            impl Actor for $actor {
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

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub(super) struct TwoFields {
        batch: String,
        index: i32,
    }
    shape!(TwoFields, TwoFieldsActor, "batch");

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub(super) struct Number {
        id: i32,
    }
    shape!(Number, NumberActor, "number");

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub(super) enum Region {
        Eu,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub(super) struct Regional {
        region: Region,
    }
    shape!(Regional, RegionalActor, "regional");

    /// One string and a field that serde skips, as Chorus's `Named<A>`.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub(super) struct Named<A> {
        name: String,
        #[serde(skip)]
        kind: PhantomData<A>,
    }

    #[derive(Debug, Clone)]
    pub(super) struct Nightly;
    shape!(Named<Nightly>, NamedActor, "cron");

    /// Typed parts, written as one string.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(into = "String", try_from = "String")]
    pub(super) struct BatchItem {
        batch: String,
        index: i32,
    }
    shape!(BatchItem, BatchItemActor, "batch-items");

    impl From<BatchItem> for String {
        fn from(item: BatchItem) -> Self {
            format!("{}.{}", item.batch, item.index)
        }
    }

    impl TryFrom<String> for BatchItem {
        type Error = String;

        fn try_from(name: String) -> Result<Self, String> {
            let (batch, index) = name.rsplit_once('.').ok_or("not batch.index")?;
            Ok(Self { batch: batch.to_owned(), index: index.parse().map_err(|_| "not an index")? })
        }
    }

    /// A struct with braces and no fields: a singleton, as a unit struct is.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub(super) struct Braces {}
    shape!(Braces, BracesActor, "braces");

    /// The ref of `counter` with its one field under another name.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub(super) struct RenamedCounter {
        name: String,
    }

    impl ActorRef for RenamedCounter {
        const TYPE: &'static str = "counter";
        type Message = CounterMsg;
    }

    pub(super) struct RenamedCounterActor;

    impl Actor for RenamedCounterActor {
        type Ref = RenamedCounter;
        type State = CounterState;
        type Message = CounterMsg;
        const UNHANDLED: Unhandled = Unhandled::Fail;

        async fn handle(&self, _turn: &mut Turn<Self>, _message: CounterMsg) -> HandlerResult {
            Ok(())
        }
    }
}

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

/// Scenarios: A ref with several fields; A ref with a number; A ref with an enum variant.
#[tokio::test]
async fn a_node_whose_ref_is_not_one_string_does_not_start_and_names_the_ref() {
    let start = async |system: ActorSystem| system.initialize().await.unwrap_err();
    let two = start(ActorSystem::builder(store()).host(shapes::TwoFieldsActor).build()).await;
    let number = start(ActorSystem::builder(store()).host(shapes::NumberActor).build()).await;
    let regional = start(ActorSystem::builder(store()).host(shapes::RegionalActor).build()).await;
    for (e, name, what) in [
        (&two, "shapes::TwoFields", "asks for the fields batch, index"),
        (&number, "shapes::Number", "its field id asks for a number"),
        (&regional, "shapes::Regional", "its field region asks for an enum"),
    ] {
        let problem = single(e);
        assert!(problem.contains("Ref:") && problem.contains(name) && problem.contains(what), "{problem}");
    }
}

/// Scenarios: A field that serde skips; A ref with typed parts that writes itself as a string.
#[tokio::test]
async fn a_node_starts_with_a_ref_that_skips_a_field_or_writes_its_parts_as_one_string() {
    let system = ActorSystem::builder(store()).host(shapes::NamedActor).host(shapes::BatchItemActor).build();
    system.initialize().await.expect("the node starts");
}

/// Scenario: A struct without fields.
#[tokio::test]
async fn a_struct_without_fields_is_a_singleton() {
    assert_eq!(shapes::Braces {}.address().unwrap().as_str(), "braces/_");
    let system = ActorSystem::builder(store()).host(shapes::BracesActor).build();
    system.initialize().await.expect("the node starts");
    let create = async |key: &str| {
        system.create_actor(&format!("braces/{key}").parse().unwrap(), serde_json::json!({})).await.unwrap()
    };
    match create("other").await {
        crate::CreateActorResult::InvalidAddress(e) => assert!(e.contains("singleton"), "{e}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(create("_").await, crate::CreateActorResult::Created);
}

/// Scenario: Changing a ref's fields.
#[tokio::test]
async fn renaming_the_field_of_a_ref_keeps_the_contract() {
    let store = store();
    ActorSystem::builder(store.clone()).host(CounterActor).build().initialize().await.unwrap();
    ActorSystem::builder(store).host(shapes::RenamedCounterActor).build().initialize().await.expect("same contract");
    let renamed = crate::actor::contract_of::<shapes::RenamedCounterActor>().unwrap();
    assert_eq!(renamed.hash(), crate::actor::contract_of::<CounterActor>().unwrap().hash());
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
