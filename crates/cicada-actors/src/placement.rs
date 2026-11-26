//! The placement pool (ADR 0067): an actor to which nodes offer places, each with a set of labels, renewed at an
//! interval; a node not heard from for the pool's silence time is marked silent, and its alert target is told once.
//! Work asks it for a place with its needs and gets a node to pin a child to (ADR 0068).

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::limits::MAX_TYPE_LENGTH;
use cicada::time::{self, MAX_INSTANT, Timestamp, format_duration, serde_duration};
use cicada::turn::TurnError;
use cicada::{Failed, Stopped};
use serde::{Deserialize, Serialize};

use crate::supervisor::retry_policy;

/// The shortest silence time of a pool.
const MIN_SILENCE: Duration = Duration::from_secs(1);
/// The longest silence time of a pool: one day.
const MAX_SILENCE: Duration = Duration::from_secs(24 * 3600);
/// The most nodes a pool can know.
const MAX_NODES: u32 = 1_000;
/// The longest node id and instance id, in characters.
const MAX_ID_LENGTH: usize = 200;
/// The most labels of one offer.
const MAX_LABELS: usize = 32;
/// The longest label, in characters.
const MAX_LABEL_LENGTH: usize = 100;
/// The most places one node offers.
const MAX_PLACES: u32 = 1_000;
/// The most waiting asks and the most holders of a pool.
const MAX_BOOKS: u32 = 1_000;
/// The longest holder id, in characters.
const MAX_HOLDER_LENGTH: usize = 200;

/// A placement pool, `cicada.placement/<pool>`: one for each set of nodes that offer places.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Placement {
    /// The name of the pool: the key of its address.
    pub pool: String,
}

impl Placement {
    /// The pool named `pool`.
    pub fn named(pool: impl Into<String>) -> Self {
        Self { pool: pool.into() }
    }
}

impl ActorRef for Placement {
    const TYPE: &'static str = "cicada.placement";
    type Message = PlacementMsg;
}

/// The messages of a [`Placement`] pool. `Check`, `Alert` and `Notify` are its own: one from anyone else is refused for
/// good. A node id or instance id is 1 through 200 characters, not blank, without NUL; a holder id is 1 through 200
/// characters without NUL; a message out of bounds is refused for good with the state unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum PlacementMsg {
    /// A node offers its places: replaces the node's entry with this instance, these labels (sorted, each once) and
    /// places, heard at the store time of this turn, and live. A silent node that offers is told `back` to the alert
    /// target. A node new to a pool that knows `maxNodes` nodes is refused for good. Send one at an interval shorter
    /// than the silence time.
    Offer {
        /// The node id the member's node pins its actors under.
        #[schemars(length(min = 1, max = 200))]
        node: String,
        /// An id the member draws at each process start, which tells two processes offering under one node id apart.
        #[schemars(length(min = 1, max = 200))]
        instance: String,
        /// What the node offers, such as its processor kind: at most 32, each 1 through 100 characters without NUL.
        #[schemars(length(max = 32), inner(length(min = 1, max = 100)))]
        labels: Vec<String>,
        /// How many places the node offers, 0 through 1,000; 0 keeps the node known but full.
        #[schemars(range(max = 1000))]
        places: u32,
    },
    /// The silence timer: sent by the pool to itself, and counted only when it is the armed one (`armedId`). Marks each
    /// live node not heard from for the silence time as silent, queues an `Alert` for each, and arms the next check at
    /// the earliest end of a live node's silence. Run early (made due by an operator) it arms itself again at `armed`;
    /// any other `Check` the pool sent itself changes nothing, and one from another sender is refused for good.
    Check,
    /// Cancels the armed check and arms a new one at its unchanged time, at once when that has passed: the recovery of
    /// a check that was purged, cancelled or failed for good. Nothing when no node is live.
    Rearm,
    /// Sends the alert target a notice about `node`, in a turn of its own so that a failing target fails only this
    /// alert; nothing without a target. Sent by the pool to itself.
    Alert {
        /// The node it is about.
        #[schemars(length(min = 1, max = 200))]
        node: String,
        /// What happened to it.
        kind: PlacementNoticeKind,
    },
    /// Sets where the pool's notices go, replacing the target before.
    Alerts {
        /// The target of the notices.
        to: ActorAddress,
        /// The message type of the notices, whose payload is a [`PlacementNotice`]; not empty, at most 200 characters,
        /// without NUL.
        #[schemars(length(min = 1, max = 200))]
        message_type: String,
    },
    /// Asks a place for `holder` on a node with every one of `needs` (sorted, each once), and names where its answer
    /// goes. It is granted on the live node with a free place and every need that has the most free places, ties to
    /// the smaller node id; queued when only silent or full nodes have the needs, or the pool holds `maxHolders`;
    /// refused with `noNode` when no node of the pool has them, `lineFull` when it must wait and the line holds
    /// `maxWaiting` asks, and `poolFull` when a place fits but the pool holds `maxHolders` and the line is full. The
    /// [`PlacementAnswer`] goes to the target in this commit, so a target that is gone fails the ask and nothing is
    /// recorded. A repeated `Ask` of a holder answers `granted` with the same grant and node, one of a waiting ask
    /// `queued`; both keep the first ask's needs and target. A holder id is forgotten at its release: use a fresh one
    /// per attempt. The pool watches the target from the commit that records the ask, and when it is deleted frees its
    /// places and takes its waiting asks out of the line; an ask whose target is the pool itself is refused for good.
    Ask {
        /// The holder: names this attempt, unique among the holders and waiting asks of the pool.
        #[schemars(length(min = 1, max = 200))]
        holder: String,
        /// The labels a node must have: at most 32, each 1 through 100 characters without NUL.
        #[schemars(length(max = 32), inner(length(min = 1, max = 100)))]
        needs: Vec<String>,
        /// The target of the answer.
        to: ActorAddress,
        /// The message type of the answer, whose payload is a [`PlacementAnswer`]; not empty, at most 200 characters,
        /// without NUL.
        #[schemars(length(min = 1, max = 200))]
        message_type: String,
    },
    /// Frees the place of `holder` when it holds `grant`; ignored otherwise, so a late release of an older grant frees
    /// nothing.
    Release {
        /// The holder.
        #[schemars(length(min = 1, max = 200))]
        holder: String,
        /// The grant number it releases.
        grant: u64,
    },
    /// Takes a waiting ask out of the line; ignored for a holder, which releases with its grant number instead.
    Cancel {
        /// The holder of the waiting ask.
        #[schemars(length(min = 1, max = 200))]
        holder: String,
    },
    /// Sends `holder` its `granted` answer when it still holds a place, and marks it as told; nothing otherwise. Sent
    /// by the pool to itself for a grant from the line, in a turn of its own, with a retry policy.
    Notify {
        /// The holder.
        #[schemars(length(min = 1, max = 200))]
        holder: String,
    },
    /// A message of the pool itself ended `failed`: a `Notify` that failed for a holder never told of its grant frees
    /// that place, as the holder never learned it. A failure of a watched target's message changes nothing. Only a
    /// notification that the store wrote counts.
    Failed(Failed),
    /// A watched target went away: every place held for it is freed and its waiting asks leave the line, once. One for
    /// the pool itself or for a target that holds and waits for nothing changes nothing. Only a notification that the
    /// store wrote counts.
    Stopped(Stopped),
}

