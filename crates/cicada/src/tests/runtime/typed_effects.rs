//! Typed refs are checked where the compiler cannot: against the TypeRegistry (a ref type that is not registered, the
//! initial state of a created type), by a hosting node (an address its ref does not accept), and at the call (a ref
//! that serde does not write as one string has no address).

use super::common::*;
use crate::address::ActorAddress;
use crate::prelude::*;
use crate::runtime::{CreateActorResult, SendResult};
use crate::store::OutcomeStatus;
use crate::testing::TestActors;
use crate::turn::TurnError;
use serde::{Deserialize, Serialize};
use serde_json::json;

// Jobs is a ref type that no node hosts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Jobs {
    id: String,
}

impl ActorRef for Jobs {
    const TYPE: &'static str = "jobs";
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

/// A ref with a reader of its own: its name is a number.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
struct Num(u32);

impl From<Num> for String {
    fn from(n: Num) -> Self {
        n.0.to_string()
    }
}

impl TryFrom<String> for Num {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        name.parse().map(Num).map_err(|_| format!("'{name}' is not a number"))
    }
}

impl ActorRef for Num {
    const TYPE: &'static str = "num";
    type Message = NumMsg;
}

/// A singleton.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Solo;

impl ActorRef for Solo {
    const TYPE: &'static str = "solo";
    type Message = NumMsg;
}

struct SoloActor;

impl Actor for SoloActor {
    type Ref = Solo;
    type State = NoState;
    type Message = NumMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _: &mut Turn<Self>, _: NumMsg) -> HandlerResult {
        Ok(())
    }
}

/// A ref of two fields, which serde does not write as one string; no node hosts its type.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pair {
    batch: String,
    index: i32,
}

impl ActorRef for Pair {
    const TYPE: &'static str = "pairs";
    type Message = JobsMsg;
}

/// A ref whose one field serde skips when it is empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkipEmpty {
    #[serde(skip_serializing_if = "String::is_empty")]
    name: String,
}

impl ActorRef for SkipEmpty {
    const TYPE: &'static str = "skip-empty";
    type Message = JobsMsg;
}

/// Hosts `SkipEmpty`: its node starts, and a ref with an empty name still has no address.
struct SkipEmptyActor;

impl Actor for SkipEmptyActor {
    type Ref = SkipEmpty;
    type State = NoState;
    type Message = JobsMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _: &mut Turn<Self>, _: JobsMsg) -> HandlerResult {
        Ok(())
    }
}

/// A ref whose one field is an `Option` that serde skips when it is `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkipNone {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl ActorRef for SkipNone {
    const TYPE: &'static str = "skip-none";
    type Message = JobsMsg;
}

/// A ref whose one field serde never writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NeverWritten {
    #[serde(skip_serializing)]
    #[allow(dead_code)] // read only by serde, which never writes it: the point of this ref
    name: String,
}

