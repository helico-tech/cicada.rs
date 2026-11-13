//! A host registers a contract without hosting its type (`ActorSystem::register_contract`): on an unstarted system, with
//! a computed hash, first registration wins, the stored descriptions stay, and invalid or over-deep contracts are refused
//! before the store, on memory, SQLite and PostgreSQL (when `CICADA_PG_URL` is set). Public API only.

use std::collections::BTreeMap;
use std::time::Duration;

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::runtime::limits::MAX_SCHEMA_DEPTH;
use cicada::runtime::{NodeState, RegisterContractResult, RegisteredType};
use cicada::store::{MessageStatus, PostgresOptions, RemoveTypeResult, SqliteOptions, TypeDef};
use cicada::{ActorChange, SendResult, StartProblem, Store, TypeName};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Connection;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Counter {
    id: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CounterMsg {
    Add { n: i64 },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct CounterState {
    total: i64,
}

struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        let CounterMsg::Add { n } = message;
        turn.state_mut().total += n;
        Ok(())
    }
}

/// Another actor under the name `counter`, with a different contract.
struct OtherCounter;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OtherRef {
    id: String,
}

impl ActorRef for OtherRef {
    const TYPE: &'static str = "counter";
    type Message = OtherMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum OtherMsg {
    Reset,
}

impl Actor for OtherCounter {
    type Ref = OtherRef;
    type State = CounterState;

    async fn handle(&self, _: &mut Turn<Self>, _: OtherMsg) -> HandlerResult {
        Ok(())
    }
}

fn counter() -> Counter {
    Counter { id: "c1".into() }
}

fn generated() -> Contract {
    contract_of::<CounterActor>().unwrap()
}

fn name(n: &str) -> TypeName {
    TypeName::parse(n).unwrap()
}

/// A contract of `counter` with `state` and one message `Add` with `message`.
fn contract(state: Value, message: Value) -> Contract {
    Contract::new(name("counter"), state, BTreeMap::from([("Add".to_owned(), message)]))
}

/// The stored state schema with the description `text`; every such contract has one hash.
fn described(text: &str) -> Contract {
    contract(json!({"type": "object", "description": text}), json!({"type": "object"}))
}

fn registered(result: RegisterContractResult) -> RegisteredType {
    match result {
        RegisterContractResult::Registered(t) => t,
        other => panic!("expected Registered, got {other:?}"),
    }
}

fn invalid(result: RegisterContractResult) -> String {
    match result {
        RegisterContractResult::InvalidContract(why) => why,
        other => panic!("expected InvalidContract, got {other:?}"),
    }
}

/// A schema `levels` deep: `{"not": {"not": … {}}}`, built without recursion.
fn deep_schema(levels: usize) -> Value {
    (1..levels).fold(json!({}), |inner, _| json!({"not": inner}))
}

/// A schema `levels` deep through its data: `{"default": [[…]]}`.
fn deep_default(levels: usize) -> Value {
    json!({"default": (2..levels).fold(json!([]), |inner, _| json!([inner]))})
}

/// Scenario: Registration on a system without handlers.
#[tokio::test(start_paused = true)]
async fn an_unstarted_system_without_handlers_registers_a_generated_contract() {
    let store = Store::memory();
    let system = ActorSystem::builder(store.clone()).build();
    let mut changes = system.changes("");
    assert!(matches!(changes.next().await, Some(ActorChange::Resync)));

    let stored = registered(system.register_contract(&generated()).await.unwrap());

    assert_eq!(stored.definition.contract_hash, generated().hash());
    assert_eq!(stored.contract.to_json(), generated().to_json());
    let types = system.types().await.unwrap();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].definition, generated().to_type_def());
    assert_eq!(system.state(), NodeState::New);
    assert!(system.hosted_types().is_empty());
    assert!(system.nodes().await.unwrap().is_empty());
    assert!(system.list_actors("", None, 10).await.unwrap().items.is_empty());
    assert!(tokio::time::timeout(Duration::from_secs(5), changes.next()).await.is_err(), "no change hint");

    // A send-only system accepts the counter's messages and leaves them for a host.
    system.create(&counter(), &CounterState::default()).await.unwrap();
    let SendResult::Accepted(message) = system.send(&counter(), CounterMsg::Add { n: 2 }).await.unwrap() else {
        panic!("the send is accepted")
    };
    assert_eq!(system.run_due_turns(10).await.unwrap(), 0);
    assert!(matches!(system.status(&message.id).await.unwrap(), Some(MessageStatus::Pending(_))));
}