impl PlacementMsg {
    /// An `Offer` of `places` places with `labels` from `instance` of `node`; the labels are sorted and duplicates
    /// removed. Fails for a node or instance id that is empty, blank, longer than 200 characters or holds a NUL, more
    /// than 32 labels, a label that is empty, longer than 100 characters or holds a NUL, and more than 1,000 places.
    ///
    /// ```
    /// use cicada_actors::PlacementMsg;
    ///
    /// let offer = PlacementMsg::offer("factory-mac-1", "3f9c", ["docker", "arm64", "docker"], 4)?;
    /// assert_eq!(serde_json::to_value(&offer)?, serde_json::json!({"type": "Offer",
    ///     "node": "factory-mac-1", "instance": "3f9c", "labels": ["arm64", "docker"], "places": 4}));
    /// assert!(PlacementMsg::offer("factory-mac-1", "3f9c", ["arm64"], 1001).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn offer<L: Into<String>>(
        node: impl Into<String>,
        instance: impl Into<String>,
        labels: impl IntoIterator<Item = L>,
        places: u32,
    ) -> Result<Self, InvalidPlacement> {
        let (node, instance) = (node.into(), instance.into());
        let labels = labels.into_iter().map(Into::into).collect();
        let labels = check_offer(&node, &instance, labels, places)?;
        Ok(PlacementMsg::Offer { node, instance, labels, places })
    }

    /// An `Alerts` that sends the pool's notices to `target` as `message_type`. Fails for a ref without an address and
    /// a message type that is empty, longer than `MAX_TYPE_LENGTH` characters or holds a NUL. Whether the target's type
    /// takes the message type is checked at each notice.
    pub fn alerts<R: ActorRef>(target: &R, message_type: impl Into<String>) -> Result<Self, InvalidPlacement> {
        let to = target.address().map_err(|e| InvalidPlacement(e.to_string()))?;
        let message_type = message_type.into();
        check_message_type(&message_type)?;
        Ok(PlacementMsg::Alerts { to, message_type })
    }

    /// An `Ask` of a place for `holder` on a node with every one of `needs`, whose answer goes to `target` as
    /// `message_type`; the needs are sorted and duplicates removed. Fails for a ref without an address, a holder id
    /// that is empty, longer than 200 characters or holds a NUL, more than 32 needs, a need that is empty, longer than
    /// 100 characters or holds a NUL, and a message type that is empty, longer than `MAX_TYPE_LENGTH` characters or
    /// holds a NUL. Whether the target's type takes the message type is checked at the answer.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Run { id: String }
    /// # impl ActorRef for Run { const TYPE: &'static str = "run"; type Message = RunMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum RunMsg { Placed(cicada_actors::PlacementAnswer) }
    /// use cicada_actors::PlacementMsg;
    ///
    /// let ask = PlacementMsg::ask("run-17", ["docker", "arm64"], &Run { id: "17".into() }, "Placed")?;
    /// assert_eq!(serde_json::to_value(&ask)?, serde_json::json!({"type": "Ask",
    ///     "holder": "run-17", "needs": ["arm64", "docker"], "to": "run/17", "messageType": "Placed"}));
    /// assert!(PlacementMsg::ask("", ["arm64"], &Run { id: "17".into() }, "Placed").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn ask<R: ActorRef, L: Into<String>>(
        holder: impl Into<String>,
        needs: impl IntoIterator<Item = L>,
        target: &R,
        message_type: impl Into<String>,
    ) -> Result<Self, InvalidPlacement> {
        let to = target.address().map_err(|e| InvalidPlacement(e.to_string()))?;
        let (holder, message_type) = (holder.into(), message_type.into());
        let needs = check_ask(&holder, needs.into_iter().map(Into::into).collect(), &message_type)?;
        Ok(PlacementMsg::Ask { holder, needs, to, message_type })
    }

    /// A `Release` of the place `holder` holds under `grant`. Fails for a holder id out of bounds.
    pub fn release(holder: impl Into<String>, grant: u64) -> Result<Self, InvalidPlacement> {
        let holder = holder.into();
        check_holder(&holder)?;
        Ok(PlacementMsg::Release { holder, grant })
    }

    /// A `Cancel` of the waiting ask of `holder`. Fails for a holder id out of bounds.
    pub fn cancel(holder: impl Into<String>) -> Result<Self, InvalidPlacement> {
        let holder = holder.into();
        check_holder(&holder)?;
        Ok(PlacementMsg::Cancel { holder })
    }
}

/// What an answer of a pool to an ask says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PlacementAnswerKind {
    /// The holder holds a place, with its grant number and node.
    Granted,
    /// The ask waits in the line.
    Queued,
    /// The ask was refused, with a reason: nothing was recorded.
    Refused,
}

/// Why a pool refused an ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PlacementRefusal {
    /// No node of the pool, live or silent, has every need.
    NoNode,
    /// The ask must wait for a place and the line holds `maxWaiting` asks.
    LineFull,
    /// A place fits, but the pool holds `maxHolders` holders and the line holds `maxWaiting` asks.
    PoolFull,
}

/// An answer of a pool to an ask, sent to its target as the message type of its `Ask`. A host's family takes it as a
/// variant such as `Placed(PlacementAnswer)`.
///
/// On the wire: `{"pool":"runners","holder":"run-17","kind":"granted","grant":4,"node":"factory-mac-1"}`; `grant` and
/// `node` are written only for `granted`, `reason` only for `refused`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementAnswer {
    /// The pool: the key of its address.
    pub pool: String,
    /// The holder it is about.
    pub holder: String,
    /// What it says.
    pub kind: PlacementAnswerKind,
    /// The grant number, set for `granted`: name it in the `Release`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<u64>,
    /// The node of the place, set for `granted`: pin the work's actors to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Why, set for `refused`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PlacementRefusal>,
}

/// A holder of a place on a node of a pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementHolder {
    /// The holder; 1 through 200 characters without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub holder: String,
    /// The needs of its ask, sorted, each once; at most 32, each 1 through 100 characters without NUL.
    #[schemars(length(max = 32), inner(length(min = 1, max = 100)), extend("uniqueItems" = true))]
    pub needs: Vec<String>,
    /// The target of its answers.
    pub to: ActorAddress,
    /// The message type of its answers; not empty, at most 200 characters, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub message_type: String,
    /// Its grant number: at least 1, below the pool's `nextGrant`.
    #[schemars(range(min = 1))]
    pub grant: u64,
    /// The node id of its place: a node the pool knows.
    #[schemars(length(min = 1, max = 200))]
    pub node: String,
    /// True once a `granted` answer to it committed; while false, a failed `Notify` frees the place.
    pub notified: bool,
}

/// An ask waiting in the line of a pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementAsk {
    /// The holder; 1 through 200 characters without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub holder: String,
    /// The labels a node must have, sorted, each once; at most 32, each 1 through 100 characters without NUL.
    #[schemars(length(max = 32), inner(length(min = 1, max = 100)), extend("uniqueItems" = true))]
    pub needs: Vec<String>,
    /// The target of its answers.
    pub to: ActorAddress,
    /// The message type of its answers; not empty, at most 200 characters, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub message_type: String,
}

/// What a notice of a pool says about a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PlacementNoticeKind {
    /// The node was not heard from for the silence time: it gets no new places. Sent once per silence.
    Silent,
    /// A silent node offered again and is live.
    Back,
}

/// A notice of a pool to its alert target, sent as the message type of its `Alerts`. A host's family takes it as a
/// variant such as `Placement(PlacementNotice)`.
///
/// On the wire: `{"pool":"runners","node":"factory-mac-1","kind":"silent"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementNotice {
    /// The pool: the key of its address.
    pub pool: String,
    /// The node it is about.
    pub node: String,
    /// What happened to it.
    pub kind: PlacementNoticeKind,
}

/// Where the notices of a pool go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementAlerts {
    /// The target of the notices.
    pub to: ActorAddress,
    /// The message type of the notices; not empty, at most 200 characters, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub message_type: String,
}

/// Whether a node of a pool is heard from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PlacementNodeStatus {
    /// Heard from within the silence time.
    Live,
    /// Not heard from for the silence time: it gets no new places until it offers again.
    Silent,
}

/// What a pool knows of a node: its last offer and whether it is heard from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementNode {
    /// The instance id of its last offer; 1 through 200 characters, not blank, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub instance: String,
    /// The labels of its last offer, sorted, each once; at most 32, each 1 through 100 characters without NUL.
    #[schemars(length(max = 32), inner(length(min = 1, max = 100)), extend("uniqueItems" = true))]
    pub labels: Vec<String>,
    /// The places of its last offer, 0 through 1,000.
    #[schemars(range(max = 1000))]
    pub places: u32,
    /// The store time of the turn of its last offer.
    pub heard_at: Timestamp,
    /// Whether it is heard from.
    pub status: PlacementNodeStatus,
}

/// A state or a message of a pool out of its bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidPlacement(String);

