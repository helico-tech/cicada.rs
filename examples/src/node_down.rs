//! Example 22 — telling on-call that a node stopped heartbeating.
//!
//! The node table marks a node dead after two minutes without a heartbeat, but nobody looks at it at night. The
//! [`NodeDownMonitorActor`](cicada_actors::NodeDownMonitorActor) reads the table every ten seconds and publishes one
//! [`NodeDown`] to a topic for each node row it sees silent for two minutes or more. [`start`] sets it up once per
//! store: it creates the topic [`alerts`] with the [`Pager`] subscribed, has the pager watch the monitor, creates the
//! monitor unpinned and sends it `Rearm`. The host reads the table through an [`ActorSystem`] that it builds on the same
//! store and never starts, so the reader writes no row of its own.
//!
//! Two kinds of trouble reach on-call in different ways. A poll that fails for good, such as one whose topic is gone,
//! is a failed turn: the pager is told through its watch and [`repair`] puts the topic back and sends `Rearm`. A read
//! of the table that fails, is slow or gives a sample the monitor refuses is not a failed turn: the monitor keeps
//! polling and records the category in `lastProblem`, which [`read_problem`] shows (see `tests/node_down.rs`).
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::{NodeDownMonitorActor, TopicActor};
//! # use cicada_examples::node_down::{PagerActor, start};
//! # async fn run(store: cicada::Store) -> Result<(), Box<dyn std::error::Error>> {
//! let reader = ActorSystem::builder(store.clone()).build(); // never started: no row, no heartbeat
//! let system = ActorSystem::builder(store)
//!     .host(NodeDownMonitorActor::new(reader))
//!     .host(TopicActor)
//!     .host(PagerActor)
//!     .build();
//! system.start().await?;
//! start(&system).await?;
//! # Ok(()) }
//! ```

use cicada::prelude::*;
use cicada::{Failed, OpError, Stopped, WatchTarget};
use cicada_actors::{
    NodeDown, NodeDownMonitor, NodeDownMonitorMsg, NodeDownMonitorState, NodeReadProblem, Topic, TopicMsg, TopicState,
};
use serde::{Deserialize, Serialize};

/// The most lines the pager keeps; older ones are dropped first.
pub const MAX_LINES: usize = 100;

/// The topic the monitor publishes to, `cicada.topic/node-down`.
pub fn alerts() -> Topic {
    Topic::named("node-down")
}

/// Sets up node-down alerts on a store: the topic with the pager subscribed, the pager's watch of the monitor, and the
/// monitor, started. Safe to repeat: a create of an actor that exists changes nothing, and `Rearm` keeps what the
/// monitor already alerted.
pub async fn start(system: &ActorSystem) -> Result<(), OpError> {
    system.create(&Pager, &PagerState::default()).await?;
    subscribe(system).await?;
    system.watch(&WatchTarget::Actor(address(&NodeDownMonitor)?), &address(&Pager)?).await?;
    let state = NodeDownMonitorState::new(alerts()).map_err(invalid)?;
    system.create(&NodeDownMonitor, &state).await?;
    system.send(&NodeDownMonitor, NodeDownMonitorMsg::Rearm).await?;
    Ok(())
}

/// Puts the topic back with the pager subscribed, and arms the monitor's timer again: the repair after the pager was
/// told that a poll failed.
pub async fn repair(system: &ActorSystem) -> Result<(), OpError> {
    subscribe(system).await?;
    system.send(&NodeDownMonitor, NodeDownMonitorMsg::Rearm).await?;
    Ok(())
}

/// Why the monitor's last read of the node table gave nothing, if it did: the part of its health that no watch shows.
pub async fn read_problem(system: &ActorSystem) -> Result<Option<NodeReadProblem>, OpError> {
    let Some(snapshot) = system.get_actor(&address(&NodeDownMonitor)?).await? else { return Ok(None) };
    let state: NodeDownMonitorState = serde_json::from_value(snapshot.state).map_err(invalid)?;
    Ok(state.last_problem)
}

async fn subscribe(system: &ActorSystem) -> Result<(), OpError> {
    system.create(&alerts(), &TopicState::default()).await?;
    system.send(&alerts(), TopicMsg::Subscribe { subscriber: address(&Pager)? }).await?;
    Ok(())
}

fn address<R: ActorRef>(r: &R) -> Result<ActorAddress, OpError> {
    r.address().map_err(invalid)
}

fn invalid(e: impl std::fmt::Display) -> OpError {
    OpError::Invalid(e.to_string())
}

/// The on-call pager, `ops.pager/_`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pager;

impl ActorRef for Pager {
    const TYPE: &'static str = "ops.pager";
    type Message = PagerMsg;
}

/// Messages of [`Pager`]: the alerts of the topic, and the notifications of its watch of the monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum PagerMsg {
    /// A node row was seen dead.
    NodeDown(NodeDown),
    /// A message of the monitor failed for good: its timer may be lost.
    Failed(Failed),
    /// The monitor was deleted.
    Stopped(Stopped),
}

/// State of [`Pager`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PagerState {
    /// The latest lines, at most [`MAX_LINES`]; a real pager would page someone.
    pub lines: Vec<String>,
}

/// The pager type: keeps one line for each alert and each notification.
pub struct PagerActor;

impl Actor for PagerActor {
    type Ref = Pager;
    type State = PagerState;

    async fn handle(&self, turn: &mut Turn<Self>, message: PagerMsg) -> HandlerResult {
        let line = match message {
            PagerMsg::NodeDown(down) => format!(
                "node {} ({}) silent since {}",
                down.node,
                down.hostname.as_deref().unwrap_or("unknown host"),
                cicada::time::format_instant(down.last_seen)
            ),
            PagerMsg::Failed(failed) => format!("monitor {} failed: {:?}", failed.message_type, failed.cause),
            PagerMsg::Stopped(_) => "monitor deleted".to_owned(),
        };
        let lines = &mut turn.state_mut().lines;
        lines.push(line);
        if lines.len() > MAX_LINES {
            lines.remove(0);
        }
        Ok(())
    }
}
