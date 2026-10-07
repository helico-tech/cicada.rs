//! Typed refs are checked against the TypeRegistry where the compiler cannot (a ref type that is not registered, the
//! initial state of a created type), and `self` is the claimed address even when it is not canonical (Kotlin
//! `TypedEffectsTest`).

mod common;

use cicada::address::ActorAddress;
use cicada::prelude::*;
use cicada::runtime::{CreateActorResult, SendResult};
use cicada::store::{OutcomeStatus, Store};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

// Overlapping templates: actor://jobs/cleanup is also an address of actor://jobs/{id}, so the two types cannot both be
// registered. The tests register Cleanup only and use Jobs as a ref type that no node hosts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Jobs {
    id: String,
}

impl ActorRef for Jobs {
    const TEMPLATE: &'static str = "actor://jobs/{id}";
    type Message = JobsMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum JobsMsg {
    RunJob,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct JobsState {
    #[serde(default)]
    runs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cleanup;

impl ActorRef for Cleanup {
    const TEMPLATE: &'static str = "actor://jobs/cleanup";
    type Message = CleanupMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CleanupMsg {
    Sweep,
}

struct CleanupActor;

impl Actor for CleanupActor {
    type Ref = Cleanup;
    type State = Stateless;
    type Message = CleanupMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: CleanupMsg) -> HandlerResult {
        Ok(())
    }
}

/// A ref with an integer parameter: actor://num/7 is canonical, actor://num/007 is not.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Num {
    n: i32,
}

impl ActorRef for Num {
    const TEMPLATE: &'static str = "actor://num/{n}";
    type Message = NumMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum NumMsg {
    Again { left: u32 },
}

struct NumActor;

impl Actor for NumActor {
    type Ref = Num;
    type State = Stateless;
    type Message = NumMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: NumMsg) -> HandlerResult {
        let NumMsg::Again { left } = message;
        if left > 0 {
            let me = turn.self_ref().clone();
            turn.send(&me, NumMsg::Again { left: left - 1 })?;
        }
        Ok(())
    }
}

/// What a handler does with refs: creates, sends.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Maker {
    id: String,
}

impl ActorRef for Maker {
    const TEMPLATE: &'static str = "actor://maker/{id}";
    type Message = MakerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum MakerMsg {
    MakeJob { id: String },
    PokeJob { id: String },
    MakeEcho,
    MakeBadCounter,
}

struct MakerActor;

impl Actor for MakerActor {
    type Ref = Maker;
    type State = Stateless;
    type Message = MakerMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: MakerMsg) -> HandlerResult {
        match message {
            MakerMsg::MakeJob { id } => turn.create(&Jobs { id }, &JobsState::default())?,
            MakerMsg::PokeJob { id } => {
                turn.send(&Jobs { id }, JobsMsg::RunJob)?;
            }
            MakerMsg::MakeEcho => turn.create(&Echo::new("e"), &JobsState::default())?,
            MakerMsg::MakeBadCounter => turn.create(&Counter::new("bad"), "not a counter state")?,
        }
        Ok(())
    }
}

fn m() -> Maker {
    Maker { id: "m".into() }
}

fn parse(uri: &str) -> ActorAddress {
    ActorAddress::parse(uri).unwrap()
}

async fn maker_and_cleanup() -> TestActors {
    let test = TestActors::builder().with(|n| n.host(MakerActor).host(CleanupActor)).build().await.unwrap();
    test.create_stateless(&m()).await;
    test
}

#[tokio::test(start_paused = true)]
async fn a_handler_cannot_create_an_instance_whose_address_belongs_to_another_type() {
    let test = maker_and_cleanup().await;
    let wrong = test.send(&m(), MakerMsg::MakeJob { id: "cleanup".into() }).await;
    let unknown = test.send(&m(), MakerMsg::MakeJob { id: "42".into() }).await;
    test.run_until_idle().await;
    let failed = test.outcome(&wrong).await.unwrap();
    assert_eq!(failed.status, OutcomeStatus::Failed);
    assert!(failed.error.as_deref().unwrap().contains("belongs to type actor://jobs/cleanup"), "{:?}", failed.error);
    assert!(test.store().get(&parse("actor://jobs/cleanup")).await.unwrap().is_none(), "nothing created");
    let unregistered = test.outcome(&unknown).await.unwrap();
    assert_eq!(unregistered.status, OutcomeStatus::Failed);
    assert!(
        unregistered.error.as_deref().unwrap().contains("no registered actor type matches"),
        "{:?}",
        unregistered.error
    );
}

#[tokio::test(start_paused = true)]
async fn a_handler_cannot_send_through_a_ref_whose_address_belongs_to_another_type() {
    let test = maker_and_cleanup().await;
    test.create_stateless(&Cleanup).await;
    let wrong = test.send(&m(), MakerMsg::PokeJob { id: "cleanup".into() }).await;
    test.run_until_idle().await;
    let failed = test.outcome(&wrong).await.unwrap();
    assert_eq!(failed.status, OutcomeStatus::Failed);
    assert!(failed.error.as_deref().unwrap().contains("belongs to type actor://jobs/cleanup"), "{:?}", failed.error);
    assert!(test.pending(&Cleanup).await.is_empty(), "nothing sent");
}

#[tokio::test(start_paused = true)]
async fn a_handler_create_checks_the_initial_state_against_the_created_type() {
    let test = TestActors::builder()
        .with(|n| n.host(MakerActor).host(EchoActor).host(CounterActor))
        .build()
        .await
        .unwrap();
    test.create_stateless(&m()).await;
    let stateless = test.send(&m(), MakerMsg::MakeEcho).await;
    let undecodable = test.send(&m(), MakerMsg::MakeBadCounter).await;
    test.run_until_idle().await;
    let error = test.outcome(&stateless).await.unwrap().error.unwrap();
    assert!(error.contains("stateless"), "{error}");
    let error = test.outcome(&undecodable).await.unwrap().error.unwrap();
    assert!(error.contains("cannot be decoded"), "{error}");
    assert!(!test.exists(&Echo::new("e")).await);
    assert!(!test.exists(&Counter::new("bad")).await);
}

#[tokio::test(start_paused = true)]
async fn a_hosting_node_refuses_to_create_an_instance_under_a_non_canonical_address() {
    let test = TestActors::builder().with(|n| n.host(NumActor)).build().await.unwrap();
    match test.system().create_actor(&parse("actor://num/007"), None).await.unwrap() {
        CreateActorResult::InvalidAddress(e) => assert!(e.contains("actor://num/7"), "{e}"),
        other => panic!("{other:?}"),
    }
    let seven = test.system().create_actor(&parse("actor://num/seven"), None).await.unwrap();
    assert!(matches!(seven, CreateActorResult::InvalidAddress(_)), "{seven:?}");
    assert_eq!(test.system().create_actor(&parse("actor://num/7"), None).await.unwrap(), CreateActorResult::Created);
}

#[tokio::test(start_paused = true)]
async fn self_is_the_claimed_address_even_when_it_is_not_canonical() {
    let test = TestActors::builder().with(|n| n.host(NumActor)).build().await.unwrap();
    // Created through a node that does not host the type (no canonical check there), e.g. over HTTP.
    let odd = parse("actor://num/007");
    test.store().create(&odd, &template::<Num>(), None).await.unwrap();
    let first = match test.send_raw(&odd, "Again", json!({"left": 2}), SendOptions::new()).await {
        SendResult::Accepted(m) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(test.run_until_idle().await, 3, "the self-sends reached the same instance");
    assert_eq!(test.outcome(&first).await.unwrap().status, OutcomeStatus::Done);
    assert!(test.failures().await.is_empty());
}