/// The state of a [`Placement`] pool: its silence time, node limit, line limit and holder limit, fixed at its create,
/// where its notices go, the nodes it knows by node id, the next grant number, its holders in the order they were
/// granted, its waiting asks in line order, and the time and id of its armed check. Build it with
/// `PlacementState::new`; a decoded state is checked the same way.
///
/// On the wire: `{"silence":"PT1M","maxNodes":1000,"maxWaiting":100,"maxHolders":1000,"alerts":null,"nodes":{},
/// "nextGrant":1,"holders":[],"waiting":[],"armed":null,"armedId":null}`; `alerts`, `nodes`, `nextGrant`, `holders`,
/// `waiting`, `armed` and `armedId` may be left out of a create.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct PlacementState {
    /// How long a node may go without an offer before it is silent.
    #[serde(serialize_with = "serde_duration::serialize")]
    pub silence: Duration,
    /// The most nodes the pool knows.
    pub max_nodes: u32,
    /// The most asks that wait in the line.
    pub max_waiting: u32,
    /// The most holders of the pool at once; a pool at this limit queues new asks.
    pub max_holders: u32,
    /// Where its notices go; none: no notices.
    pub alerts: Option<PlacementAlerts>,
    /// The nodes it knows, by node id.
    pub nodes: BTreeMap<String, PlacementNode>,
    /// The number of the next grant; it only goes up.
    pub next_grant: u64,
    /// The holders, in the order they were granted.
    pub holders: Vec<PlacementHolder>,
    /// The waiting asks, in line order.
    pub waiting: Vec<PlacementAsk>,
    /// The time of the armed `Check`, set exactly while a node is live.
    pub armed: Option<Timestamp>,
    /// The id of the armed `Check`, set exactly with `armed`. It stays set when the check goes without running.
    pub armed_id: Option<MessageId>,
}

/// The wire form of a [`PlacementState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RawState {
    /// How long a node may go without an offer before it is silent, as an ISO-8601 duration; 1 second through 1 day.
    #[serde(deserialize_with = "serde_duration::deserialize")]
    #[schemars(with = "String")]
    silence: Duration,
    /// The most nodes the pool knows; 1 through 1000.
    #[schemars(range(min = 1, max = 1000))]
    max_nodes: u32,
    /// The most asks that wait in the line; 0 through 1000.
    #[schemars(range(max = 1000))]
    max_waiting: u32,
    /// The most holders of the pool at once; 1 through 1000.
    #[schemars(range(min = 1, max = 1000))]
    max_holders: u32,
    /// Where its notices go; absent or null: no notices.
    #[serde(default)]
    alerts: Option<PlacementAlerts>,
    /// The nodes it knows, by node id (1 through 200 characters, not blank, without NUL), at most `maxNodes`; absent:
    /// none.
    #[serde(default)]
    nodes: BTreeMap<String, PlacementNode>,
    /// The number of the next grant, at least 1; it only goes up. Absent: 1.
    #[serde(default = "first_grant")]
    #[schemars(range(min = 1))]
    next_grant: u64,
    /// The holders, in the order they were granted, at most `maxHolders`, each on a known node, each holder id once
    /// among the holders and waiting asks; absent: none.
    #[serde(default)]
    holders: Vec<PlacementHolder>,
    /// The waiting asks, in line order, at most `maxWaiting`, each holder id once among the holders and waiting asks;
    /// absent: none.
    #[serde(default)]
    waiting: Vec<PlacementAsk>,
    /// The time of the armed `Check`: present exactly while a node is live.
    #[serde(default)]
    armed: Option<Timestamp>,
    /// The id of the armed `Check`: present exactly with `armed`.
    #[serde(default)]
    armed_id: Option<MessageId>,
}

fn first_grant() -> u64 {
    1
}

impl TryFrom<RawState> for PlacementState {
    type Error = InvalidPlacement;

    fn try_from(raw: RawState) -> Result<Self, InvalidPlacement> {
        let state = Self {
            silence: raw.silence,
            max_nodes: raw.max_nodes,
            max_waiting: raw.max_waiting,
            max_holders: raw.max_holders,
            alerts: raw.alerts,
            nodes: raw.nodes,
            next_grant: raw.next_grant,
            holders: raw.holders,
            waiting: raw.waiting,
            armed: raw.armed,
            armed_id: raw.armed_id,
        };
        state.check()?;
        Ok(state)
    }
}

impl PlacementState {
    /// A pool that marks a node silent after `silence` without an offer, knows at most `max_nodes` nodes, lets at most
    /// `max_waiting` asks wait and at most `max_holders` hold a place, with no alert target, no nodes, no holders, no
    /// waiting asks and next grant number 1. Fails unless the silence is 1 second through 1 day, the node limit and
    /// the holder limit 1 through 1,000, and the line limit 0 through 1,000.
    ///
    /// ```
    /// use std::time::Duration;
    /// use cicada_actors::PlacementState;
    ///
    /// let state = PlacementState::new(Duration::from_secs(60), 1000, 100, 1000)?;
    /// assert_eq!(serde_json::to_value(&state)?, serde_json::json!({
    ///     "silence": "PT1M", "maxNodes": 1000, "maxWaiting": 100, "maxHolders": 1000, "alerts": null, "nodes": {},
    ///     "nextGrant": 1, "holders": [], "waiting": [], "armed": null, "armedId": null}));
    /// assert!(PlacementState::new(Duration::from_secs(60), 1001, 100, 1000).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(
        silence: Duration,
        max_nodes: u32,
        max_waiting: u32,
        max_holders: u32,
    ) -> Result<Self, InvalidPlacement> {
        let state = Self {
            silence,
            max_nodes,
            max_waiting,
            max_holders,
            alerts: None,
            nodes: BTreeMap::new(),
            next_grant: 1,
            holders: Vec::new(),
            waiting: Vec::new(),
            armed: None,
            armed_id: None,
        };
        state.check()?;
        Ok(state)
    }

    fn check(&self) -> Result<(), InvalidPlacement> {
        if self.silence < MIN_SILENCE || self.silence > MAX_SILENCE {
            return Err(InvalidPlacement(format!(
                "silence must be {} through {}, was {}",
                format_duration(MIN_SILENCE),
                format_duration(MAX_SILENCE),
                format_duration(self.silence)
            )));
        }
        if self.max_nodes == 0 || self.max_nodes > MAX_NODES {
            return Err(InvalidPlacement(format!("maxNodes must be 1 through {MAX_NODES}, was {}", self.max_nodes)));
        }
        if self.max_waiting > MAX_BOOKS {
            return Err(InvalidPlacement(format!(
                "maxWaiting must be 0 through {MAX_BOOKS}, was {}",
                self.max_waiting
            )));
        }
        if self.max_holders == 0 || self.max_holders > MAX_BOOKS {
            return Err(InvalidPlacement(format!(
                "maxHolders must be 1 through {MAX_BOOKS}, was {}",
                self.max_holders
            )));
        }
        if let Some(alerts) = &self.alerts {
            check_message_type(&alerts.message_type)?;
        }
        if self.nodes.len() > self.max_nodes as usize {
            return Err(InvalidPlacement(format!(
                "at most maxNodes ({}) nodes are known, had {}",
                self.max_nodes,
                self.nodes.len()
            )));
        }
        for (node, entry) in &self.nodes {
            check_id("a node", node)?;
            check_id("an instance", &entry.instance)?;
            check_labels(&entry.labels, "labels", "label")?;
            if !sorted_once(&entry.labels) {
                return Err(InvalidPlacement(format!("the labels of '{node}' must be sorted, each once")));
            }
            check_places(entry.places)?;
            self.deadline(entry.heard_at).map_err(InvalidPlacement)?;
        }
        self.check_books()?;
        let live = self.nodes.values().any(|n| n.status == PlacementNodeStatus::Live);
        if live != self.armed.is_some() || live != self.armed_id.is_some() {
            return Err(InvalidPlacement("armed and armedId are set exactly while a node is live".to_owned()));
        }
        Ok(())
    }

    /// Checks the holders and the line: their limits, each holder id once, needs and message types in bounds, each
    /// holder on a known node with a grant below `nextGrant`.
    fn check_books(&self) -> Result<(), InvalidPlacement> {
        if self.next_grant == 0 {
            return Err(InvalidPlacement("nextGrant must be at least 1".to_owned()));
        }
        if self.holders.len() > self.max_holders as usize {
            return Err(InvalidPlacement(format!(
                "at most maxHolders ({}) hold a place, had {}",
                self.max_holders,
                self.holders.len()
            )));
        }
        if self.waiting.len() > self.max_waiting as usize {
            return Err(InvalidPlacement(format!(
                "at most maxWaiting ({}) asks wait, had {}",
                self.max_waiting,
                self.waiting.len()
            )));
        }
        let mut seen = HashSet::new();
        let held = self.holders.iter().map(|h| (&h.holder, &h.needs, &h.message_type));
        let waiting = self.waiting.iter().map(|w| (&w.holder, &w.needs, &w.message_type));
        for (holder, needs, message_type) in held.chain(waiting) {
            check_ask_fields(holder, needs, message_type)?;
            if !sorted_once(needs) {
                return Err(InvalidPlacement(format!("the needs of '{holder}' must be sorted, each once")));
            }
            if !seen.insert(holder.as_str()) {
                return Err(InvalidPlacement(format!("the holder '{holder}' is in the books twice")));
            }
        }
        for holder in &self.holders {
            if holder.grant == 0 || holder.grant >= self.next_grant {
                return Err(InvalidPlacement(format!(
                    "a grant is at least 1 and below nextGrant ({}), was {}",
                    self.next_grant, holder.grant
                )));
            }
            if !self.nodes.contains_key(&holder.node) {
                return Err(InvalidPlacement(format!(
                    "the holder '{}' is on '{}', a node the pool does not know",
                    holder.holder, holder.node
                )));
            }
        }
        Ok(())
    }

    /// The end of the silence of a node heard at `at`: `at` plus the silence, refused when that is not representable
    /// or after `MAX_INSTANT`.
    fn deadline(&self, at: Timestamp) -> Result<Timestamp, String> {
        match time::add(at, self.silence) {
            Ok(due) if due <= MAX_INSTANT => Ok(due),
            _ => Err(format!("the silence would end after {}", time::format_instant(MAX_INSTANT))),
        }
    }

    /// The earliest end of a live node's silence; none when no node is live.
    fn next_check(&self) -> Option<Timestamp> {
        let live = self.nodes.values().filter(|n| n.status == PlacementNodeStatus::Live);
        live.filter_map(|n| self.deadline(n.heard_at).ok()).min()
    }
}

