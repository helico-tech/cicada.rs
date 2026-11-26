//! The placement pool (ADR 0067): an actor to which nodes offer places, each with a set of labels, renewed at an
//! interval; a node not heard from for the pool's silence time is marked silent, and its alert target is told once. A
//! second machine offering under a live node's id is ignored and reported once (ADR 0069), as is a second machine its
//! member reports under the node's id and instance (ADR 0071), and a node offering a resource key that another live
//! node holds, such as one Docker daemon's id (ADR 0072). Work asks it for a place
//! with its needs and gets a node to pin a child to (ADR 0068). An operator drains a node, and declares a silent node
//! gone, whose holders are told. A [`PlacementMember`] offers a node's places while the host's probe answers, under a
//! [`NodeIdentity`] kept across restarts.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::limits::MAX_TYPE_LENGTH;
use cicada::time::{self, MAX_INSTANT, Timestamp, format_duration, serde_duration};
use cicada::turn::TurnError;
use cicada::{Failed, Stopped};
use serde::{Deserialize, Serialize};

use crate::supervisor::retry_policy;

mod member;

pub use member::{Capacity, IdentityErrorKind, InvalidIdentity, MemberSettings, NodeIdentity, PlacementMember};

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
/// The longest resource key, in characters.
const MAX_RESOURCE_LENGTH: usize = 200;

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