/// Scenario: A hash supplied through a registry definition.
#[tokio::test(start_paused = true)]
async fn a_supplied_hash_is_replaced_by_the_computed_one() {
    let system = ActorSystem::builder(Store::memory()).build();
    let def = TypeDef { contract_hash: "sha256:supplied".into(), ..generated().to_type_def() };
    let carrying = Contract::of(&def).unwrap();
    assert_eq!(carrying.hash(), "sha256:supplied");

    let stored = registered(system.register_contract(&carrying).await.unwrap());

    assert_eq!(stored.definition.contract_hash, generated().hash());
    assert_eq!(stored.contract.hash(), generated().hash());
    assert_eq!(system.types().await.unwrap()[0].definition.contract_hash, generated().hash());
}

/// Scenario: An equal registration preserves the stored descriptions.
#[tokio::test(start_paused = true)]
async fn an_equal_registration_keeps_the_first_descriptions() {
    let system = ActorSystem::builder(Store::memory()).build();
    registered(system.register_contract(&described("first")).await.unwrap());

    let again = registered(system.register_contract(&described("second")).await.unwrap());

    assert_eq!(again.definition.contract_hash, described("first").hash());
    assert_eq!(again.contract.state_schema()["description"], "first");
    assert_eq!(system.types().await.unwrap()[0].contract.state_schema()["description"], "first");
}

/// Scenario: A conflicting registration, with and without an actor of the type.
#[tokio::test(start_paused = true)]
async fn a_conflicting_registration_changes_nothing() {
    let system = ActorSystem::builder(Store::memory()).build();
    registered(system.register_contract(&generated()).await.unwrap());
    let other = contract_of::<OtherCounter>().unwrap();

    for with_actor in [false, true] {
        if with_actor {
            system.create(&counter(), &CounterState::default()).await.unwrap();
        }
        let RegisterContractResult::Conflict { existing, requested_hash } =
            system.register_contract(&other).await.unwrap()
        else {
            panic!("a different hash conflicts")
        };
        assert_eq!(existing.definition, generated().to_type_def());
        assert_eq!(requested_hash, other.hash());
        assert_eq!(system.types().await.unwrap()[0].definition, generated().to_type_def(), "{with_actor}");
    }
}

/// Scenario: A peer replaces descriptions after deletion. Both systems have populated caches.
#[tokio::test(start_paused = true)]
async fn a_peer_that_registers_again_with_new_descriptions_is_seen_by_every_system() {
    let store = Store::memory();
    let (a, b) = (ActorSystem::builder(store.clone()).build(), ActorSystem::builder(store.clone()).build());
    registered(a.register_contract(&described("A")).await.unwrap());
    assert_eq!(a.types().await.unwrap()[0].contract.state_schema()["description"], "A");
    assert_eq!(b.types().await.unwrap()[0].contract.state_schema()["description"], "A");

    assert_eq!(b.remove_type(&name("counter")).await.unwrap(), RemoveTypeResult::Removed);
    let replaced = registered(b.register_contract(&described("B")).await.unwrap());
    assert_eq!(replaced.contract.state_schema()["description"], "B");

    for system in [&a, &b] {
        assert_eq!(system.types().await.unwrap()[0].contract.state_schema()["description"], "B");
        let again = registered(system.register_contract(&described("A")).await.unwrap());
        assert_eq!(again.contract.state_schema()["description"], "B");
        assert_eq!(again.definition.contract["state"]["description"], "B");
    }
}