/// Checks the fields of an offer and gives its labels sorted, each once.
fn check_offer(
    node: &str,
    instance: &str,
    mut labels: Vec<String>,
    places: u32,
) -> Result<Vec<String>, InvalidPlacement> {
    check_id("a node", node)?;
    check_id("an instance", instance)?;
    check_labels(&labels, "labels", "label")?;
    check_places(places)?;
    labels.sort();
    labels.dedup();
    Ok(labels)
}

/// Checks the fields of an ask and gives its needs sorted, each once.
fn check_ask(holder: &str, mut needs: Vec<String>, message_type: &str) -> Result<Vec<String>, InvalidPlacement> {
    check_ask_fields(holder, &needs, message_type)?;
    needs.sort();
    needs.dedup();
    Ok(needs)
}

fn check_ask_fields(holder: &str, needs: &[String], message_type: &str) -> Result<(), InvalidPlacement> {
    check_holder(holder)?;
    check_labels(needs, "needs", "need")?;
    check_message_type(message_type)
}

fn check_holder(holder: &str) -> Result<(), InvalidPlacement> {
    let length = holder.chars().count();
    if length == 0 || length > MAX_HOLDER_LENGTH || holder.contains('\0') {
        return Err(InvalidPlacement(format!(
            "a holder id must be 1 through {MAX_HOLDER_LENGTH} characters without NUL, had {length}"
        )));
    }
    Ok(())
}

/// Whether `labels` are sorted, each once.
fn sorted_once(labels: &[String]) -> bool {
    labels.windows(2).all(|pair| pair[0] < pair[1])
}

fn check_id(what: &str, id: &str) -> Result<(), InvalidPlacement> {
    let length = id.chars().count();
    if length == 0 || length > MAX_ID_LENGTH || id.contains('\0') || id.trim().is_empty() {
        return Err(InvalidPlacement(format!(
            "{what} id must be 1 through {MAX_ID_LENGTH} characters, not blank, without NUL, had {length}"
        )));
    }
    Ok(())
}

/// Checks the labels of an offer or the needs of an ask; `plural` and `one` name them in the error.
fn check_labels(labels: &[String], plural: &str, one: &str) -> Result<(), InvalidPlacement> {
    if labels.len() > MAX_LABELS {
        return Err(InvalidPlacement(format!("at most {MAX_LABELS} {plural}, had {}", labels.len())));
    }
    for label in labels {
        let length = label.chars().count();
        if length == 0 || length > MAX_LABEL_LENGTH || label.contains('\0') {
            return Err(InvalidPlacement(format!(
                "a {one} must be 1 through {MAX_LABEL_LENGTH} characters without NUL, had {length}"
            )));
        }
    }
    Ok(())
}

fn check_places(places: u32) -> Result<(), InvalidPlacement> {
    if places > MAX_PLACES {
        return Err(InvalidPlacement(format!("places must be 0 through {MAX_PLACES}, was {places}")));
    }
    Ok(())
}

fn check_message_type(message_type: &str) -> Result<(), InvalidPlacement> {
    let length = message_type.chars().count();
    if length == 0 || length > MAX_TYPE_LENGTH || message_type.contains('\0') {
        return Err(InvalidPlacement(format!(
            "messageType must be 1 through {MAX_TYPE_LENGTH} characters without NUL, had {length}"
        )));
    }
    Ok(())
}

/// The placement pool type. It has no settings: the silence time and the limits are in the state of each pool, given
/// at its create; a create of a pool that exists changes nothing. It never stops itself, and answers no query: read
/// its state.
///
/// Each `Offer` replaces the node's entry and records the store time of its turn as `heardAt`. While a node is live the
/// pool has one `Check` armed, with the crate's shared retry policy, at the earliest `heardAt` plus the silence of a
/// live node, and keeps its time in `armed` and its id in `armedId`. The armed check marks each live node whose silence
/// ended as `silent` and queues one `Alert` to itself for each; its next offer makes the node live and queues a `back`
/// alert. Each `Alert` sends a [`PlacementNotice`] to the target of the last `Alerts`, so a target that is gone or
/// refuses the notice fails that alert and nothing else; the node stays silent. The armed check run early, made due by
/// an operator, marks nothing and arms itself again at `armed`; any other `Check` the pool sent itself changes nothing,
/// and one from another sender (an operator's retry or redrive comes without one) is refused. A check that is purged,
/// cancelled or failed for good leaves the pool with a stale `armedId` and no timer until `Rearm`, which cancels it and
/// arms a new one at `armed`.
///
/// Each `Ask` is answered in its own commit with a [`PlacementAnswer`], with the shared retry policy: `granted` with the
/// next grant number and the node, `queued`, or `refused` with its reason, so a target that is gone fails the ask and
/// nothing is recorded. A node's free places are its offered places minus its holders; an offer with fewer places, or a
/// silent node, keeps every holder and gets no new ones. After every message the pool grants waiting asks in line
/// order while it holds fewer than `maxHolders`, skipping each ask no live node has a free place for now (ADR 0068), in
/// the commit that made room; each `granted` answer follows in a `Notify` that the pool sends itself, which reads the
/// state when it runs. A place comes back by a `Release` that names its grant number, or when its holder's target is
/// deleted. A holder whose target is gone before its `granted` answer ever committed costs nothing: the pool watches
/// itself, and the `Failed` of its own `Notify` frees that place and grants the next ask that fits. A waiting ask stays
/// until it is granted or cancelled, also when no node has its needs any more, or until its target is deleted.
///
/// The pool watches the target of each holder and waiting ask with
/// [`Turn::watch_existing`](cicada::Turn::watch_existing), from the commit that first records it, and unwatches it at
/// the end of the turn after which it holds and waits for nothing. So a target deleted before or after that commit
/// sends one `Stopped`, which frees every place held for it and takes its waiting asks out of the line: a holder need
/// not release before it stops. The watch also tells the pool of each failed message of a target, a turn that changes
/// nothing. An ask whose target is the pool itself is refused, as the pool's own watch frees undelivered grants.
///
/// Silence is counted from the store time an offer was handled; due times are not run times, so a paused or slow pool
/// marks a node late, never before its silence ended. A pool hosted only on its member nodes runs no check once they
/// all go quiet. Bad input is refused for good with the state unchanged: a message out of bounds, a new node past
/// `maxNodes`, a `Check`, `Alert` or `Notify` from another sender, and a notice or answer the send refuses; a read of
/// the store that fails is tried again.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlacementActor;