/// The messages of a [`Placement`] pool. `Check`, `Alert`, `Notify` and `TellGone` are its own: one from anyone else is
/// refused for good. A node id or instance id is 1 through 200 characters, not blank, without NUL; a holder id is 1
/// through 200 characters without NUL; a message out of bounds is refused for good with the state unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum PlacementMsg {
    /// A node offers its places: replaces the node's entry with this instance, these labels (sorted, each once),
    /// places and resource key, heard at the store time of this turn, and live. A silent node that offers is told
    /// `back` to the alert target, whatever its instance. An offer of another instance than the recorded one while the
    /// node is live changes no entry field: it marks the node in `conflict`, and a new mark is told `conflict`. An
    /// offer of a resource key that another live node holds changes nothing for the node: it is recorded in
    /// `contested`, and a new entry there is told `conflict` (ADR 0072). A node new to a pool that knows `maxNodes`
    /// nodes or more is refused for good, as is a new contest in a pool that lists `maxNodes` or more. Send one at an
    /// interval shorter than the silence time.
    Offer {
        /// The node id the member's node pins its actors under.
        #[schemars(length(min = 1, max = 200))]
        node: String,
        /// An id that tells two machines offering under one node id apart, kept across restarts, such as
        /// `NodeIdentity::instance`. One drawn at each process start makes a restart within the silence time look
        /// like a second machine.
        #[schemars(length(min = 1, max = 200))]
        instance: String,
        /// What the node offers, such as its processor kind: at most 32, each 1 through 100 characters without NUL.
        #[schemars(length(max = 32), inner(length(min = 1, max = 100)))]
        labels: Vec<String>,
        /// How many places the node offers, 0 through 1,000; 0 keeps the node known but full.
        #[schemars(range(max = 1000))]
        places: u32,
        /// The resource whose places these are, such as the Docker daemon's id (`RuntimeInfo::id` of
        /// `cicada-containers`): 1 through 200 characters without NUL. One live node holds a key (ADR 0072). Absent:
        /// none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(length(min = 1, max = 200))]
        resource: Option<String>,
    },
    /// A report, from anyone, such as a [`PlacementMember`] that found another host name in its node's row: another
    /// machine, `host`, runs under the id of `node` (ADR 0071). For a live node it marks the node in `conflict` with
    /// `host`, the node's own instance unless another instance marked it already, and the store time; a new mark is
    /// told `conflict`. The node gets no new places while the mark has a `host`, which ends one silence time after the
    /// last report (a mark set by an offer too ends as an offer's would, so when both would). Nothing for a silent
    /// node, which gets no new places anyway; refused for good for an unknown node.
    Duplicate {
        /// The node id the other machine runs under.
        #[schemars(length(min = 1, max = 200))]
        node: String,
        /// The host name of the other machine, as the node table shows it.
        #[schemars(length(min = 1, max = 200))]
        host: String,
    },
    /// The silence timer: sent by the pool to itself, and counted only when it is the armed one (`armedId`). Marks each
    /// live node not heard from for the silence time as silent, queues an `Alert` for each, removes each contest one
    /// silence time after its last offer, and arms the next check at the earliest end of a live node's silence or of a
    /// contest. Run early (made due by an operator) it arms itself again at `armed`;
    /// any other `Check` the pool sent itself changes nothing, and one from another sender is refused for good.
    Check,
    /// Cancels the armed check and arms a new one at its unchanged time, at once when that has passed: the recovery of
    /// a check that was purged, cancelled or failed for good. Nothing when no node is live and no contest is listed.
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
    /// Changes the pool's settings, from anyone: each field set replaces its value, each absent one keeps it, within
    /// the bounds of [`PlacementState::new`]. A limit below what the pool has removes nothing: the nodes, holders and
    /// waiting asks stay, and new ones are refused or queued until the count is below it. A larger `maxHolders` grants
    /// waiting asks in this turn. A silence other than the current one, with a check armed, arms the check at the new
    /// earliest end of a live node's silence or of a contest, at once when that has passed, so a shorter one marks the
    /// quiet nodes silent in the next turn. Refused for good with the state unchanged when no field is set, a value is
    /// out of bounds, or a node's or contest's silence would end after the last representable instant. Unknown fields
    /// are ignored, as in every message of the pool, so a misspelled field next to a valid one changes nothing.
    Configure {
        /// How long a node may go without an offer before it is silent, as an ISO-8601 duration; 1 second through 1
        /// day. Keep it above the members' offer interval. Absent: kept.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_duration::option")]
        #[schemars(with = "Option<String>")]
        silence: Option<Duration>,
        /// The most nodes the pool knows, 1 through 1,000. Absent: kept.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 1, max = 1000))]
        max_nodes: Option<u32>,
        /// The most asks that wait in the line, 0 through 1,000. Absent: kept.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(max = 1000))]
        max_waiting: Option<u32>,
        /// The most holders of the pool at once, 1 through 1,000. Absent: kept.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 1, max = 1000))]
        max_holders: Option<u32>,
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
    /// the smaller node id; queued when only silent or full nodes have the needs, or the pool holds `maxHolders` or
    /// more; refused with `noNode` when no node of the pool has them, `lineFull` when it must wait and the line holds
    /// `maxWaiting` asks or more, and `poolFull` when a place fits but the pool holds `maxHolders` and the line is
    /// full. The [`PlacementAnswer`] goes to the target in this commit, so a target that is gone fails the ask and
    /// nothing is recorded. A repeated `Ask` of a holder answers `granted` with the same grant and node, one of a
    /// waiting ask `queued`; both keep the first ask's needs and target. A holder id is forgotten at its release: use a
    /// fresh one per attempt. The pool watches the target from the commit that records the ask, and when it is deleted
    /// frees its places and takes its waiting asks out of the line; an ask whose target is the pool itself is refused
    /// for good.
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
    /// Stops new places on `node`: it keeps its holders, renews its offers and can go silent and come back, and an
    /// offer does not clear the drain. Draining a draining node changes nothing; an unknown node is refused for good.
    Drain {
        /// The node.
        #[schemars(length(min = 1, max = 200))]
        node: String,
    },
    /// Gives `node` new places again and grants the waiting asks that fit. Undraining a node that is not draining
    /// changes nothing; an unknown node is refused for good.
    Undrain {
        /// The node.
        #[schemars(length(min = 1, max = 200))]
        node: String,
    },
    /// Declares a silent node gone: removes it, frees all its places and tells each of its holders with a `gone`
    /// answer, each in a turn of its own. Refused for good for a live node (it is heard from) and an unknown one. A
    /// later `Release` of those grants frees nothing; the node offering again is a new node.
    Gone {
        /// The node; its status is `silent`.
        #[schemars(length(min = 1, max = 200))]
        node: String,
    },
    /// Sends `holder` its `gone` answer for `grant` on `node`, to `to` as `messageType`: sent by the pool to itself for
    /// each holder of a node declared gone, with a retry policy, so a failing target fails only its own answer.
    TellGone {
        /// The holder.
        #[schemars(length(min = 1, max = 200))]
        holder: String,
        /// The grant number it held.
        grant: u64,
        /// The node that is gone.
        #[schemars(length(min = 1, max = 200))]
        node: String,
        /// The target of its answers.
        to: ActorAddress,
        /// The message type of its answers; not empty, at most 200 characters, without NUL.
        #[schemars(length(min = 1, max = 200))]
        message_type: String,
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
        Self::offer_of(node.into(), instance.into(), labels.into_iter().map(Into::into).collect(), places, None)
    }

    /// An `Offer` as [`offer`](Self::offer) with the resource key `resource`, checked too.
    pub(crate) fn offer_of(
        node: String,
        instance: String,
        labels: Vec<String>,
        places: u32,
        resource: Option<String>,
    ) -> Result<Self, InvalidPlacement> {
        let labels = check_offer(&node, &instance, labels, places, resource.as_deref())?;
        Ok(PlacementMsg::Offer { node, instance, labels, places, resource })
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

    /// A `Drain` of `node`. Fails for a node id that is empty, blank, longer than 200 characters or holds a NUL.
    ///
    /// ```
    /// use cicada_actors::PlacementMsg;
    ///
    /// let drain = PlacementMsg::drain("factory-mac-1")?;
    /// assert_eq!(serde_json::to_value(&drain)?, serde_json::json!({"type": "Drain", "node": "factory-mac-1"}));
    /// assert!(PlacementMsg::drain(" ").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn drain(node: impl Into<String>) -> Result<Self, InvalidPlacement> {
        let node = node.into();
        check_id("a node", &node)?;
        Ok(PlacementMsg::Drain { node })
    }

    /// An `Undrain` of `node`. Fails for a node id out of bounds, as [`PlacementMsg::drain`].
    pub fn undrain(node: impl Into<String>) -> Result<Self, InvalidPlacement> {
        let node = node.into();
        check_id("a node", &node)?;
        Ok(PlacementMsg::Undrain { node })
    }

    /// A `Gone` of `node`. Fails for a node id out of bounds, as [`PlacementMsg::drain`].
    pub fn gone(node: impl Into<String>) -> Result<Self, InvalidPlacement> {
        let node = node.into();
        check_id("a node", &node)?;
        Ok(PlacementMsg::Gone { node })
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
    /// The node of the holder's place was declared gone, with the grant number and the node: the place is no longer
    /// counted, and what ran there may still run.
    Gone,
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
/// `node` are written only for `granted` and `gone`, `reason` only for `refused`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementAnswer {
    /// The pool: the key of its address.
    pub pool: String,
    /// The holder it is about.
    pub holder: String,
    /// What it says.
    pub kind: PlacementAnswerKind,
    /// The grant number, set for `granted` and `gone`: name it in the `Release`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<u64>,
    /// The node of the place, set for `granted` and `gone`: pin the work's actors to it.
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
    /// Another instance offered under the live node's id, whose offers are ignored, or a member reported another
    /// machine under it (`Duplicate`). Sent once while the mark lasts; the node's `conflict` names the instance and,
    /// for a report, the host. Or the node offered a resource key another live node holds, whose offers are ignored:
    /// sent once while its entry in the pool's `contested` lasts, which names the key.
    Conflict,
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
    /// Whether an operator drained it: it gets no new places until `Undrain`. Absent: false.
    #[serde(default)]
    pub draining: bool,
    /// The resource key of its last offer; 1 through 200 characters without NUL. While it is live no other live node
    /// has this key. Absent: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 200))]
    pub resource: Option<String>,
    /// The last offer of another instance, ignored, or the last report of another machine, while it was live; set only
    /// while it is live. Absent: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<PlacementConflict>,
}

/// An offer of another instance under a live node's id, which the pool ignored, or a report of another machine under
/// it (`Duplicate`, ADR 0071).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementConflict {
    /// The other instance id: not the node's recorded one, except for a mark set only by reports, which names the
    /// node's own; 1 through 200 characters, not blank, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub instance: String,
    /// The store time of the last conflicting offer or report.
    pub at: Timestamp,
    /// The host name of the other machine of the last report; set once a report marked the node, and the node gets no
    /// new places while it is. 1 through 200 characters, not blank, without NUL. Absent: no report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 200))]
    pub host: Option<String>,
}