impl ActorRef for NeverWritten {
    const TYPE: &'static str = "never-written";
    type Message = JobsMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum NumMsg {
    Again { left: u32 },
}

struct NumActor;

impl Actor for NumActor {
    type Ref = Num;
    type State = NoState;
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
    const TYPE: &'static str = "maker";
    type Message = MakerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum MakerMsg {
    MakeJob {
        id: String,
    },
    PokeJob {
        id: String,
    },
    MakeEcho,
    MakeBadCounter,
    /// Sends to a `Pair`, which fails at the call; ends `done` only then.
    PokePair,
    /// Creates a ref of each kind that declares a field and writes none; ends `done` only when each fails at the call.
    MakeUnwritten,
}

struct MakerActor;

impl Actor for MakerActor {
    type Ref = Maker;
    type State = NoState;
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
            MakerMsg::PokePair => match turn.send(&Pair { batch: "b1".into(), index: 7 }, JobsMsg::RunJob) {
                Err(TurnError::Invalid(e)) if e.contains("Pair has no address") => {}
                other => return Err(turn.fail(format!("expected a refused send, got {other:?}"))),
            },
            MakerMsg::MakeUnwritten => {
                let created = [
                    turn.create(&SkipEmpty { name: String::new() }, &NoState {}),
                    turn.create(&SkipNone { name: None }, &NoState {}),
                    turn.create(&NeverWritten { name: "n".into() }, &NoState {}),
                ];
                for result in created {
                    match result {
                        Err(TurnError::Invalid(e)) if e.contains("has no address") => {}
                        other => return Err(turn.fail(format!("expected a refused create, got {other:?}"))),
                    }
                }
            }
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

async fn maker() -> TestActors {
    let test = TestActors::builder().with(|n| n.host(MakerActor)).build().await.unwrap();
    test.create(&m(), &NoState {}).await;
    test
}

#[tokio::test(start_paused = true)]
async fn a_handler_cannot_create_an_instance_of_an_unregistered_type() {
    let test = maker().await;
    let unknown = test.send(&m(), MakerMsg::MakeJob { id: "42".into() }).await;
    test.run_until_idle().await;
    let unregistered = test.outcome(&unknown).await.unwrap();
    assert_eq!(unregistered.status, OutcomeStatus::Failed);
    assert!(
        unregistered.error.as_deref().unwrap().contains("jobs/42: its type is not registered"),
        "{:?}",
        unregistered.error
    );
}

#[tokio::test(start_paused = true)]
async fn a_handler_send_to_an_unregistered_type_finds_no_instance() {
    let test = maker().await;
    let unknown = test.send(&m(), MakerMsg::PokeJob { id: "42".into() }).await;
    test.run_until_idle().await;
    let failed = test.outcome(&unknown).await.unwrap();
    assert_eq!(failed.status, OutcomeStatus::Failed);
    assert!(failed.error.as_deref().unwrap().contains("absent instance jobs/42"), "{:?}", failed.error);
}

#[tokio::test(start_paused = true)]
async fn a_handler_create_checks_the_initial_state_against_the_created_type() {
    let test =
        TestActors::builder().with(|n| n.host(MakerActor).host(EchoActor).host(CounterActor)).build().await.unwrap();
    test.create(&m(), &NoState {}).await;
    let with_a_field = test.send(&m(), MakerMsg::MakeEcho).await;
    let undecodable = test.send(&m(), MakerMsg::MakeBadCounter).await;
    test.run_until_idle().await;
    let error = test.outcome(&with_a_field).await.unwrap().error.unwrap();
    assert!(error.contains("cannot be decoded") && error.contains("unknown field"), "{error}");
    let error = test.outcome(&undecodable).await.unwrap().error.unwrap();
    assert!(error.contains("cannot be decoded"), "{error}");
    assert!(!test.exists(&Echo::new("e")).await);
    assert!(!test.exists(&Counter::new("bad")).await);
}

#[tokio::test(start_paused = true)]
async fn a_hosting_node_refuses_to_create_an_instance_under_an_address_its_ref_does_not_accept() {
    let test = TestActors::builder().with(|n| n.host(NumActor).host(SoloActor)).build().await.unwrap();
    let create = async |address: &str| test.system().create_actor(&parse(address), json!({})).await.unwrap();
    match create("num/seven").await {
        CreateActorResult::InvalidAddress(e) => assert!(e.contains("'seven' is not a number"), "{e}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(create("num/7").await, CreateActorResult::Created);
    match create("solo/other").await {
        CreateActorResult::InvalidAddress(e) => assert!(e.contains("singleton"), "{e}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(create("solo/_").await, CreateActorResult::Created);
}

#[tokio::test(start_paused = true)]
async fn a_ref_with_a_reader_of_its_own_sends_to_itself() {
    let test = TestActors::builder().with(|n| n.host(NumActor)).build().await.unwrap();
    test.create(&Num(7), &NoState {}).await;
    let first = match test.send_raw(&parse("num/7"), "Again", json!({"left": 2}), SendOptions::new()).await {
        SendResult::Accepted(m) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(test.run_until_idle().await, 3, "the self-sends reached the same instance");
    assert_eq!(test.outcome(&first).await.unwrap().status, OutcomeStatus::Done);
    assert!(test.failures().await.is_empty());
}

/// Scenario: A ref of a type the node does not host.
#[tokio::test(start_paused = true)]
async fn a_send_to_a_ref_that_is_not_one_string_fails_at_the_call_and_the_turn_goes_on() {
    let test = maker().await;
    let poke = test.send(&m(), MakerMsg::PokePair).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&poke).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done, "{:?}", outcome.error);
}

/// A ref that declares a field and writes none has no address: a create fails at the call, in a turn and from outside,
/// also on a node that hosts the type.
#[tokio::test(start_paused = true)]
async fn a_ref_that_declares_a_field_and_writes_none_fails_at_the_call() {
    let test = TestActors::builder().with(|n| n.host(MakerActor).host(SkipEmptyActor)).build().await.unwrap();
    test.create(&m(), &NoState {}).await;
    let make = test.send(&m(), MakerMsg::MakeUnwritten).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&make).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done, "{:?}", outcome.error);
    match test.system().create(&SkipEmpty { name: String::new() }, &NoState {}).await {
        Err(crate::OpError::Invalid(e)) => assert!(e.contains("skips its field name"), "{e}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(test.system().get_actor(&parse("skip-empty/_")).await.unwrap(), None);
}

// ---- a ref that changed its shape, read where it is held ----

/// A ref that had the fields `batch` and `index` and is now written as `batch.index`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
struct Item {
    batch: String,
    index: i32,
}

impl From<Item> for String {
    fn from(item: Item) -> Self {
        format!("{}.{}", item.batch, item.index)
    }
}

impl TryFrom<String> for Item {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        let (batch, index) = name.rsplit_once('.').ok_or("not batch.index")?;
        Ok(Self { batch: batch.to_owned(), index: index.parse().map_err(|_| "not an index")? })
    }
}

impl ActorRef for Item {
    const TYPE: &'static str = "items";
    type Message = JobsMsg;
}

/// Reads an `Item` in its new form or in the object form it had before.
fn item_old_or_new<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Item, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OldOrNew {
        New(Item),
        Old { batch: String, index: i32 },
    }
    Ok(match OldOrNew::deserialize(d)? {
        OldOrNew::New(item) => item,
        OldOrNew::Old { batch, index } => Item { batch, index },
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct HolderState {
    #[serde(deserialize_with = "item_old_or_new")]
    #[schemars(with = "String")]
    item: Item,
    #[serde(default)]
    seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Holder {
    name: String,
}

impl ActorRef for Holder {
    const TYPE: &'static str = "holders";
    type Message = HolderMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum HolderMsg {
    Look,
}

struct HolderActor;

impl Actor for HolderActor {
    type Ref = Holder;
    type State = HolderState;
    type Message = HolderMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, _: HolderMsg) -> HandlerResult {
        let seen = turn.state().item.address()?.to_string();
        turn.state_mut().seen = seen;
        Ok(())
    }
}

/// The second way of the upgrade note of 0.6.0: the old form of a ref is read where the ref is held.
#[tokio::test(start_paused = true)]
async fn the_old_form_of_a_ref_is_read_with_deserialize_with_on_the_field_that_holds_it() {
    let test = TestActors::builder().with(|n| n.host(HolderActor)).build().await.unwrap();
    let holder = Holder { name: "h".into() };
    let old = json!({"item": {"batch": "b1", "index": 7}});
    assert_eq!(test.system().create_actor(&holder.address().unwrap(), old).await.unwrap(), CreateActorResult::Created);
    let look = test.send(&holder, HolderMsg::Look).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&look).await.unwrap().status, OutcomeStatus::Done);
    // The turn wrote the state back, with the ref in its new form.
    assert_eq!(test.state_json(&holder).await.unwrap(), json!({"item": "b1.7", "seen": "items/b1.7"}));
}