impl Actor for PlacementActor {
    type Ref = Placement;
    type State = PlacementState;

    async fn handle(&self, turn: &mut Turn<Self>, message: PlacementMsg) -> HandlerResult {
        let before = targets(turn.state());
        match message {
            PlacementMsg::Offer { node, instance, labels, places } => {
                let labels = check_offer(&node, &instance, labels, places).map_err(HandlerError::permanent)?;
                offer(turn, node, instance, labels, places)?;
            }
            PlacementMsg::Check => {
                refuse_foreign(turn, "Check")?;
                check(turn)?;
            }
            PlacementMsg::Rearm => {
                if let Some(at) = turn.state().armed {
                    arm(turn, at)?;
                }
            }
            PlacementMsg::Alert { node, kind } => {
                check_id("a node", &node).map_err(HandlerError::permanent)?;
                refuse_foreign(turn, "Alert")?;
                alert(turn, node, kind).await?;
            }
            PlacementMsg::Alerts { to, message_type } => {
                check_message_type(&message_type).map_err(HandlerError::permanent)?;
                turn.state_mut().alerts = Some(PlacementAlerts { to, message_type });
            }
            PlacementMsg::Ask { holder, needs, to, message_type } => {
                let needs = check_ask(&holder, needs, &message_type).map_err(HandlerError::permanent)?;
                if &to == turn.self_address() {
                    return Err(HandlerError::permanent("an ask's target cannot be the pool itself"));
                }
                ask(turn, PlacementAsk { holder, needs, to, message_type }).await?;
            }
            PlacementMsg::Release { holder, grant } => {
                check_holder(&holder).map_err(HandlerError::permanent)?;
                if let Some(i) = turn.state().holders.iter().position(|h| h.holder == holder && h.grant == grant) {
                    turn.state_mut().holders.remove(i);
                }
            }
            PlacementMsg::Cancel { holder } => {
                check_holder(&holder).map_err(HandlerError::permanent)?;
                if let Some(i) = turn.state().waiting.iter().position(|w| w.holder == holder) {
                    turn.state_mut().waiting.remove(i);
                }
            }
            PlacementMsg::Notify { holder } => {
                check_holder(&holder).map_err(HandlerError::permanent)?;
                refuse_foreign(turn, "Notify")?;
                notify(turn, &holder).await?;
            }
            PlacementMsg::Failed(failed) if turn.message().notification => undelivered(turn, &failed),
            PlacementMsg::Stopped(stopped) if turn.message().notification => gone(turn, &stopped.actor),
            PlacementMsg::Failed(_) | PlacementMsg::Stopped(_) => {}
        }
        grant_waiting(turn)?;
        follow(turn, before)
    }
}

/// The targets of the holders and the waiting asks.
fn targets(state: &PlacementState) -> HashSet<ActorAddress> {
    let held = state.holders.iter().map(|h| &h.to);
    held.chain(state.waiting.iter().map(|w| &w.to)).cloned().collect()
}

/// Watches the targets new since `before` with `watch_existing`, so one deleted before this commit is told of too, and
/// unwatches those that hold and wait for nothing now.
fn follow(turn: &mut Turn<PlacementActor>, before: HashSet<ActorAddress>) -> HandlerResult {
    let after = targets(turn.state());
    let mut new: Vec<&ActorAddress> = after.difference(&before).collect();
    let mut left: Vec<&ActorAddress> = before.difference(&after).collect();
    new.sort();
    left.sort();
    for target in new {
        turn.watch_existing(target)?;
    }
    for target in left {
        turn.unwatch(target)?;
    }
    Ok(())
}

/// Frees every place held for `actor`, which went away, and takes its waiting asks out of the line; nothing for the
/// pool itself.
fn gone(turn: &mut Turn<PlacementActor>, actor: &ActorAddress) {
    if actor == turn.self_address() {
        return;
    }
    let state = turn.state_mut();
    state.holders.retain(|h| &h.to != actor);
    state.waiting.retain(|w| &w.to != actor);
}

/// The free places of each live node: its offered places minus its holders.
fn free_places(state: &PlacementState) -> BTreeMap<&str, u32> {
    let mut free: BTreeMap<&str, u32> = state
        .nodes
        .iter()
        .filter(|(_, n)| n.status == PlacementNodeStatus::Live)
        .map(|(id, n)| (id.as_str(), n.places))
        .collect();
    for holder in &state.holders {
        if let Some(places) = free.get_mut(holder.node.as_str()) {
            *places = places.saturating_sub(1);
        }
    }
    free
}

/// Whether `labels` has every one of `needs`; both sorted.
fn has_needs(labels: &[String], needs: &[String]) -> bool {
    needs.iter().all(|need| labels.binary_search(need).is_ok())
}

/// The node for `needs` now: of the live nodes with every need and a free place, the one with the most free places,
/// ties to the smaller node id.
fn place<'a>(state: &PlacementState, free: &BTreeMap<&'a str, u32>, needs: &[String]) -> Option<&'a str> {
    let mut best: Option<(&str, u32)> = None;
    for (&node, &places) in free {
        if places > 0 && has_needs(&state.nodes[node].labels, needs) && best.is_none_or(|(_, most)| places > most) {
            best = Some((node, places));
        }
    }
    best.map(|(node, _)| node)
}

/// Answers an `Ask` in this commit: again for a holder id in the books, otherwise by a grant, a place in the line, or a
/// refusal that records nothing.
async fn ask(turn: &mut Turn<PlacementActor>, ask: PlacementAsk) -> HandlerResult {
    let state = turn.state();
    if let Some(i) = state.holders.iter().position(|h| h.holder == ask.holder) {
        let h = &state.holders[i];
        let (to, message_type) = (h.to.clone(), h.message_type.clone());
        let answer = granted(turn, &ask.holder, h.grant, h.node.clone());
        tell(turn, &to, &message_type, answer).await?;
        turn.state_mut().holders[i].notified = true;
        return Ok(());
    }
    if let Some(w) = state.waiting.iter().find(|w| w.holder == ask.holder) {
        let (to, message_type) = (w.to.clone(), w.message_type.clone());
        let answer = answer(turn, &ask.holder, PlacementAnswerKind::Queued);
        return tell(turn, &to, &message_type, answer).await;
    }
    if !state.nodes.values().any(|n| has_needs(&n.labels, &ask.needs)) {
        return refuse(turn, &ask, PlacementRefusal::NoNode).await;
    }
    let free = free_places(state);
    let node = place(state, &free, &ask.needs).map(str::to_owned);
    let room = state.holders.len() < state.max_holders as usize;
    match node {
        Some(node) if room => {
            let grant = state.next_grant;
            let next_grant = next_grant(grant)?;
            let answer = granted(turn, &ask.holder, grant, node.clone());
            tell(turn, &ask.to, &ask.message_type, answer).await?;
            let state = turn.state_mut();
            state.next_grant = next_grant;
            let PlacementAsk { holder, needs, to, message_type } = ask;
            state.holders.push(PlacementHolder { holder, needs, to, message_type, grant, node, notified: true });
        }
        _ if state.waiting.len() < state.max_waiting as usize => {
            let answer = answer(turn, &ask.holder, PlacementAnswerKind::Queued);
            tell(turn, &ask.to, &ask.message_type, answer).await?;
            turn.state_mut().waiting.push(ask);
        }
        Some(_) => refuse(turn, &ask, PlacementRefusal::PoolFull).await?,
        None => refuse(turn, &ask, PlacementRefusal::LineFull).await?,
    }
    Ok(())
}

/// The grant number after `grant`; refused for good when the numbers are used up.
fn next_grant(grant: u64) -> Result<u64, HandlerError> {
    grant.checked_add(1).ok_or_else(|| HandlerError::permanent("the grant numbers of this pool are used up"))
}