/// Scenario: Concurrent first registrations, on SQLite: one contract wins, equal ones all succeed.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_first_registrations_admit_one_contract() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::sqlite(dir.path().join("cicada.db"), SqliteOptions::default()).await.unwrap();
    let first = generated();
    let second = contract_of::<OtherCounter>().unwrap();
    let calls = (0..16).map(|i| {
        let system = ActorSystem::builder(store.clone()).build();
        let contract = if i % 2 == 0 { first.clone() } else { second.clone() };
        tokio::spawn(async move { system.register_contract(&contract).await.unwrap() })
    });
    let results: Vec<RegisterContractResult> =
        futures::future::join_all(calls).await.into_iter().map(Result::unwrap).collect();

    let types = ActorSystem::builder(store.clone()).build().types().await.unwrap();
    assert_eq!(types.len(), 1);
    let winner = types[0].definition.contract_hash.clone();
    for result in results {
        match result {
            RegisterContractResult::Registered(t) => assert_eq!(t.definition.contract_hash, winner),
            RegisterContractResult::Conflict { existing, requested_hash } => {
                assert_eq!(existing.definition.contract_hash, winner);
                assert_ne!(requested_hash, winner);
            }
            other => panic!("{other:?}"),
        }
    }
    store.close().await;
}

/// Scenario: A local host starts later: the same generated contract initializes, a different one conflicts.
#[tokio::test(start_paused = true)]
async fn a_local_host_started_later_accepts_the_same_contract_and_refuses_another() {
    let store = Store::memory();
    registered(ActorSystem::builder(store.clone()).build().register_contract(&generated()).await.unwrap());

    let same = ActorSystem::builder(store.clone()).host(CounterActor).build();
    same.initialize().await.unwrap();
    same.create(&counter(), &CounterState::default()).await.unwrap();
    same.send(&counter(), CounterMsg::Add { n: 3 }).await.unwrap();
    assert_eq!(same.run_due_turns(10).await.unwrap(), 1);

    let error = ActorSystem::builder(store).host(OtherCounter).build().initialize().await.unwrap_err();
    assert!(matches!(error.problems[..], [StartProblem::ContractConflict { .. }]), "{error}");
}

/// Scenarios: Basic schema shapes, Message names follow send rules, A schema contains a NUL. Nothing is written.
#[tokio::test(start_paused = true)]
async fn invalid_contracts_are_refused_before_the_store() {
    let system = ActorSystem::builder(Store::memory()).build();
    let ok = json!({"type": "object"});
    let with_messages = |messages: Vec<(&str, Value)>| {
        let messages = messages.into_iter().map(|(n, s)| (n.to_owned(), s)).collect();
        Contract::new(name("counter"), ok.clone(), messages)
    };
    let mut refused = Vec::new();
    for root in [json!(null), json!(1), json!("x"), json!([])] {
        refused.push((contract(root.clone(), ok.clone()), "The state schema is not an object or a boolean".into()));
        refused.push((contract(ok.clone(), root), "The schema of message 'Add' is not an object or a boolean".into()));
    }
    refused.push((with_messages(vec![(&"m".repeat(201), ok.clone())]), "The message name is too long".to_owned()));
    refused.push((with_messages(vec![("A\0", ok.clone())]), "The message name contains a NUL character".into()));
    refused.push((contract(json!({"title": "a\0"}), ok.clone()), "The state schema contains a NUL character".into()));
    refused.push((
        contract(ok.clone(), json!({"properties": {"a\0": {}}})),
        "The schema of message 'Add' contains a NUL character".into(),
    ));
    refused.push((contract(json!({"default": ["\0"]}), ok.clone()), "The state schema contains a NUL".into()));

    for (contract, why) in refused {
        let got = invalid(system.register_contract(&contract).await.unwrap());
        assert!(got.starts_with(&why), "{got} / {why}");
    }
    assert!(system.types().await.unwrap().is_empty(), "nothing was written");

    // Boolean roots, an empty map, empty and 200-character names are accepted.
    let boolean = Contract::new(name("flags"), json!(true), BTreeMap::from([("Off".to_owned(), json!(false))]));
    registered(system.register_contract(&boolean).await.unwrap());
    registered(system.register_contract(&Contract::new(name("empty"), json!({}), BTreeMap::new())).await.unwrap());
    let names = with_messages(vec![("", ok.clone()), (&"m".repeat(200), ok.clone())]);
    registered(system.register_contract(&names).await.unwrap());
}

