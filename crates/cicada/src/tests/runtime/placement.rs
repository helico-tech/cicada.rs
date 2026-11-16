//! Placement on the runtime (ADR 0052), on virtual time: creates with options from outside and inside a turn (typed,
//! raw, child, checked), the executing owner a handler reads, a node and a worker that claim only what is pinned to
//! them or to nobody, and messages that wait while no eligible executor polls.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use crate::address::{ActorAddress, TypeName};
use crate::prelude::*;
use crate::runtime::{CreateActorResult, LeaseTurnsResult, NodeConfig, OpError, Worker, WorkerConfig};
use crate::store::{Backend, Claim, NodeId, OutcomeStatus, StoreResult};
use crate::turn::TurnError;
use crate::{ActorSystem, CreateOptions};

/// type `spot`, key `{id}`: records who ran each of its turns.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Spot {
    id: String,
}

impl ActorRef for Spot {
    const TYPE: &'static str = "spot";
    type Message = SpotMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SpotMsg {
    Hello,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct SpotState {
    #[serde(default)]
    seen_by: Vec<String>,
}

struct SpotActor;

impl Actor for SpotActor {
    type Ref = Spot;
    type State = SpotState;

    async fn handle(&self, turn: &mut Turn<Self>, _: SpotMsg) -> HandlerResult {
        let me = turn.node_id().to_string();
        turn.state_mut().seen_by.push(me);
        Ok(())
    }
}

/// type `placer`, key `{id}`: creates spots with options and logs what the calls answered.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Placer {
    id: String,
}

impl ActorRef for Placer {
    const TYPE: &'static str = "placer";
    type Message = PlacerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum PlacerMsg {
    /// Creates `spot/{key}` as a child, unpinned, and `spot/{key}-here` as a child pinned to this turn's executor.
    Children { key: String },
    /// Creates `spot/{key}` by its raw address pinned to `pin`, independent or as a child.
    Raw { key: String, pin: String, child: bool },
    /// Creates `spot/{key}` pinned to `pin`, then again unpinned, and again pinned to `other`; logs the answers.
    Conflict { key: String, pin: String, other: String },
    /// Creates `spot/{key}` with a pin beyond its bounds; logs the answer.
    BadPin { key: String },
    /// Creates `spot/{key}` pinned to `pin`, then fails for good.
    CreateThenFail { key: String, pin: String },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct PlacerState {
    #[serde(default)]
    log: Vec<String>,
}

fn pin(id: &str) -> CreateOptions {
    CreateOptions::new().pinned_to(NodeId::new(id).unwrap())
}

fn answer(result: Result<(), TurnError>) -> String {
    match result {
        Ok(()) => "ok".to_owned(),
        Err(TurnError::Invalid(_)) => "invalid".to_owned(),
        Err(e) => format!("other: {e}"),
    }
}

struct PlacerActor;

impl Actor for PlacerActor {
    type Ref = Placer;
    type State = PlacerState;

    async fn handle(&self, turn: &mut Turn<Self>, message: PlacerMsg) -> HandlerResult {
        match message {
            PlacerMsg::Children { key } => {
                turn.create_child(&Spot { id: key.clone() }, &SpotState::default())?;
                let here = CreateOptions::new().pinned_to(turn.node_id().clone());
                turn.create_child_with(&Spot { id: format!("{key}-here") }, &SpotState::default(), here)?;
            }
            PlacerMsg::Raw { key, pin: to, child } => {
                let address = ActorAddress::parse(&format!("spot/{key}"))?;
                if child {
                    turn.create_child_raw_with(&address, json!({}), pin(&to)).await?;
                } else {
                    turn.create_raw_with(&address, json!({}), pin(&to)).await?;
                }
            }
            PlacerMsg::Conflict { key, pin: first, other } => {
                let spot = Spot { id: key };
                let answers = [
                    answer(turn.create_with(&spot, &SpotState::default(), pin(&first))),
                    answer(turn.create(&spot, &SpotState::default())),
                    answer(turn.create_with(&spot, &SpotState::default(), pin(&other))),
                    answer(turn.create_with(&spot, &SpotState::default(), pin(&first))),
                ];
                turn.state_mut().log.push(answers.join(","));
            }
            PlacerMsg::BadPin { key } => {
                let spot = Spot { id: key };
                let long = answer(turn.create_with(&spot, &SpotState::default(), pin(&"x".repeat(201))));
                let nul = answer(turn.create_with(&spot, &SpotState::default(), pin("a\0b")));
                let address = ActorAddress::parse("spot/raw-bad")?;
                let raw = match turn.create_raw_with(&address, json!({}), pin("a\0b")).await {
                    Err(TurnError::Invalid(_)) => "invalid".to_owned(),
                    other => format!("{other:?}"),
                };
                turn.state_mut().log.push(format!("{long},{nul},{raw}"));
            }
            PlacerMsg::CreateThenFail { key, pin: to } => {
                turn.create_with(&Spot { id: key }, &SpotState::default(), pin(&to))?;
                return Err(HandlerError::permanent("declined"));
            }
        }
        Ok(())
    }
}

fn spot(id: &str) -> Spot {
    Spot { id: id.to_owned() }
}

async fn pin_of(system: &ActorSystem, r: &impl ActorRef) -> Option<NodeId> {
    system.get_actor(&r.address().unwrap()).await.unwrap().expect("exists").pinned_to
}

async fn seen_by(system: &ActorSystem, id: &str) -> Vec<String> {
    let snapshot = system.get_actor(&spot(id).address().unwrap()).await.unwrap().expect("the spot");
    serde_json::from_value::<SpotState>(snapshot.state).unwrap().seen_by
}

fn hosting(store: &crate::store::Store, name: &str) -> ActorSystem {
    node(store.clone(), name).host(SpotActor).host(PlacerActor).build()
}

/// Scenarios: Pin a future executor; Target is unavailable; Identify the executing owner.
#[tokio::test(start_paused = true)]
async fn only_the_node_a_spot_is_pinned_to_runs_it_and_the_others_do_not_wait_for_it() {
    let store = virtual_store();
    let (a, b) = (hosting(&store, "node-a"), hosting(&store, "node-b"));
    a.initialize().await.unwrap();
    b.initialize().await.unwrap();
    let created = a.create_with(&spot("s"), &SpotState::default(), pin("node-b")).await.unwrap();
    assert_eq!(created, CreateActorResult::Created);
    assert_eq!(pin_of(&a, &spot("s")).await, Some(NodeId::new("node-b").unwrap()));
    a.create(&spot("free"), &SpotState::default()).await.unwrap();
    let m = accepted(a.send(&spot("s"), SpotMsg::Hello).await.unwrap());

    assert_eq!(a.run_due_turns(10).await.unwrap(), 0, "node A skips the spot pinned to node B");
    assert_eq!(a.next_due_at().await.unwrap(), None, "and does not count it in its next due time");
    assert!(b.next_due_at().await.unwrap().is_some());
    tokio::time::advance(Duration::from_secs(3600)).await;
    assert_eq!(a.run_due_turns(10).await.unwrap(), 0, "it waits as long as node B does not come");
    let waiting = a.pending(&spot("s").address().unwrap(), None, 10).await.unwrap().unwrap().items;
    assert_eq!(waiting.iter().map(|w| (w.id.clone(), w.attempt)).collect::<Vec<_>>(), [(m.id.clone(), 0)]);

    accepted(a.send(&spot("free"), SpotMsg::Hello).await.unwrap());
    assert_eq!(a.run_due_turns(10).await.unwrap(), 1, "an unpinned spot runs anywhere");
    assert_eq!(seen_by(&a, "free").await, ["node-a"]);
    assert_eq!(b.run_due_turns(10).await.unwrap(), 1);
    assert_eq!(seen_by(&a, "s").await, ["node-b"], "the handler reads the id of the node that runs it");
    assert_eq!(a.outcome(&m.id).await.unwrap().map(|o| o.status), Some(OutcomeStatus::Done));
}

/// Counts the claims a node asks for.
struct CountClaims(Arc<AtomicUsize>);

#[async_trait]
impl Faults for CountClaims {
    async fn claim_due(
        &self,
        inner: &dyn Backend,
        types: &BTreeSet<TypeName>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        inner.claim_due(types, node, ttl, limit).await
    }
}

/// A started node whose only due message is pinned elsewhere sleeps for its poll interval instead of spinning, and the
/// owner's node runs it once it starts.
#[tokio::test(start_paused = true)]
async fn a_started_node_does_not_spin_on_a_spot_pinned_elsewhere() {
    let store = virtual_store();
    let claims = Arc::new(AtomicUsize::new(0));
    let counted = Faulty::new(store.clone(), CountClaims(claims.clone()));
    let poll = Duration::from_secs(1);
    let config = NodeConfig { poll_interval: poll, ..node_config("node-a") };
    let a = ActorSystem::builder(counted.into_store()).config(config).host(SpotActor).host(PlacerActor).build();
    a.start().await.unwrap();
    a.create_with(&spot("s"), &SpotState::default(), pin("node-b")).await.unwrap();
    accepted(a.send(&spot("s"), SpotMsg::Hello).await.unwrap());
    settle().await;
    let before = claims.load(Ordering::SeqCst);
    // Sleeping, not advancing: the paused clock moves to each timer in turn, so a looping node would claim each time.
    tokio::time::sleep(60 * poll).await;
    let polled = claims.load(Ordering::SeqCst) - before;
    assert!(polled <= 62, "about one claim per poll interval, not a busy loop: {polled} in 60 intervals");
    let waiting = a.pending(&spot("s").address().unwrap(), None, 10).await.unwrap().unwrap().items;
    assert_eq!(waiting[0].attempt, 0, "node A never claimed it");
    let b = hosting(&store, "node-b");
    b.start().await.unwrap();
    eventually(Duration::from_secs(30), async || seen_by(&a, "s").await == ["node-b"]).await;
    a.stop().await;
    b.stop().await;
}

/// Scenarios: Reject inconsistent buffered creates; Preserve idempotent creation; children do not inherit.
#[tokio::test(start_paused = true)]
async fn a_turn_creates_with_options_and_conflicting_or_invalid_pins_fail_at_the_call() {
    let store = virtual_store();
    let a = hosting(&store, "node-a");
    a.initialize().await.unwrap();
    let placer = Placer { id: "p".into() };
    a.create_with(&placer, &PlacerState::default(), pin("node-b")).await.unwrap();
    let run = async |msg: PlacerMsg| {
        let m = accepted(a.send(&placer, msg).await.unwrap());
        // The placer is pinned to node B; node A must not run it.
        assert_eq!(a.run_due_turns(10).await.unwrap(), 0);
        let b = hosting(&store, "node-b");
        b.initialize().await.unwrap();
        b.run_due_turns(10).await.unwrap();
        a.outcome(&m.id).await.unwrap().map(|o| o.status)
    };

    assert_eq!(run(PlacerMsg::Children { key: "c".into() }).await, Some(OutcomeStatus::Done));
    assert_eq!(pin_of(&a, &spot("c")).await, None, "a child does not inherit its parent's pin");
    assert_eq!(pin_of(&a, &spot("c-here")).await, Some(NodeId::new("node-b").unwrap()), "pinned to the executor");
    let child = a.get_actor(&spot("c-here").address().unwrap()).await.unwrap().unwrap();
    assert_eq!(child.parent, Some(placer.address().unwrap()), "pin and parent are independent");

    let raw = PlacerMsg::Raw { key: "r".into(), pin: "node-z".into(), child: false };
    assert_eq!(run(raw).await, Some(OutcomeStatus::Done));
    assert_eq!(pin_of(&a, &spot("r")).await, Some(NodeId::new("node-z").unwrap()));
    let raw_child = PlacerMsg::Raw { key: "rc".into(), pin: "node-y".into(), child: true };
    assert_eq!(run(raw_child).await, Some(OutcomeStatus::Done));
    assert_eq!(pin_of(&a, &spot("rc")).await, Some(NodeId::new("node-y").unwrap()));

    let conflict = PlacerMsg::Conflict { key: "x".into(), pin: "node-a".into(), other: "node-c".into() };
    assert_eq!(run(conflict).await, Some(OutcomeStatus::Done));
    let state = a.get_actor(&placer.address().unwrap()).await.unwrap().unwrap().state;
    let log = serde_json::from_value::<PlacerState>(state).unwrap().log;
    assert_eq!(log.last().unwrap(), "ok,invalid,invalid,ok", "None against Some and another pin are refused");
    assert_eq!(pin_of(&a, &spot("x")).await, Some(NodeId::new("node-a").unwrap()), "the agreeing creates landed");

    assert_eq!(run(PlacerMsg::BadPin { key: "bad".into() }).await, Some(OutcomeStatus::Done));
    let state = a.get_actor(&placer.address().unwrap()).await.unwrap().unwrap().state;
    let log = serde_json::from_value::<PlacerState>(state).unwrap().log;
    assert_eq!(log.last().unwrap(), "invalid,invalid,invalid");
    assert!(a.get_actor(&spot("bad").address().unwrap()).await.unwrap().is_none());

    let fail = PlacerMsg::CreateThenFail { key: "gone".into(), pin: "node-a".into() };
    assert_eq!(run(fail).await, Some(OutcomeStatus::Failed));
    assert!(a.get_actor(&spot("gone").address().unwrap()).await.unwrap().is_none(), "a failed turn applies no pin");
}

/// Options from outside a turn: a pin beyond its bounds is invalid before anything is created, on every create form;
/// an existing actor keeps its pin; deleting it lets a new life choose another.
#[tokio::test(start_paused = true)]
async fn creates_from_outside_check_the_pin_first_and_keep_an_existing_one() {
    let store = virtual_store();
    let a = hosting(&store, "node-a");
    let long = pin(&"x".repeat(201));
    // Checked before the node initializes or reads the store.
    let refused = a.create_with(&spot("s"), &SpotState::default(), long.clone()).await;
    assert!(matches!(refused, Err(OpError::Invalid(_))), "{refused:?}");
    let address = spot("s").address().unwrap();
    let refused = a.create_actor_with(&address, json!({}), pin("a\0b")).await;
    assert!(matches!(refused, Err(OpError::Invalid(_))), "{refused:?}");
    let refused = a.create_actor_checked_with(&address, json!({}), long, &|_, _| panic!("not reached")).await;
    assert!(matches!(refused, Err(OpError::Invalid(_))), "{refused:?}");
    assert!(a.get_actor(&address).await.unwrap().is_none());

    let checked = a.create_actor_checked_with(&address, json!({}), pin("node-b"), &|_, _| true).await.unwrap();
    assert_eq!(checked, CreateActorResult::Created);
    let other = spot("t").address().unwrap();
    let refused = a.create_actor_checked_with(&other, json!({}), pin("node-b"), &|_, _| false).await.unwrap();
    assert!(matches!(refused, CreateActorResult::InvalidState(_)), "the schema check still applies");
    let again = a.create_actor_with(&address, json!({"seen_by": ["x"]}), pin("node-c")).await.unwrap();
    let CreateActorResult::Exists(existing) = again else { panic!("{again:?}") };
    assert_eq!(existing.pinned_to, Some(NodeId::new("node-b").unwrap()), "the existing actor keeps its pin");
    assert_eq!(a.create(&spot("s"), &SpotState::default()).await.unwrap(), CreateActorResult::Exists(existing));

    a.delete_actor(&address).await.unwrap();
    assert_eq!(a.create_actor_with(&address, json!({}), pin("node-c")).await.unwrap(), CreateActorResult::Created);
    assert_eq!(pin_of(&a, &spot("s")).await, Some(NodeId::new("node-c").unwrap()), "a new life, a new pin");
}

/// Scenario: Local and remote placement share semantics: a worker leasing under its own id through a server claims
/// what is pinned to it and what is unpinned, not what is pinned to the server, needs no node row, and its handlers
/// read the worker's id.
#[tokio::test(start_paused = true)]
async fn a_worker_claims_under_its_own_id_not_the_servers() {
    let store = virtual_store();
    let server = node(store.clone(), "server-a").build();
    server.register_contract(&crate::actor::contract_of::<SpotActor>().unwrap()).await.unwrap();
    server.create_with(&spot("mine"), &SpotState::default(), pin("worker-b")).await.unwrap();
    server.create_with(&spot("servers"), &SpotState::default(), pin("server-a")).await.unwrap();
    server.create(&spot("free"), &SpotState::default()).await.unwrap();
    for id in ["mine", "servers", "free"] {
        accepted(server.send(&spot(id), SpotMsg::Hello).await.unwrap());
    }
    let config = WorkerConfig { worker_id: NodeId::new("worker-b").unwrap(), ..WorkerConfig::default() };
    let worker = Worker::builder(server.clone()).host(SpotActor).config(config).build();
    assert_eq!(worker.run_due_turns(10).await.unwrap(), 2, "the spot pinned to the server waits");
    assert_eq!(seen_by(&server, "mine").await, ["worker-b"], "the handler reads the worker's id");
    assert_eq!(seen_by(&server, "free").await, ["worker-b"]);
    assert!(seen_by(&server, "servers").await.is_empty());
    assert!(server.nodes().await.unwrap().iter().all(|n| n.id.as_str() != "worker-b"), "no node row for the worker");

    // A lease under the server's own id is eligible for the spot pinned to it.
    let hash = server.types().await.unwrap()[0].definition.contract_hash.clone();
    let types = [(crate::address::TypeName::parse("spot").unwrap(), hash)].into();
    let leased = server.lease_turns(&NodeId::new("server-a").unwrap(), &types, 10).await.unwrap();
    let LeaseTurnsResult::Leased(leases) = leased else { panic!("{leased:?}") };
    assert_eq!(leases.iter().map(|l| l.actor.to_string()).collect::<Vec<_>>(), ["spot/servers"]);
}