/// Grants waiting asks in line order while the pool holds fewer than `maxHolders`, skipping each ask no live node has
/// a free place for, and sends each its `granted` answer in a turn of its own.
fn grant_waiting(turn: &mut Turn<PlacementActor>) -> HandlerResult {
    let state = turn.state();
    if state.waiting.is_empty() || state.holders.len() >= state.max_holders as usize {
        return Ok(());
    }
    let mut free = free_places(state);
    let mut room = state.max_holders as usize - state.holders.len();
    let mut placed: Vec<(usize, String)> = Vec::new();
    for (i, waiting) in state.waiting.iter().enumerate() {
        if room == 0 {
            break;
        }
        if let Some(node) = place(state, &free, &waiting.needs) {
            if let Some(places) = free.get_mut(node) {
                *places -= 1;
            }
            placed.push((i, node.to_owned()));
            room -= 1;
        }
    }
    if placed.is_empty() {
        return Ok(());
    }
    let mut placed = placed.into_iter().peekable();
    let line = std::mem::take(&mut turn.state_mut().waiting);
    for (i, waiting) in line.into_iter().enumerate() {
        let Some((_, node)) = placed.next_if(|(j, _)| *j == i) else {
            turn.state_mut().waiting.push(waiting);
            continue;
        };
        let PlacementAsk { holder, needs, to, message_type } = waiting;
        let grant = turn.state().next_grant;
        let next_grant = next_grant(grant)?;
        let state = turn.state_mut();
        state.next_grant = next_grant;
        let record = PlacementHolder { holder: holder.clone(), needs, to, message_type, grant, node, notified: false };
        state.holders.push(record);
        let mut options = SendOptions::new().retry(retry_policy());
        options.correlation_id = turn.message().correlation_id.clone();
        turn.send_self_with(PlacementMsg::Notify { holder }, options)?;
    }
    let itself = turn.self_address().clone();
    turn.watch(&itself)?;
    Ok(())
}

/// Sends `holder` its `granted` answer when it still holds a place, and marks it as told.
async fn notify(turn: &mut Turn<PlacementActor>, holder: &str) -> HandlerResult {
    let Some(i) = turn.state().holders.iter().position(|h| h.holder == holder) else { return Ok(()) };
    let h = &turn.state().holders[i];
    let (to, message_type, grant, node) = (h.to.clone(), h.message_type.clone(), h.grant, h.node.clone());
    let answer = granted(turn, holder, grant, node);
    tell(turn, &to, &message_type, answer).await?;
    turn.state_mut().holders[i].notified = true;
    Ok(())
}

/// Frees the place of a `Notify` that the pool sent itself and that failed before the holder's `granted` answer ever
/// committed: the holder never learned its place, so no work runs on it.
fn undelivered(turn: &mut Turn<PlacementActor>, failed: &Failed) {
    let itself = turn.self_address();
    if &failed.actor != itself || failed.from.as_ref() != Some(itself) {
        return;
    }
    let Ok(PlacementMsg::Notify { holder }) =
        cicada::message::decode::<PlacementMsg>(&failed.message_type, &failed.payload)
    else {
        return;
    };
    if let Some(i) = turn.state().holders.iter().position(|h| h.holder == holder && !h.notified) {
        turn.state_mut().holders.remove(i);
    }
}

/// An answer of this pool to `holder` without grant, node or reason.
fn answer(turn: &Turn<PlacementActor>, holder: &str, kind: PlacementAnswerKind) -> PlacementAnswer {
    let pool = turn.self_ref().pool.clone();
    PlacementAnswer { pool, holder: holder.to_owned(), kind, grant: None, node: None, reason: None }
}

/// A `granted` answer of this pool to `holder`.
fn granted(turn: &Turn<PlacementActor>, holder: &str, grant: u64, node: String) -> PlacementAnswer {
    PlacementAnswer { grant: Some(grant), node: Some(node), ..answer(turn, holder, PlacementAnswerKind::Granted) }
}

/// Answers `ask` with `refused` and `reason`, recording nothing.
async fn refuse(turn: &mut Turn<PlacementActor>, ask: &PlacementAsk, reason: PlacementRefusal) -> HandlerResult {
    let answer = PlacementAnswer { reason: Some(reason), ..answer(turn, &ask.holder, PlacementAnswerKind::Refused) };
    tell(turn, &ask.to, &ask.message_type, answer).await
}

/// Sends an answer to the target of an ask, with the shared retry policy and this message's correlation id. A send the
/// registry refuses is refused for good; a failed read of the store is tried again.
async fn tell(
    turn: &mut Turn<PlacementActor>,
    to: &ActorAddress,
    message_type: &str,
    answer: PlacementAnswer,
) -> HandlerResult {
    let mut options = SendOptions::new().retry(retry_policy());
    options.correlation_id = turn.message().correlation_id.clone();
    match turn.send_raw(to, message_type, serde_json::to_value(answer)?, options).await {
        Ok(_) => Ok(()),
        // Tried again by the retry policy of the incoming message.
        Err(e @ TurnError::Store(_)) => Err(e.into()),
        // No registered type, a type that does not take the message, or a field out of bounds.
        Err(e) => Err(HandlerError::permanent_from(e)),
    }
}

/// Replaces the entry of `node`, tells the target of a node that is back, and arms the check when none is armed.
fn offer(
    turn: &mut Turn<PlacementActor>,
    node: String,
    instance: String,
    labels: Vec<String>,
    places: u32,
) -> HandlerResult {
    let now = turn.now();
    let state = turn.state();
    let due = state.deadline(now).map_err(HandlerError::permanent)?;
    let previous = state.nodes.get(&node).map(|n| n.status);
    if previous.is_none() && state.nodes.len() >= state.max_nodes as usize {
        return Err(HandlerError::permanent(format!(
            "the pool knows maxNodes ({}) nodes: '{node}' is refused",
            state.max_nodes
        )));
    }
    if previous == Some(PlacementNodeStatus::Silent) {
        queue_alert(turn, node.clone(), PlacementNoticeKind::Back)?;
    }
    if turn.state().armed.is_none() {
        arm(turn, due)?;
    }
    let entry = PlacementNode { instance, labels, places, heard_at: now, status: PlacementNodeStatus::Live };
    turn.state_mut().nodes.insert(node, entry);
    Ok(())
}

/// Marks each live node whose silence ended as silent with an alert for each, and arms the next check; for the armed
/// check run early, arms it again at `armed`; for any other check, nothing.
fn check(turn: &mut Turn<PlacementActor>) -> HandlerResult {
    let state = turn.state();
    let (Some(armed), Some(armed_id)) = (state.armed, state.armed_id.as_ref()) else { return Ok(()) };
    if *armed_id != turn.message().id {
        return Ok(());
    }
    let now = turn.now();
    if now < armed {
        return Ok(arm(turn, armed)?);
    }
    let ended: Vec<String> = state
        .nodes
        .iter()
        .filter(|(_, n)| n.status == PlacementNodeStatus::Live && state.deadline(n.heard_at).is_ok_and(|d| d <= now))
        .map(|(node, _)| node.clone())
        .collect();
    for node in ended {
        if let Some(entry) = turn.state_mut().nodes.get_mut(&node) {
            entry.status = PlacementNodeStatus::Silent;
        }
        queue_alert(turn, node, PlacementNoticeKind::Silent)?;
    }
    match turn.state().next_check() {
        Some(at) => arm(turn, at)?,
        None => {
            let state = turn.state_mut();
            state.armed = None;
            state.armed_id = None;
        }
    }
    Ok(())
}

/// Cancels the armed check, unless it is the message of this turn, and arms a `Check` at `at` with the shared retry
/// policy, keeping its time and id.
fn arm(turn: &mut Turn<PlacementActor>, at: Timestamp) -> Result<(), TurnError> {
    if let Some(armed) = turn.state().armed_id.clone()
        && armed != turn.message().id
    {
        turn.cancel(&armed)?;
    }
    let id = turn.send_self_with(PlacementMsg::Check, SendOptions::new().at(at).retry(retry_policy()))?;
    let state = turn.state_mut();
    state.armed = Some(at);
    state.armed_id = Some(id);
    Ok(())
}

/// Sends the pool an `Alert` about `node`, with the shared retry policy.
fn queue_alert(turn: &mut Turn<PlacementActor>, node: String, kind: PlacementNoticeKind) -> Result<(), TurnError> {
    turn.send_self_with(PlacementMsg::Alert { node, kind }, SendOptions::new().retry(retry_policy()))?;
    Ok(())
}