/// Scenario: A schema the HTTP validator cannot compile is stored as it is, without fetching anything.
#[tokio::test(start_paused = true)]
async fn an_unusable_schema_is_stored_without_compiling_it() {
    let system = ActorSystem::builder(Store::memory()).build();
    let message = json!({"$ref": "https://example.invalid/schema.json", "type": "nonsense"});
    let stored = registered(system.register_contract(&contract(json!({}), message.clone())).await.unwrap());
    assert_eq!(stored.contract.message_schemas()["Add"], message);
}

/// Scenarios: The accepted schema depth boundary and A schema beyond the bound, on memory; the SQL stores follow.
#[tokio::test(start_paused = true)]
async fn schemas_of_120_levels_are_registered_and_121_refused_on_memory() {
    let system = ActorSystem::builder(Store::memory()).build();
    check_depth_bound(&system).await;
}

#[tokio::test]
async fn schemas_of_120_levels_survive_a_sqlite_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cicada.db");
    let store = Store::sqlite(&file, SqliteOptions::default()).await.unwrap();
    check_depth_bound(&ActorSystem::builder(store.clone()).build()).await;
    store.close().await;

    let reopened = Store::sqlite(&file, SqliteOptions::default()).await.unwrap();
    check_reopened(&ActorSystem::builder(reopened.clone()).build()).await;
    reopened.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn schemas_of_120_levels_survive_a_postgres_reopen() {
    let Some(url) = std::env::var("CICADA_PG_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("CICADA_PG_URL is unset: skipping a PostgreSQL test");
        return;
    };
    let schema = format!("registration_{}_{}", std::process::id(), &MessageId::random().as_str()[..8]);
    let store = Store::postgres(&url, PostgresOptions::for_schema(&schema)).await.unwrap();
    check_depth_bound(&ActorSystem::builder(store.clone()).build()).await;
    store.close().await;

    let reopened = Store::postgres(&url, PostgresOptions::for_schema(&schema)).await.unwrap();
    check_reopened(&ActorSystem::builder(reopened.clone()).build()).await;
    reopened.close().await;
    let mut c = sqlx::PgConnection::connect(&url).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA \"{schema}\" CASCADE"))).execute(&mut c).await.unwrap();
}

/// The contract at the bound: state and message schemas each [`MAX_SCHEMA_DEPTH`] deep.
fn at_bound() -> Contract {
    Contract::new(name("deep"), deep_schema(MAX_SCHEMA_DEPTH), BTreeMap::from([("Go".to_owned(), deep_default(120))]))
}

/// Refuses 121 levels anywhere, also in data and also beyond the value limit of 64 levels, then registers 120.
async fn check_depth_bound(system: &ActorSystem) {
    assert_eq!(MAX_SCHEMA_DEPTH, 120);
    let beyond = [
        contract(deep_schema(121), json!({})),
        contract(json!({}), deep_schema(121)),
        contract(deep_default(121), json!({})),
        contract(json!({}), deep_default(121)),
    ];
    for c in &beyond {
        let why = invalid(system.register_contract(c).await.unwrap());
        assert!(why.ends_with("is nested deeper than 120 levels"), "{why}");
    }
    assert!(system.types().await.unwrap().is_empty(), "nothing was written");

    registered(system.register_contract(&contract(deep_schema(65), deep_default(100))).await.unwrap());
    let stored = registered(system.register_contract(&at_bound()).await.unwrap());
    assert_eq!(stored.definition, at_bound().to_type_def());
}

/// After a reopen, the type reads back whole and an equal registration succeeds.
async fn check_reopened(system: &ActorSystem) {
    let types = system.types().await.unwrap();
    let deep = types.iter().find(|t| t.name().as_str() == "deep").expect("the deep type is stored");
    assert_eq!(deep.definition, at_bound().to_type_def());
    let again = registered(system.register_contract(&at_bound()).await.unwrap());
    assert_eq!(again.definition, at_bound().to_type_def());
}