/// An offer of a resource key that another live node held, which the pool ignored (ADR 0072): one per key and
/// offering node, kept for one silence time after the last such offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementContest {
    /// The resource key; 1 through 200 characters without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub resource: String,
    /// The node that offered it; 1 through 200 characters, not blank, without NUL. It may be one the pool does not
    /// know.
    #[schemars(length(min = 1, max = 200))]
    pub node: String,
    /// The store time of its last offer of the key while another live node held it.
    pub at: Timestamp,
}

/// A state or a message of a pool out of its bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidPlacement(String);

/// The state of a [`Placement`] pool: its silence time, node limit, line limit and holder limit, given at its create
/// and changed by `Configure`, where its notices go, the nodes it knows by node id, the next grant number, its holders
/// in the order they were granted, its waiting asks in line order, the contests of resource keys, and the time and id
/// of its armed check.
/// Build it with `PlacementState::new`; a decoded state is checked the same way.
///
/// On the wire: `{"silence":"PT1M","maxNodes":1000,"maxWaiting":100,"maxHolders":1000,"alerts":null,"nodes":{},
/// "nextGrant":1,"holders":[],"waiting":[],"contested":[],"armed":null,"armedId":null}`; `alerts`, `nodes`,
/// `nextGrant`, `holders`, `waiting`, `contested`, `armed` and `armedId` may be left out of a create.
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
    /// The offers of resource keys that another live node held, ignored (ADR 0072), in the order they were first
    /// recorded.
    pub contested: Vec<PlacementContest>,
    /// The time of the armed `Check`, set exactly while a node is live or a contest is listed.
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
    /// The nodes it knows, by node id (1 through 200 characters, not blank, without NUL), at most 1,000 (more than
    /// `maxNodes` after a lower limit); absent: none.
    #[serde(default)]
    nodes: BTreeMap<String, PlacementNode>,
    /// The number of the next grant, at least 1; it only goes up. Absent: 1.
    #[serde(default = "first_grant")]
    #[schemars(range(min = 1))]
    next_grant: u64,
    /// The holders, in the order they were granted, at most 1,000 (more than `maxHolders` after a lower limit), each on
    /// a known node, each holder id once among the holders and waiting asks; absent: none.
    #[serde(default)]
    holders: Vec<PlacementHolder>,
    /// The waiting asks, in line order, at most 1,000 (more than `maxWaiting` after a lower limit), each holder id once
    /// among the holders and waiting asks; absent: none.
    #[serde(default)]
    waiting: Vec<PlacementAsk>,
    /// The offers of resource keys that another live node held, at most 1,000 (more than `maxNodes` after a lower
    /// limit), each key and node once; absent: none.
    #[serde(default)]
    contested: Vec<PlacementContest>,
    /// The time of the armed `Check`: present exactly while a node is live or a contest is listed.
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
            contested: raw.contested,
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
    ///     "nextGrant": 1, "holders": [], "waiting": [], "contested": [], "armed": null, "armedId": null}));
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
            contested: Vec::new(),
            armed: None,
            armed_id: None,
        };
        state.check()?;
        Ok(state)
    }

    /// This state with the settings of a `Configure`: each one given replaces its value. Fails when none is given or
    /// the result fails the check of a decoded state.
    fn configured(
        &self,
        silence: Option<Duration>,
        max_nodes: Option<u32>,
        max_waiting: Option<u32>,
        max_holders: Option<u32>,
    ) -> Result<Self, InvalidPlacement> {
        if silence.is_none() && max_nodes.is_none() && max_waiting.is_none() && max_holders.is_none() {
            return Err(InvalidPlacement(
                "a Configure sets at least one of silence, maxNodes, maxWaiting and maxHolders".to_owned(),
            ));
        }
        let state = Self {
            silence: silence.unwrap_or(self.silence),
            max_nodes: max_nodes.unwrap_or(self.max_nodes),
            max_waiting: max_waiting.unwrap_or(self.max_waiting),
            max_holders: max_holders.unwrap_or(self.max_holders),
            ..self.clone()
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
        if self.nodes.len() > MAX_NODES as usize {
            return Err(InvalidPlacement(format!("at most {MAX_NODES} nodes are known, had {}", self.nodes.len())));
        }
        for (node, entry) in &self.nodes {
            check_id("a node", node)?;
            check_id("an instance", &entry.instance)?;
            check_labels(&entry.labels, "labels", "label")?;
            if !sorted_once(&entry.labels) {
                return Err(InvalidPlacement(format!("the labels of '{node}' must be sorted, each once")));
            }
            check_places(entry.places)?;
            if let Some(resource) = &entry.resource {
                check_resource(resource)?;
            }
            self.deadline(entry.heard_at).map_err(InvalidPlacement)?;
            if let Some(conflict) = &entry.conflict {
                if entry.status != PlacementNodeStatus::Live {
                    return Err(InvalidPlacement(format!("'{node}' is in conflict only while it is live")));
                }
                check_id("a conflict's instance", &conflict.instance)?;
                if let Some(host) = &conflict.host {
                    check_id("a conflict's host", host)?;
                } else if conflict.instance == entry.instance {
                    return Err(InvalidPlacement(format!(
                        "the conflict of '{node}' names its recorded instance without a host"
                    )));
                }
                self.deadline(conflict.at).map_err(InvalidPlacement)?;
            }
        }
        self.check_books()?;
        self.check_resources()?;
        let live = self.nodes.values().any(|n| n.status == PlacementNodeStatus::Live) || !self.contested.is_empty();
        if live != self.armed.is_some() || live != self.armed_id.is_some() {
            return Err(InvalidPlacement(
                "armed and armedId are set exactly while a node is live or a contest is listed".to_owned(),
            ));
        }
        Ok(())
    }

    /// Checks the resource keys: at most one live node holds each, and the contests are at most 1,000, each key and
    /// node once, in bounds.
    fn check_resources(&self) -> Result<(), InvalidPlacement> {
        let mut held = HashSet::new();
        let live = self.nodes.values().filter(|n| n.status == PlacementNodeStatus::Live);
        for resource in live.filter_map(|n| n.resource.as_deref()) {
            if !held.insert(resource) {
                return Err(InvalidPlacement(format!("two live nodes hold the resource key '{resource}'")));
            }
        }
        if self.contested.len() > MAX_NODES as usize {
            return Err(InvalidPlacement(format!(
                "at most {MAX_NODES} contests are listed, had {}",
                self.contested.len()
            )));
        }
        let mut seen = HashSet::new();
        for contest in &self.contested {
            check_resource(&contest.resource)?;
            check_id("a contest's node", &contest.node)?;
            self.deadline(contest.at).map_err(InvalidPlacement)?;
            if !seen.insert((contest.resource.as_str(), contest.node.as_str())) {
                return Err(InvalidPlacement(format!(
                    "the contest of '{}' by '{}' is listed twice",
                    contest.resource, contest.node
                )));
            }
        }
        Ok(())
    }

    /// The live node other than `node` that holds `resource`.
    fn holder_of(&self, resource: &str, node: &str) -> Option<&str> {
        let mut live = self.nodes.iter().filter(|(_, n)| n.status == PlacementNodeStatus::Live);
        live.find(|(id, n)| id.as_str() != node && n.resource.as_deref() == Some(resource)).map(|(id, _)| id.as_str())
    }

    /// Checks the holders and the line: at most 1,000 each, each holder id once, needs and message types in bounds,
    /// each holder on a known node with a grant below `nextGrant`.
    fn check_books(&self) -> Result<(), InvalidPlacement> {
        if self.next_grant == 0 {
            return Err(InvalidPlacement("nextGrant must be at least 1".to_owned()));
        }
        if self.holders.len() > MAX_BOOKS as usize {
            return Err(InvalidPlacement(format!("at most {MAX_BOOKS} hold a place, had {}", self.holders.len())));
        }
        if self.waiting.len() > MAX_BOOKS as usize {
            return Err(InvalidPlacement(format!("at most {MAX_BOOKS} asks wait, had {}", self.waiting.len())));
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

    /// The earliest end of a live node's silence or of a contest; none when no node is live and no contest listed.
    fn next_check(&self) -> Option<Timestamp> {
        let live = self.nodes.values().filter(|n| n.status == PlacementNodeStatus::Live).map(|n| n.heard_at);
        live.chain(self.contested.iter().map(|c| c.at)).filter_map(|at| self.deadline(at).ok()).min()
    }
}

/// Checks the fields of an offer and gives its labels sorted, each once.
fn check_offer(
    node: &str,
    instance: &str,
    mut labels: Vec<String>,
    places: u32,
    resource: Option<&str>,
) -> Result<Vec<String>, InvalidPlacement> {
    check_id("a node", node)?;
    check_id("an instance", instance)?;
    check_labels(&labels, "labels", "label")?;
    check_places(places)?;
    if let Some(resource) = resource {
        check_resource(resource)?;
    }
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

fn check_resource(resource: &str) -> Result<(), InvalidPlacement> {
    let length = resource.chars().count();
    if length == 0 || length > MAX_RESOURCE_LENGTH || resource.contains('\0') {
        return Err(InvalidPlacement(format!(
            "a resource key must be 1 through {MAX_RESOURCE_LENGTH} characters without NUL, had {length}"
        )));
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
/// at its create and changed by `Configure`; a create of a pool that exists changes nothing. It never stops itself,
/// and answers no query: read its state.
///
/// A `Configure` replaces the settings it names, within the bounds of the create. A limit lowered below what the pool
/// has removes nothing: the known nodes, the holders, the waiting asks and the contests stay, and new ones are refused
/// or queued as at the limit until the count is below it; the pool only serves under its limits. A larger holder
/// limit grants waiting asks in the same turn, and a larger node limit admits a new node at its next offer. A new
/// silence, with a check armed, arms the check at the new earliest end of a live node's silence or of a contest, due
/// at once when that has passed: a shorter silence marks the quiet nodes silent in the next turn, and conflict marks
/// and contests end one new silence time after their last offer or report. A silence at or below the members' offer
/// interval makes nodes flip between silent and back.
///
/// Each `Offer` replaces the node's entry and records the store time of its turn as `heardAt`, except an offer of
/// another instance than the recorded one while the node is live (ADR 0069): that turn changes no entry field, sets the
/// node's `conflict` to that instance and the store time, and, when the node had no mark, queues one `Alert` of kind
/// `conflict`; later conflicting offers, from any other instance, only update the mark. The mark ends when the node
/// goes silent, or when the recorded instance offers one silence time or more after the mark's time, as the other
/// machine stopped; a later conflict is told again. A silent node's entry is taken over by the next offer, whatever its
/// instance: a replaced machine. The pool only reports a conflict: two machines under one node id also share its leases
/// and its node row in the store, and a cloned disk with the member's identity copied looks like one machine to its
/// offers.
///
/// Such a machine is found by its member instead (ADR 0071), which reports `Duplicate` with the other machine's host
/// name: it marks a live node in `conflict` with that `host` (and the node's own instance for a new mark), and a new
/// mark queues one `conflict` alert. While its mark has a `host` the node gets no new places and keeps its holders. A
/// mark set only by reports ends at the first turn one silence time after the last report; one set by offers and
/// reports ends as an offer's mark does, counted from the later of the two. Going silent ends every mark.
///
/// An offer may name a resource key, such as a Docker daemon's id (ADR 0072), and one live node holds a key: the node
/// whose offer recorded it while no other live node had it. An offer whose key a different live node holds changes
/// nothing for the offering node: a new node is not added, and a known one keeps its last entry, so it goes silent if
/// it keeps offering only the held key. The pool lists the contest in `contested`, one entry per key and offering node
/// with the store time of the last such offer, and queues one `conflict` alert about the offering node when an entry is
/// added; the check removes an entry one silence time after its time, so a later contest is told again. The check runs
/// while a contest is listed, also when no node is live. When the holder goes silent or is gone, the next offer of the
/// key takes it, while the runs on the old holder may still use the resource for a while: a brief overbooking. Offers
/// without a key are not checked; the pool cannot tell that two different keys name one resource.
///
/// While a node is live or a contest is listed the pool has one `Check` armed, with the crate's shared retry policy, at
/// the earliest `heardAt` of a live node or `at` of a contest plus the silence, and keeps its time in `armed` and its
/// id in `armedId`. The armed check marks each live node whose silence ended as `silent` and queues one `Alert` to
/// itself for each; its next offer makes the node live and queues a `back` alert. Each `Alert` sends a
/// [`PlacementNotice`] to the target of the last `Alerts`, so a target that is gone or refuses the notice fails that
/// alert and nothing else; the node stays silent. The armed check run early, made due by an operator, marks nothing and
/// arms itself again at `armed`; any other `Check` the pool sent itself changes nothing, and one from another sender
/// (an operator's retry or redrive comes without one) is refused. A check that is purged, cancelled or failed for good
/// leaves the pool with a stale `armedId` and no timer until `Rearm`, which cancels it and arms a new one at `armed`.
///
/// Each `Ask` is answered in its own commit with a [`PlacementAnswer`], with the shared retry policy: `granted` with
/// the next grant number and the node, `queued`, or `refused` with its reason, so a target that is gone fails the ask
/// and nothing is recorded. A node's free places are its offered places minus its holders; an offer with fewer places,
/// or a silent node, keeps every holder and gets no new ones. After every message the pool grants waiting asks in line
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
/// An operator's `Drain` stops new places on a node, which keeps its holders and its offers: a draining node counts as
/// no free places, while the refusal of an ask no node could ever meet still counts it, so such an ask waits. An offer
/// keeps the drain; `Undrain` clears it and grants the waiting asks that fit. `Gone` of a silent node removes it and
/// frees its places, and tells each of its holders with a `gone` [`PlacementAnswer`] in a `TellGone` that the pool
/// sends itself, one per holder, so a target that is gone fails only its own answer. Waiting asks stay in the line. A
/// holder granted from the line can hear `gone` before its `granted` answer, whose `Notify` then finds no place and
/// sends nothing.
///
/// Silence is counted from the store time an offer was handled; due times are not run times, so a paused or slow pool
/// marks a node late, never before its silence ended. A pool hosted only on its member nodes runs no check once they
/// all go quiet. Bad input is refused for good with the state unchanged: a message out of bounds, a new node past
/// `maxNodes`, a `Configure` without a setting, a `Drain`, `Undrain` or `Gone` of an unknown node, a `Gone` of a live
/// node, a `Check`, `Alert`, `Notify` or `TellGone` from another sender, and a notice or answer the send refuses; a
/// read of the store that fails is tried again.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlacementActor;

impl Actor for PlacementActor {
    type Ref = Placement;
    type State = PlacementState;

    async fn handle(&self, turn: &mut Turn<Self>, message: PlacementMsg) -> HandlerResult {
        let before = targets(turn.state());
        end_reports(turn);
        match message {
            PlacementMsg::Offer { node, instance, labels, places, resource } => {
                let labels = check_offer(&node, &instance, labels, places, resource.as_deref())
                    .map_err(HandlerError::permanent)?;
                offer(turn, node, instance, labels, places, resource)?;
            }
            PlacementMsg::Duplicate { node, host } => {
                check_id("a node", &node).map_err(HandlerError::permanent)?;
                check_id("a host", &host).map_err(HandlerError::permanent)?;
                duplicate(turn, node, host)?;
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
            PlacementMsg::Configure { silence, max_nodes, max_waiting, max_holders } => {
                let configured = turn
                    .state()
                    .configured(silence, max_nodes, max_waiting, max_holders)
                    .map_err(HandlerError::permanent)?;
                configure(turn, configured)?;
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
            PlacementMsg::Drain { node } => drain(turn, &node, true)?,
            PlacementMsg::Undrain { node } => drain(turn, &node, false)?,
            PlacementMsg::Gone { node } => {
                check_id("a node", &node).map_err(HandlerError::permanent)?;
                declare_gone(turn, &node)?;
            }
            PlacementMsg::TellGone { holder, grant, node, to, message_type } => {
                check_holder(&holder).map_err(HandlerError::permanent)?;
                check_id("a node", &node).map_err(HandlerError::permanent)?;
                check_message_type(&message_type).map_err(HandlerError::permanent)?;
                refuse_foreign(turn, "TellGone")?;
                let answer = PlacementAnswer {
                    grant: Some(grant),
                    node: Some(node),
                    ..answer(turn, &holder, PlacementAnswerKind::Gone)
                };
                tell(turn, &to, &message_type, answer).await?;
            }
            PlacementMsg::Failed(failed) if turn.message().notification => undelivered(turn, &failed),
            PlacementMsg::Stopped(stopped) if turn.message().notification => went_away(turn, &stopped.actor),
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
fn went_away(turn: &mut Turn<PlacementActor>, actor: &ActorAddress) {
    if actor == turn.self_address() {
        return;
    }
    let state = turn.state_mut();
    state.holders.retain(|h| &h.to != actor);
    state.waiting.retain(|w| &w.to != actor);
}

/// Takes the settings of `configured`, a checked copy of the state; with another silence and a check armed, arms the
/// check at the new earliest end.
fn configure(turn: &mut Turn<PlacementActor>, configured: PlacementState) -> HandlerResult {
    let rearm = configured.silence != turn.state().silence && turn.state().armed.is_some();
    if configured != *turn.state() {
        *turn.state_mut() = configured;
    }
    if rearm && let Some(at) = turn.state().next_check() {
        arm(turn, at)?;
    }
    Ok(())
}

/// Sets or clears the drain of a known `node`; an unknown one is refused for good.
fn drain(turn: &mut Turn<PlacementActor>, node: &str, draining: bool) -> HandlerResult {
    check_id("a node", node).map_err(HandlerError::permanent)?;
    let Some(entry) = turn.state().nodes.get(node) else {
        return Err(HandlerError::permanent(format!("the pool does not know the node '{node}'")));
    };
    if entry.draining != draining {
        turn.state_mut().nodes.get_mut(node).expect("known").draining = draining;
    }
    Ok(())
}

/// Removes a silent `node` with its holders and sends the pool a `TellGone` for each, with the shared retry policy;
/// refused for good for a live or unknown node.
fn declare_gone(turn: &mut Turn<PlacementActor>, node: &str) -> HandlerResult {
    match turn.state().nodes.get(node).map(|n| n.status) {
        None => return Err(HandlerError::permanent(format!("the pool does not know the node '{node}'"))),
        Some(PlacementNodeStatus::Live) => {
            return Err(HandlerError::permanent(format!("the node '{node}' is live: it is heard from, so not gone")));
        }
        Some(PlacementNodeStatus::Silent) => {}
    }
    let state = turn.state_mut();
    state.nodes.remove(node);
    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut state.holders).into_iter().partition(|h| h.node == node);
    state.holders = kept;
    for PlacementHolder { holder, to, message_type, grant, node, .. } in gone {
        let mut options = SendOptions::new().retry(retry_policy());
        options.correlation_id = turn.message().correlation_id.clone();
        turn.send_self_with(PlacementMsg::TellGone { holder, grant, node, to, message_type }, options)?;
    }
    Ok(())
}

/// The free places of each live node that is not draining nor reported duplicate: its offered places minus its holders.
fn free_places(state: &PlacementState) -> BTreeMap<&str, u32> {
    let mut free: BTreeMap<&str, u32> = state
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.status == PlacementNodeStatus::Live && !n.draining && n.conflict.as_ref().is_none_or(|c| c.host.is_none())
        })
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

/// The node for `needs` now: of the live nodes, not draining, with every need and a free place, the one with the most
/// free places, ties to the smaller node id.
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

/// Replaces the entry of `node`, tells the target of a node that is back, and arms the check when none is armed; for
/// another instance under a live node, marks the conflict instead, with an alert for a new mark.
fn offer(
    turn: &mut Turn<PlacementActor>,
    node: String,
    instance: String,
    labels: Vec<String>,
    places: u32,
    resource: Option<String>,
) -> HandlerResult {
    let now = turn.now();
    let state = turn.state();
    let due = state.deadline(now).map_err(HandlerError::permanent)?;
    let existing = state.nodes.get(&node);
    let previous = existing.map(|n| n.status);
    let draining = existing.is_some_and(|n| n.draining);
    let live = existing.filter(|n| n.status == PlacementNodeStatus::Live);
    if let Some(entry) = live.filter(|n| n.instance != instance) {
        let new = entry.conflict.is_none();
        let entry = turn.state_mut().nodes.get_mut(&node).expect("known");
        let host = entry.conflict.take().and_then(|c| c.host);
        entry.conflict = Some(PlacementConflict { instance, at: now, host });
        if new {
            queue_alert(turn, node, PlacementNoticeKind::Conflict)?;
        }
        return Ok(());
    }
    if let Some(resource) = resource.as_deref()
        && state.holder_of(resource, &node).is_some()
    {
        return contest(turn, resource, node);
    }
    // The mark lasts until the other instance was not heard from for one silence time.
    let conflict = live.and_then(|n| n.conflict.clone()).filter(|c| state.deadline(c.at).is_ok_and(|end| now < end));
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
    let status = PlacementNodeStatus::Live;
    let entry = PlacementNode { instance, labels, places, heard_at: now, status, draining, resource, conflict };
    turn.state_mut().nodes.insert(node, entry);
    Ok(())
}

/// Records that `node` offered `resource`, which another live node holds, at the store time, with an alert for a new
/// entry; refused for good when a new entry would pass `maxNodes`.
fn contest(turn: &mut Turn<PlacementActor>, resource: &str, node: String) -> HandlerResult {
    let now = turn.now();
    let state = turn.state_mut();
    if let Some(contest) = state.contested.iter_mut().find(|c| c.resource == resource && c.node == node) {
        contest.at = now;
        return Ok(());
    }
    if state.contested.len() >= state.max_nodes as usize {
        return Err(HandlerError::permanent(format!(
            "the pool lists maxNodes ({}) contests: the contest of '{resource}' by '{node}' is refused",
            state.max_nodes
        )));
    }
    state.contested.push(PlacementContest { resource: resource.to_owned(), node: node.clone(), at: now });
    Ok(queue_alert(turn, node, PlacementNoticeKind::Conflict)?)
}

/// Marks a live `node` in conflict with `host`, with an alert for a new mark; nothing for a silent node, and refused
/// for good for an unknown one.
fn duplicate(turn: &mut Turn<PlacementActor>, node: String, host: String) -> HandlerResult {
    let now = turn.now();
    let Some(entry) = turn.state_mut().nodes.get_mut(&node) else {
        return Err(HandlerError::permanent(format!("the pool does not know the node '{node}'")));
    };
    if entry.status != PlacementNodeStatus::Live {
        return Ok(());
    }
    let new = entry.conflict.is_none();
    let instance = entry.conflict.take().map_or_else(|| entry.instance.clone(), |c| c.instance);
    entry.conflict = Some(PlacementConflict { instance, at: now, host: Some(host) });
    if new {
        queue_alert(turn, node, PlacementNoticeKind::Conflict)?;
    }
    Ok(())
}

/// Ends each mark set only by reports (its instance is the node's own) one silence time after the last report.
fn end_reports(turn: &mut Turn<PlacementActor>) {
    let now = turn.now();
    let state = turn.state();
    let ended: Vec<String> = state
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.conflict.as_ref().is_some_and(|c| {
                c.host.is_some() && c.instance == n.instance && state.deadline(c.at).is_ok_and(|end| end <= now)
            })
        })
        .map(|(node, _)| node.clone())
        .collect();
    for node in ended {
        turn.state_mut().nodes.get_mut(&node).expect("known").conflict = None;
    }
}

/// Marks each live node whose silence ended as silent with an alert for each, removes each contest one silence time
/// after its last offer, and arms the next check; for the armed check run early, arms it again at `armed`; for any
/// other check, nothing.
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
            entry.conflict = None;
        }
        queue_alert(turn, node, PlacementNoticeKind::Silent)?;
    }
    let state = turn.state_mut();
    let silence = state.silence;
    state.contested.retain(|c| time::add(c.at, silence).is_ok_and(|end| now < end));
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
        let three = serde_json::json!({"nodes": {"a": node("silent"), "b": node("silent"), "c": node("silent")}});
        assert!(decode(state(three)).is_ok(), "more nodes than maxNodes is a state after a lower limit");
        let too_many: serde_json::Map<String, serde_json::Value> =
            (0..=MAX_NODES).map(|n| (format!("n{n}"), node("silent"))).collect();
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
        let mut empty_key = node("silent");
        empty_key["resource"] = serde_json::json!("");
        let mut long_key = node("silent");
        long_key["resource"] = serde_json::json!("k".repeat(201));
        let long_node = "n".repeat(201);
        for (extra, why) in [
            (serde_json::json!({"silence": "PT0.5S"}), "silence must be"),
            (serde_json::json!({"silence": "P2D"}), "silence must be"),
            (serde_json::json!({"maxNodes": 0}), "maxNodes must be"),
            (serde_json::json!({"maxNodes": 1001}), "maxNodes must be"),
            (serde_json::json!({"alerts": {"to": "ops/_", "messageType": ""}}), "messageType must be"),
            (serde_json::json!({"nodes": too_many}), "at most 1000 nodes"),
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
            (serde_json::json!({"nodes": {"a": empty_key}}), "a resource key must be"),
            (serde_json::json!({"nodes": {"a": long_key}}), "a resource key must be"),
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

    #[test]
    fn a_decoded_conflict_is_allowed_only_on_a_live_node_and_names_another_instance() {
        let with = |status: &str, conflict: serde_json::Value| {
            let mut entry = node(status);
            entry["conflict"] = conflict;
            let armed = if status == "live" { serde_json::json!(AT) } else { serde_json::Value::Null };
            let armed_id = if status == "live" { serde_json::json!("c1") } else { serde_json::Value::Null };
            state(serde_json::json!({"nodes": {"a": entry}, "armed": armed, "armedId": armed_id}))
        };
        let decoded = decode(with("live", serde_json::json!({"instance": "i2", "at": AT}))).unwrap();
        let at: Timestamp = AT.parse().unwrap();
        assert_eq!(decoded.nodes["a"].conflict, Some(PlacementConflict { instance: "i2".into(), at, host: None }));
        let reported = decode(with("live", serde_json::json!({"instance": "i1", "at": AT, "host": "mac-b"}))).unwrap();
        let host = Some("mac-b".to_owned());
        assert_eq!(reported.nodes["a"].conflict, Some(PlacementConflict { instance: "i1".into(), at, host }));
        let entry = serde_json::to_value(&reported.nodes["a"]).unwrap();
        assert_eq!(entry["conflict"], serde_json::json!({"instance": "i1", "at": AT, "host": "mac-b"}));
        assert_eq!(decode(with("live", serde_json::Value::Null)).unwrap().nodes["a"].conflict, None);
        let entry = serde_json::to_value(&decoded.nodes["a"]).unwrap();
        assert_eq!(entry["conflict"], serde_json::json!({"instance": "i2", "at": AT}));
        let plain = serde_json::to_value(decode(with("live", serde_json::Value::Null)).unwrap().nodes["a"].clone());
        assert!(plain.unwrap().get("conflict").is_none(), "no mark: not written");
        for (status, conflict, why) in [
            ("silent", serde_json::json!({"instance": "i2", "at": AT}), "in conflict only while it is live"),
            ("live", serde_json::json!({"instance": "i1", "at": AT}), "names its recorded instance without a host"),
            ("live", serde_json::json!({"instance": "i1", "at": AT, "host": " "}), "a conflict's host id must be"),
            (
                "silent",
                serde_json::json!({"instance": "i1", "at": AT, "host": "b"}),
                "in conflict only while it is live",
            ),
            ("live", serde_json::json!({"instance": " ", "at": AT}), "a conflict's instance id must be"),
            ("live", serde_json::json!({"instance": "i2", "at": "9999-12-31T23:59:30Z"}), "after 9999-12-31"),
        ] {
            let decoded = decode(with(status, conflict.clone()));
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {conflict}");
        }
    }

    #[test]
    fn a_decoded_state_with_two_live_holders_of_one_key_or_a_bad_contest_is_refused() {
        let keyed = |status: &str, key: &str| {
            let mut entry = node(status);
            entry["resource"] = serde_json::json!(key);
            entry
        };
        let contest = |key: &str, node: &str| serde_json::json!({"resource": key, "node": node, "at": AT});
        let armed = serde_json::json!({"armed": AT, "armedId": "c1"});
        let with = |extra: serde_json::Value| {
            let mut json = state(armed.clone());
            json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            json
        };
        let held = decode(with(serde_json::json!({"nodes": {"a": keyed("live", "d1"), "b": keyed("silent", "d1")},
            "contested": [contest("d1", "b"), contest("d1", "c")]})))
        .unwrap();
        let at: Timestamp = AT.parse().unwrap();
        let first = PlacementContest { resource: "d1".into(), node: "b".into(), at };
        assert_eq!(held.contested[0], first);
        assert_eq!(held.nodes["a"].resource.as_deref(), Some("d1"));
        let only = decode(state(serde_json::json!({"contested": [contest("d1", "c")], "armed": AT, "armedId": "c1"})));
        assert!(only.is_ok(), "a contest keeps the check armed without a live node");
        let two_keys = with(serde_json::json!({"nodes": {"a": keyed("live", "d1"), "b": keyed("live", "d2")}}));
        assert!(decode(two_keys).is_ok());
        let three = [contest("d1", "a"), contest("d1", "b"), contest("d1", "c")];
        assert!(decode(with(serde_json::json!({"contested": three}))).is_ok(), "more contests than maxNodes");
        let too_many: Vec<serde_json::Value> = (0..=MAX_NODES).map(|n| contest("d1", &format!("n{n}"))).collect();
        let mut late = contest("d1", "c");
        late["at"] = serde_json::json!("9999-12-31T23:59:30Z");
        for (extra, why) in [
            (
                serde_json::json!({"nodes": {"a": keyed("live", "d1"), "b": keyed("live", "d1")}}),
                "two live nodes hold the resource key 'd1'",
            ),
            (serde_json::json!({"contested": too_many}), "at most 1000 contests"),
            (serde_json::json!({"contested": [contest("d1", "c"), contest("d1", "c")]}), "listed twice"),
            (serde_json::json!({"contested": [contest("", "c")]}), "a resource key must be"),
            (serde_json::json!({"contested": [contest("d1", " ")]}), "a contest's node id must be"),
            (serde_json::json!({"contested": [late]}), "after 9999-12-31"),
            (
                serde_json::json!({"contested": [contest("d1", "c")], "armed": null, "armedId": null}),
                "armed and armedId",
            ),
        ] {
            let json = with(extra);
            let decoded = decode(json.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {json}");
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
        let over = books(serde_json::json!({"nextGrant": 9, "holders": [holder("h1", 1, "a"), holder("h2", 2, "a"),
            holder("h3", 3, "a")], "waiting": [waiting("w1"), waiting("w2"), waiting("w3")]}));
        assert!(decode(over).is_ok(), "more holders and waiting asks than their limits is a state after lower limits");
        let next_grant = u64::from(MAX_BOOKS) + 2;
        let many_holders: Vec<_> = (1..=u64::from(MAX_BOOKS) + 1).map(|g| holder(&format!("h{g}"), g, "a")).collect();
        let many_waiting: Vec<_> = (0..=MAX_BOOKS).map(|n| waiting(&format!("w{n}"))).collect();
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
            (serde_json::json!({"nextGrant": next_grant, "holders": many_holders}), "at most 1000 hold a place"),
            (serde_json::json!({"waiting": many_waiting}), "at most 1000 asks wait"),
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
    fn configured_replaces_the_given_settings_within_the_bounds_of_the_create() {
        let minute = Duration::from_secs(60);
        let pool = PlacementState::new(minute, 2, 3, 4).unwrap();
        let both = pool.configured(None, None, None, Some(8)).unwrap();
        assert_eq!((both.silence, both.max_nodes, both.max_waiting, both.max_holders), (minute, 2, 3, 8));
        let all = pool.configured(Some(MIN_SILENCE), Some(1), Some(0), Some(1)).unwrap();
        assert_eq!((all.silence, all.max_nodes, all.max_waiting, all.max_holders), (MIN_SILENCE, 1, 0, 1));
        let top = pool.configured(Some(MAX_SILENCE), Some(MAX_NODES), Some(MAX_BOOKS), Some(MAX_BOOKS)).unwrap();
        assert_eq!((top.silence, top.max_nodes, top.max_waiting, top.max_holders), (MAX_SILENCE, 1000, 1000, 1000));
        let short = MIN_SILENCE - Duration::from_nanos(1);
        let long = MAX_SILENCE + Duration::from_nanos(1);
        for (silence, max_nodes, max_waiting, max_holders, why) in [
            (None, None, None, None, "sets at least one of"),
            (Some(short), None, None, None, "silence must be"),
            (Some(long), None, None, None, "silence must be"),
            (None, Some(0), None, None, "maxNodes must be"),
            (None, Some(1001), None, None, "maxNodes must be"),
            (None, None, Some(1001), None, "maxWaiting must be"),
            (None, None, None, Some(0), "maxHolders must be"),
            (None, None, None, Some(1001), "maxHolders must be"),
        ] {
            let built = pool.configured(silence, max_nodes, max_waiting, max_holders);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
        }
        let mut late = decode(state(serde_json::json!({"nodes": {"a": node("silent")}}))).unwrap();
        late.nodes.get_mut("a").unwrap().heard_at = "9999-12-31T23:58:30Z".parse().unwrap();
        assert!(late.configured(None, None, None, Some(1)).is_ok());
        let longer = late.configured(Some(Duration::from_secs(120)), None, None, None);
        assert!(longer.is_err_and(|e| e.to_string().contains("after 9999-12-31")), "a silence must stay representable");
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
                places: 0,
                resource: None
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
        let keyed = |key: &str| PlacementMsg::offer_of("n".into(), "i".into(), vec![], 1, Some(key.into()));
        assert!(keyed(" ").is_ok() && keyed(&"k".repeat(200)).is_ok(), "a blank key is allowed");
        for key in ["".to_owned(), "k".repeat(201), "a\0".to_owned()] {
            let built = keyed(&key);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains("a resource key must be")), "{built:?}");
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
