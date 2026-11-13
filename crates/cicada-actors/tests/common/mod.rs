//! Shared test types: a worker that does what its messages say, and a witness that keeps every `Failed` it is told.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::time::Duration;

use cicada::prelude::*;
use cicada::store::{Message, OutcomeStatus};
use cicada::{Failed, Stopped, TypeName, WatchTarget};
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

/// `worker/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worker {
    pub id: String,
}

impl ActorRef for Worker {
    const TYPE: &'static str = "worker";
    type Message = WorkerMsg;
}

pub fn worker(id: &str) -> Worker {
    Worker { id: id.into() }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum WorkerMsg {
    /// The start message: notes `begin`.
    Begin,
    /// Adds one.
    Work,
    /// Fails: the cause `handler`.
    Fail,
    /// Refuses for good: the cause `refused`.
    Refuse,
    /// Sleeps past its maximum turn: the cause `timeout`.
    Hang,
    /// Sends to an actor that does not exist: the cause `commit`.
    Orphan,
    /// Fails with a store error inside: the cause `node`.
    StoreDown,
    /// Writes a state that the type cannot read again: the next message fails with the cause `state`.
    Poison,
    /// Stops.
    StopSelf,
    /// A message of a published topic, noted with its text.
    Note { text: String },
    /// Fails on its first run; a run that retries it notes its text.
    FailOnce { text: String },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerState {
    #[serde(default)]
    pub n: u32,
    #[serde(default)]
    pub log: Vec<String>,
    /// Written as a text when set, which reads back as no boolean.
    #[serde(default, serialize_with = "poison", skip_serializing_if = "std::ops::Not::not")]
    pub poisoned: bool,
}

fn poison<S: serde::Serializer>(_: &bool, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str("poisoned")
}

pub const WORKER_MAX_TURN: Duration = Duration::from_secs(1);

pub struct WorkerActor;

impl Actor for WorkerActor {
    type Ref = Worker;
    type State = WorkerState;
    const MAX_TURN: Option<Duration> = Some(WORKER_MAX_TURN);

    async fn handle(&self, turn: &mut Turn<Self>, message: WorkerMsg) -> HandlerResult {
        match message {
            WorkerMsg::Begin => turn.state_mut().log.push("begin".into()),
            WorkerMsg::Work => turn.state_mut().n += 1,
            WorkerMsg::Fail => return Err(HandlerError::msg("boom")),
            WorkerMsg::Refuse => return Err(turn.fail("no")),
            WorkerMsg::Hang => tokio::time::sleep(WORKER_MAX_TURN * 10).await,
            WorkerMsg::Orphan => {
                turn.send(&worker("absent"), WorkerMsg::Work)?;
            }
            WorkerMsg::StoreDown => {
                return Err(HandlerError::new(cicada::store::StoreError::unavailable("the store is down")));
            }
            WorkerMsg::Poison => turn.state_mut().poisoned = true,
            WorkerMsg::StopSelf => turn.stop(),
            WorkerMsg::Note { text } => turn.state_mut().log.push(text),
            WorkerMsg::FailOnce { .. } if turn.message().failures == 0 => return Err(HandlerError::msg("first run")),
            WorkerMsg::FailOnce { text } => turn.state_mut().log.push(text),
        }
        Ok(())
    }
}

/// `witness/_`: watches what the test says and keeps every `Failed` it is told.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Witness;

impl ActorRef for Witness {
    const TYPE: &'static str = "witness";
    type Message = WitnessMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum WitnessMsg {
    Failed(Failed),
    Stopped(Stopped),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Told {
    pub failed: Vec<Failed>,
}

pub struct WitnessActor;

impl Actor for WitnessActor {
    type Ref = Witness;
    type State = Told;

    async fn handle(&self, turn: &mut Turn<Self>, message: WitnessMsg) -> HandlerResult {
        if let WitnessMsg::Failed(failed) = message {
            turn.state_mut().failed.push(failed);
        }
        Ok(())
    }
}

/// Creates the witness on the node of `test`, watching `target`.
pub async fn witness(test: &TestActors, target: &str) {
    test.create(&Witness, &Told::default()).await;
    test.system().watch(&WatchTarget::parse(target).unwrap(), &addr(&Witness)).await.unwrap();
}

/// Every `Failed` the witness was told.
pub async fn told(test: &TestActors) -> Vec<Failed> {
    test.state::<_, Told>(&Witness).await.failed
}

pub fn addr<R: ActorRef>(r: &R) -> ActorAddress {
    r.address().unwrap()
}

pub fn types<R: ActorRef>() -> BTreeSet<TypeName> {
    BTreeSet::from([TypeName::parse(R::TYPE).unwrap()])
}

/// The status of the outcome of `message`; `None` while it is pending.
pub async fn status(test: &TestActors, message: &Message) -> Option<OutcomeStatus> {
    test.outcome(message).await.map(|o| o.status)
}