/// Sends the alert target its notice, with the shared retry policy; nothing without a target. A send the registry
/// refuses is refused for good; a failed read of the store is tried again.
async fn alert(turn: &mut Turn<PlacementActor>, node: String, kind: PlacementNoticeKind) -> HandlerResult {
    let Some(PlacementAlerts { to, message_type }) = turn.state().alerts.clone() else { return Ok(()) };
    let notice = PlacementNotice { pool: turn.self_ref().pool.clone(), node, kind };
    let options = SendOptions::new().retry(retry_policy());
    match turn.send_raw(&to, &message_type, serde_json::to_value(notice)?, options).await {
        Ok(_) => Ok(()),
        // Tried again by the retry policy of the alert.
        Err(e @ TurnError::Store(_)) => Err(e.into()),
        // No registered type, a type that does not take the message, or a field out of bounds.
        Err(e) => Err(HandlerError::permanent_from(e)),
    }
}

/// Refuses for good a message of the pool's own that another sender sent.
fn refuse_foreign(turn: &Turn<PlacementActor>, what: &str) -> HandlerResult {
    if turn.message().from.as_ref() != Some(turn.self_address()) {
        return Err(HandlerError::permanent(format!("a {what} is sent only by the pool itself")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: &str = "2026-01-01T00:00:00Z";

    fn node(status: &str) -> serde_json::Value {
        serde_json::json!({"instance": "i1", "labels": ["arm64", "docker"], "places": 4, "heardAt": AT,
            "status": status})
    }

    fn state(extra: serde_json::Value) -> serde_json::Value {
        let mut json = serde_json::json!({"silence": "PT1M", "maxNodes": 2, "maxWaiting": 2, "maxHolders": 2});
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    }

    fn decode(json: serde_json::Value) -> Result<PlacementState, serde_json::Error> {
        serde_json::from_value(json)
    }

    #[test]
    fn the_constructor_takes_a_silence_of_1_second_through_1_day_and_its_limits() {
        let minute = Duration::from_secs(60);
        let state = PlacementState::new(minute, 2, 3, 4).unwrap();
        assert_eq!((state.silence, state.max_nodes, state.alerts, state.armed), (minute, 2, None, None));
        assert_eq!((state.max_waiting, state.max_holders, state.next_grant), (3, 4, 1));
        assert!(state.nodes.is_empty() && state.holders.is_empty() && state.waiting.is_empty());
        assert!(PlacementState::new(MIN_SILENCE, 1, 0, 1).is_ok());
        assert!(PlacementState::new(MAX_SILENCE, MAX_NODES, MAX_BOOKS, MAX_BOOKS).is_ok());
        let short = MIN_SILENCE - Duration::from_nanos(1);
        let long = MAX_SILENCE + Duration::from_nanos(1);
        for (silence, max_nodes, max_waiting, max_holders, why) in [
            (short, 1, 0, 1, "silence must be"),
            (long, 1, 0, 1, "silence must be"),
            (Duration::ZERO, 1, 0, 1, "silence must be"),
            (minute, 0, 0, 1, "maxNodes must be"),
            (minute, 1001, 0, 1, "maxNodes must be"),
            (minute, 1, 1001, 1, "maxWaiting must be"),
            (minute, 1, 0, 0, "maxHolders must be"),
            (minute, 1, 0, 1001, "maxHolders must be"),
        ] {
            let built = PlacementState::new(silence, max_nodes, max_waiting, max_holders);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
        }
    }

    #[test]
    fn a_decoded_state_is_checked_like_a_built_one() {
        let decoded = decode(state(serde_json::json!({"other": 1}))).unwrap();
        assert_eq!(
            decoded,
            PlacementState::new(Duration::from_secs(60), 2, 2, 2).unwrap(),
            "unknown fields are ignored"
        );
        let full = state(serde_json::json!({"alerts": {"to": "ops/_", "messageType": "Placement"},
            "nodes": {"a": node("live"), "b": node("silent")}, "armed": AT, "armedId": "c1"}));
        assert!(decode(full).is_ok());
        assert!(decode(state(serde_json::json!({"nodes": {"b": node("silent")}}))).is_ok(), "no live node, no armed");
        let mut unsorted = node("live");
        unsorted["labels"] = serde_json::json!(["docker", "arm64"]);
        let mut twice = node("live");
        twice["labels"] = serde_json::json!(["arm64", "arm64"]);
        let mut many = node("live");
        many["labels"] = serde_json::json!((0..33).map(|n| format!("l{n:02}")).collect::<Vec<_>>());
        let mut long_label = node("live");
        long_label["labels"] = serde_json::json!(["l".repeat(101)]);
        let mut places = node("live");
        places["places"] = serde_json::json!(1001);
        let mut instance = node("live");
        instance["instance"] = serde_json::json!(" ");
        let mut late = node("silent");
        late["heardAt"] = serde_json::json!("9999-12-31T23:59:30Z");
        let long_node = "n".repeat(201);
        for (extra, why) in [
            (serde_json::json!({"silence": "PT0.5S"}), "silence must be"),
            (serde_json::json!({"silence": "P2D"}), "silence must be"),
            (serde_json::json!({"maxNodes": 0}), "maxNodes must be"),
            (serde_json::json!({"maxNodes": 1001}), "maxNodes must be"),
            (serde_json::json!({"alerts": {"to": "ops/_", "messageType": ""}}), "messageType must be"),
            (
                serde_json::json!({"nodes": {"a": node("silent"), "b": node("silent"), "c": node("silent")}}),
                "at most maxNodes",
            ),
            (serde_json::json!({"nodes": {"": node("silent")}}), "a node id must be"),
            (serde_json::json!({"nodes": {"  ": node("silent")}}), "a node id must be"),
            (serde_json::json!({"nodes": {long_node: node("silent")}}), "a node id must be"),
            (serde_json::json!({"nodes": {"a": instance}, "armed": AT, "armedId": "c1"}), "an instance id must be"),
            (serde_json::json!({"nodes": {"a": unsorted}, "armed": AT, "armedId": "c1"}), "sorted, each once"),
            (serde_json::json!({"nodes": {"a": twice}, "armed": AT, "armedId": "c1"}), "sorted, each once"),
            (serde_json::json!({"nodes": {"a": many}, "armed": AT, "armedId": "c1"}), "at most 32 labels"),
            (serde_json::json!({"nodes": {"a": long_label}, "armed": AT, "armedId": "c1"}), "a label must be"),
            (serde_json::json!({"nodes": {"a": places}, "armed": AT, "armedId": "c1"}), "places must be"),
            (serde_json::json!({"nodes": {"a": late}}), "after 9999-12-31"),
            (serde_json::json!({"nodes": {"a": node("live")}}), "armed and armedId are set exactly"),
            (
                serde_json::json!({"nodes": {"a": node("silent")}, "armed": AT, "armedId": "c1"}),
                "armed and armedId are set exactly",
            ),
            (serde_json::json!({"armed": AT, "armedId": "c1"}), "armed and armedId are set exactly"),
            (serde_json::json!({"nodes": {"a": node("live")}, "armed": AT}), "armed and armedId are set exactly"),
            (serde_json::json!({"nodes": {"a": node("live")}, "armedId": "c1"}), "armed and armedId are set exactly"),
        ] {
            let json = state(extra);
            let decoded = decode(json.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {json}");
        }
        for missing in ["silence", "maxNodes", "maxWaiting", "maxHolders"] {
            let mut json = state(serde_json::json!({}));
            json.as_object_mut().unwrap().remove(missing);
            assert!(decode(json).is_err(), "{missing} is required");
        }
    }

    fn holder(holder: &str, grant: u64, node: &str) -> serde_json::Value {
        serde_json::json!({"holder": holder, "needs": ["arm64"], "to": "run/r", "messageType": "Placed",
            "grant": grant, "node": node, "notified": true})
    }

    fn waiting(holder: &str) -> serde_json::Value {
        serde_json::json!({"holder": holder, "needs": ["gpu"], "to": "run/r", "messageType": "Placed"})
    }

    #[test]
    fn decoded_holders_and_waiting_asks_are_checked() {
        let nodes = serde_json::json!({"a": node("silent")});
        let books = |extra: serde_json::Value| {
            let mut json = state(serde_json::json!({"nodes": nodes}));
            json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            json
        };
        let full = books(serde_json::json!({"nextGrant": 5, "holders": [holder("h1", 3, "a"), holder("h2", 4, "a")],
            "waiting": [waiting("w1"), waiting("w2")]}));
        assert!(decode(full).is_ok(), "more holders than places is a state after a shrink");
        assert_eq!(decode(books(serde_json::json!({}))).unwrap().next_grant, 1, "nextGrant may be left out");
        let mut unsorted = waiting("w1");
        unsorted["needs"] = serde_json::json!(["gpu", "arm64"]);
        let mut twice = holder("h1", 1, "a");
        twice["needs"] = serde_json::json!(["arm64", "arm64"]);
        let mut many = waiting("w1");
        many["needs"] = serde_json::json!((0..33).map(|n| format!("l{n:02}")).collect::<Vec<_>>());
        let mut empty_need = waiting("w1");
        empty_need["needs"] = serde_json::json!([""]);
        let mut message_type = waiting("w1");
        message_type["messageType"] = serde_json::json!("");
        for (extra, why) in [
            (serde_json::json!({"nextGrant": 0}), "nextGrant must be at least 1"),
            (serde_json::json!({"maxWaiting": 1001}), "maxWaiting must be"),
            (serde_json::json!({"maxHolders": 0}), "maxHolders must be"),
            (serde_json::json!({"maxHolders": 1001}), "maxHolders must be"),
            (
                serde_json::json!({"nextGrant": 9, "holders": [holder("h1", 1, "a"), holder("h2", 2, "a"),
                    holder("h3", 3, "a")]}),
                "at most maxHolders",
            ),
            (serde_json::json!({"waiting": [waiting("w1"), waiting("w2"), waiting("w3")]}), "at most maxWaiting"),
            (serde_json::json!({"nextGrant": 9, "holders": [holder("h", 1, "a"), holder("h", 2, "a")]}), "twice"),
            (serde_json::json!({"nextGrant": 9, "holders": [holder("h", 1, "a")], "waiting": [waiting("h")]}), "twice"),
            (serde_json::json!({"waiting": [waiting("w"), waiting("w")]}), "twice"),
            (serde_json::json!({"nextGrant": 3, "holders": [holder("h", 3, "a")]}), "below nextGrant"),
            (serde_json::json!({"nextGrant": 3, "holders": [holder("h", 0, "a")]}), "below nextGrant"),
            (serde_json::json!({"nextGrant": 3, "holders": [holder("h", 1, "b")]}), "a node the pool does not know"),
            (serde_json::json!({"waiting": [waiting("")]}), "a holder id must be"),
            (serde_json::json!({"waiting": [waiting(&"h".repeat(201))]}), "a holder id must be"),
            (serde_json::json!({"waiting": [waiting("a\0b")]}), "a holder id must be"),
            (serde_json::json!({"waiting": [unsorted]}), "sorted, each once"),
            (serde_json::json!({"nextGrant": 3, "holders": [twice]}), "sorted, each once"),
            (serde_json::json!({"waiting": [many]}), "at most 32 needs"),
            (serde_json::json!({"waiting": [empty_need]}), "a need must be"),
            (serde_json::json!({"waiting": [message_type]}), "messageType must be"),
        ] {
            let json = books(extra);
            let decoded = decode(json.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {json}");
        }
    }

    #[test]
    fn the_ask_release_and_cancel_helpers_check_their_fields() {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct Run;
        impl ActorRef for Run {
            const TYPE: &'static str = "run";
            type Message = PlacementMsg;
        }
        let ask = PlacementMsg::ask("h", ["b", "a", "b"], &Run, "Placed").unwrap();
        let to = Run.address().unwrap();
        let needs = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(ask, PlacementMsg::Ask { holder: "h".into(), needs, to, message_type: "Placed".into() });
        let max_needs: Vec<String> = (0..MAX_LABELS).map(|n| format!("l{n}")).collect();
        assert!(PlacementMsg::ask("h".repeat(200), max_needs, &Run, "Placed").is_ok());
        assert!(PlacementMsg::ask(" ", Vec::<String>::new(), &Run, "Placed").is_ok(), "a blank holder id is allowed");
        let too_many: Vec<String> = (0..=MAX_LABELS).map(|n| format!("l{n}")).collect();
        for (holder, needs, message_type, why) in [
            ("".to_owned(), vec![], "Placed".to_owned(), "a holder id"),
            ("h".repeat(201), vec![], "Placed".to_owned(), "a holder id"),
            ("a\0".to_owned(), vec![], "Placed".to_owned(), "a holder id"),
            ("h".to_owned(), too_many, "Placed".to_owned(), "at most 32 needs"),
            ("h".to_owned(), vec!["".to_owned()], "Placed".to_owned(), "a need"),
            ("h".to_owned(), vec!["l".repeat(101)], "Placed".to_owned(), "a need"),
            ("h".to_owned(), vec![], "".to_owned(), "messageType must be"),
            ("h".to_owned(), vec![], "T".repeat(MAX_TYPE_LENGTH + 1), "messageType must be"),
        ] {
            let built = PlacementMsg::ask(&holder, needs, &Run, &message_type);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
        }
        assert_eq!(PlacementMsg::release("h", 4).unwrap(), PlacementMsg::Release { holder: "h".into(), grant: 4 });
        assert_eq!(PlacementMsg::cancel("h").unwrap(), PlacementMsg::Cancel { holder: "h".into() });
        assert!(PlacementMsg::release("", 4).is_err() && PlacementMsg::cancel("h".repeat(201)).is_err());
    }

    #[test]
    fn grant_numbers_end_at_the_largest_number() {
        assert_eq!(next_grant(1).unwrap(), 2);
        assert!(next_grant(u64::MAX).is_err());
    }

    #[test]
    fn the_offer_helper_checks_its_fields_and_sorts_the_labels() {
        let offer = PlacementMsg::offer("n", "i", ["b", "a", "b"], 0).unwrap();
        assert_eq!(
            offer,
            PlacementMsg::Offer {
                node: "n".into(),
                instance: "i".into(),
                labels: vec!["a".into(), "b".into()],
                places: 0
            }
        );
        let max_labels: Vec<String> = (0..MAX_LABELS).map(|n| format!("l{n}")).collect();
        assert!(PlacementMsg::offer("n".repeat(200), "i".repeat(200), max_labels, MAX_PLACES).is_ok());
        let too_many: Vec<String> = (0..=MAX_LABELS).map(|n| format!("l{n}")).collect();
        for (node, instance, labels, places, why) in [
            ("".to_owned(), "i".to_owned(), vec![], 0, "a node id"),
            (" ".to_owned(), "i".to_owned(), vec![], 0, "a node id"),
            ("n".repeat(201), "i".to_owned(), vec![], 0, "a node id"),
            ("a\0".to_owned(), "i".to_owned(), vec![], 0, "a node id"),
            ("n".to_owned(), "".to_owned(), vec![], 0, "an instance id"),
            ("n".to_owned(), "i\0".to_owned(), vec![], 0, "an instance id"),
            ("n".to_owned(), "i".to_owned(), too_many, 0, "at most 32 labels"),
            ("n".to_owned(), "i".to_owned(), vec!["".to_owned()], 0, "a label"),
            ("n".to_owned(), "i".to_owned(), vec!["l".repeat(101)], 0, "a label"),
            ("n".to_owned(), "i".to_owned(), vec!["a\0".to_owned()], 0, "a label"),
            ("n".to_owned(), "i".to_owned(), vec![], 1001, "places must be"),
        ] {
            let built = PlacementMsg::offer(&node, &instance, labels, places);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
        }
    }

    #[test]
    fn the_alerts_helper_checks_the_message_type() {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct Ops;
        impl ActorRef for Ops {
            const TYPE: &'static str = "ops";
            type Message = PlacementMsg;
        }
        let alerts = PlacementMsg::alerts(&Ops, "Placement").unwrap();
        assert_eq!(
            serde_json::to_value(&alerts).unwrap(),
            serde_json::json!({"type": "Alerts", "to": "ops/_",
            "messageType": "Placement"})
        );
        for message_type in ["".to_owned(), "T".repeat(MAX_TYPE_LENGTH + 1), "a\0".to_owned()] {
            assert!(PlacementMsg::alerts(&Ops, &message_type).is_err(), "{message_type:?}");
        }
    }
}
