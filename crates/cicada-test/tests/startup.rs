//! Node start: validation of hosted types and configuration, contract registration.
//!
//! A missing handler and a missing unhandled policy are not tested here: a handler is one exhaustive
//! `match` and `UNHANDLED` is a required associated constant, so both are checked by the compiler (ADR 0003).

#[macro_use]
mod common;

use std::sync::Arc;
use std::time::Duration;

use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::runtime::{NodeConfig, NodeState};
use cicada::schema;
use cicada::store::{NodeId, Store};
use cicada::{ActorSystem, StartError};
use cicada_test::{InMemoryStore, VirtualClock};
use common::*;
use serde::{Deserialize, Serialize};

fn store() -> Arc<dyn Store> {
    Arc::new(InMemoryStore::with_clock(Arc::new(VirtualClock::default())))
}

/// Another type for `actor://counter/{id}`, with a different contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OtherCounter {
    id: String,
}

impl ActorRef for OtherCounter {
    const TEMPLATE: &'static str = "actor://counter/{id}";
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

/// The same shape as `actor://counter/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SameShape {
    x: String,
}

impl ActorRef for SameShape {
    const TEMPLATE: &'static str = "actor://counter/{x}";
    type Message = OtherMsg;
}

/// Overlaps `actor://counter/{id}` in one address only: `actor://counter/main`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MainCounter;

impl ActorRef for MainCounter {
    const TEMPLATE: &'static str = "actor://counter/main";
    type Message = OtherMsg;
}

macro_rules! other_type {
    ($name:ident, $ref:ty) => {
        struct $name;

        impl Actor for $name {
            type Ref = $ref;
            type State = Stateless;
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
other_type!(MainActor, MainCounter);

counter_type!(SameCounter, Unhandled::Fail);

fn single(e: &StartError) -> &str {
    assert_eq!(e.problems.len(), 1, "{:?}", e.problems);
    &e.problems[0]
}

#[tokio::test]
async fn same_contract_registers_on_many_nodes_a_different_contract_for_the_template_fails_start() {
    let store = store();
    ActorSystem::builder(store.clone()).host(CounterActor).build().initialize().await.unwrap();
    ActorSystem::builder(store.clone()).host(SameCounter).build().initialize().await.unwrap(); // equal contract: fine
    let e = ActorSystem::builder(store.clone()).host(OtherActor).build().initialize().await.unwrap_err();
    assert!(single(&e).contains("different contract"), "{:?}", e.problems);
}

#[tokio::test]
async fn two_hosted_types_whose_templates_overlap_or_the_same_type_twice_fail_start() {
    let e = ActorSystem::builder(store()).host(CounterActor).host(SameShapeActor).build().start().await.unwrap_err();
    assert!(single(&e).contains("overlaps actor://counter/{id}"), "{:?}", e.problems);
    let literal = ActorSystem::builder(store()).host(CounterActor).host(MainActor).build().start().await.unwrap_err();
    assert!(single(&literal).contains("both match actor://counter/main"), "{:?}", literal.problems);
    let twice = ActorSystem::builder(store()).host(CounterActor).host(CounterActor).build().start().await.unwrap_err();
    assert!(single(&twice).contains("hosted twice"), "{:?}", twice.problems);
}

#[tokio::test]
async fn a_type_overlapping_one_another_node_registered_fails_start() {
    let store = store();
    ActorSystem::builder(store.clone()).host(CounterActor).build().initialize().await.unwrap();
    let e = ActorSystem::builder(store).host(MainActor).build().initialize().await.unwrap_err();
    assert!(single(&e).contains("overlaps actor://counter/{id}"), "{:?}", e.problems);
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
    assert_eq!(system.hosted_templates().into_iter().collect::<Vec<_>>(), vec![template::<Counter>()]);
}

#[tokio::test]
async fn start_registers_contracts_and_a_failed_start_leaves_the_node_unstarted() {
    let store = store();
    let system = ActorSystem::builder(store.clone()).host(CounterActor).host(EchoActor).build();
    system.initialize().await.unwrap();
    let types = store.list_types().await.unwrap();
    assert_eq!(
        types.iter().map(|t| t.template.to_string()).collect::<Vec<_>>(),
        vec!["actor://counter/{id}", "actor://echo/{name}"]
    );
    let expected = Contract::new(
        template::<Counter>(),
        Some(schema::root_schema::<CounterState>()),
        schema::message_schemas::<CounterMsg>().unwrap(),
    );
    assert_eq!(types[0].contract_hash, expected.hash());
    assert_eq!(system.state(), NodeState::New);

    let conflicting = ActorSystem::builder(store).host(OtherActor).build();
    assert!(conflicting.start().await.is_err());
    assert_eq!(conflicting.state(), NodeState::New);
}
