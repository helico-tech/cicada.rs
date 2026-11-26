//! The placement pool (ADR 0067): an actor to which nodes offer places, each with a set of labels, renewed at an
//! interval; a node not heard from for the pool's silence time is marked silent, and its alert target is told once.

use std::collections::BTreeMap;
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::limits::MAX_TYPE_LENGTH;
use cicada::time::{self, MAX_INSTANT, Timestamp, format_duration, serde_duration};
use cicada::turn::TurnError;
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

/// The messages of a [`Placement`] pool. `Check` and `Alert` are its own: one from anyone else is refused for good. A
/// node id or instance id is 1 through 200 characters, not blank, without NUL; a message out of bounds is refused for
/// good with the state unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

/// The state of a [`Placement`] pool: its silence time and node limit, fixed at its create, where its notices go, the
/// nodes it knows by node id, and the time and id of its armed check. Build it with `PlacementState::new`; a decoded
/// state is checked the same way.
///
/// On the wire: `{"silence":"PT1M","maxNodes":1000,"alerts":null,"nodes":{},"armed":null,"armedId":null}`; `alerts`,
/// `nodes`, `armed` and `armedId` may be left out of a create.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct PlacementState {
    /// How long a node may go without an offer before it is silent.
    #[serde(serialize_with = "serde_duration::serialize")]
    pub silence: Duration,
    /// The most nodes the pool knows.
    pub max_nodes: u32,
    /// Where its notices go; none: no notices.
    pub alerts: Option<PlacementAlerts>,
    /// The nodes it knows, by node id.
    pub nodes: BTreeMap<String, PlacementNode>,
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
    /// Where its notices go; absent or null: no notices.
    #[serde(default)]
    alerts: Option<PlacementAlerts>,
    /// The nodes it knows, by node id (1 through 200 characters, not blank, without NUL), at most `maxNodes`; absent:
    /// none.
    #[serde(default)]
    nodes: BTreeMap<String, PlacementNode>,
    /// The time of the armed `Check`: present exactly while a node is live.
    #[serde(default)]
    armed: Option<Timestamp>,
    /// The id of the armed `Check`: present exactly with `armed`.
    #[serde(default)]
    armed_id: Option<MessageId>,
}

impl TryFrom<RawState> for PlacementState {
    type Error = InvalidPlacement;

    fn try_from(raw: RawState) -> Result<Self, InvalidPlacement> {
        let state = Self {
            silence: raw.silence,
            max_nodes: raw.max_nodes,
            alerts: raw.alerts,
            nodes: raw.nodes,
            armed: raw.armed,
            armed_id: raw.armed_id,
        };
        state.check()?;
        Ok(state)
    }
}

impl PlacementState {
    /// A pool that marks a node silent after `silence` without an offer and knows at most `max_nodes` nodes, with no
    /// alert target and no nodes. Fails unless the silence is 1 second through 1 day and the node limit 1 through
    /// 1,000.
    ///
    /// ```
    /// use std::time::Duration;
    /// use cicada_actors::PlacementState;
    ///
    /// let state = PlacementState::new(Duration::from_secs(60), 1000)?;
    /// assert_eq!(serde_json::to_value(&state)?, serde_json::json!({
    ///     "silence": "PT1M", "maxNodes": 1000, "alerts": null, "nodes": {}, "armed": null, "armedId": null}));
    /// assert!(PlacementState::new(Duration::from_secs(60), 1001).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(silence: Duration, max_nodes: u32) -> Result<Self, InvalidPlacement> {
        let state = Self { silence, max_nodes, alerts: None, nodes: BTreeMap::new(), armed: None, armed_id: None };
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
            check_labels(&entry.labels)?;
            if entry.labels.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(InvalidPlacement(format!("the labels of '{node}' must be sorted, each once")));
            }
            check_places(entry.places)?;
            self.deadline(entry.heard_at).map_err(InvalidPlacement)?;
        }
        let live = self.nodes.values().any(|n| n.status == PlacementNodeStatus::Live);
        if live != self.armed.is_some() || live != self.armed_id.is_some() {
            return Err(InvalidPlacement("armed and armedId are set exactly while a node is live".to_owned()));
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
    check_labels(&labels)?;
    check_places(places)?;
    labels.sort();
    labels.dedup();
    Ok(labels)
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

fn check_labels(labels: &[String]) -> Result<(), InvalidPlacement> {
    if labels.len() > MAX_LABELS {
        return Err(InvalidPlacement(format!("at most {MAX_LABELS} labels, had {}", labels.len())));
    }
    for label in labels {
        let length = label.chars().count();
        if length == 0 || length > MAX_LABEL_LENGTH || label.contains('\0') {
            return Err(InvalidPlacement(format!(
                "a label must be 1 through {MAX_LABEL_LENGTH} characters without NUL, had {length}"
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

/// The placement pool type. It has no settings: the silence time and the node limit are in the state of each pool,
/// given at its create; a create of a pool that exists changes nothing. It never stops itself, and answers no query:
/// read its state.
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
/// Silence is counted from the store time an offer was handled; due times are not run times, so a paused or slow pool
/// marks a node late, never before its silence ended. A pool hosted only on its member nodes runs no check once they
/// all go quiet. Bad input is refused for good with the state unchanged: a message out of bounds, a new node past
/// `maxNodes`, a `Check` or `Alert` from another sender, and a notice the send refuses; a read of the store that fails
/// is tried again.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlacementActor;

impl Actor for PlacementActor {
    type Ref = Placement;
    type State = PlacementState;

    async fn handle(&self, turn: &mut Turn<Self>, message: PlacementMsg) -> HandlerResult {
        match message {
            PlacementMsg::Offer { node, instance, labels, places } => {
                let labels = check_offer(&node, &instance, labels, places).map_err(HandlerError::permanent)?;
                offer(turn, node, instance, labels, places)
            }
            PlacementMsg::Check => {
                refuse_foreign(turn, "Check")?;
                check(turn)
            }
            PlacementMsg::Rearm => {
                if let Some(at) = turn.state().armed {
                    arm(turn, at)?;
                }
                Ok(())
            }
            PlacementMsg::Alert { node, kind } => {
                check_id("a node", &node).map_err(HandlerError::permanent)?;
                refuse_foreign(turn, "Alert")?;
                alert(turn, node, kind).await
            }
            PlacementMsg::Alerts { to, message_type } => {
                check_message_type(&message_type).map_err(HandlerError::permanent)?;
                turn.state_mut().alerts = Some(PlacementAlerts { to, message_type });
                Ok(())
            }
        }
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
        let mut json = serde_json::json!({"silence": "PT1M", "maxNodes": 2});
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    }

    fn decode(json: serde_json::Value) -> Result<PlacementState, serde_json::Error> {
        serde_json::from_value(json)
    }

    #[test]
    fn the_constructor_takes_a_silence_of_1_second_through_1_day_and_1_through_1000_nodes() {
        let minute = Duration::from_secs(60);
        let state = PlacementState::new(minute, 2).unwrap();
        assert_eq!((state.silence, state.max_nodes, state.alerts, state.armed), (minute, 2, None, None));
        assert!(state.nodes.is_empty());
        assert!(PlacementState::new(MIN_SILENCE, 1).is_ok());
        assert!(PlacementState::new(MAX_SILENCE, MAX_NODES).is_ok());
        let short = MIN_SILENCE - Duration::from_nanos(1);
        let long = MAX_SILENCE + Duration::from_nanos(1);
        for (silence, max_nodes, why) in [
            (short, 1, "silence must be"),
            (long, 1, "silence must be"),
            (Duration::ZERO, 1, "silence must be"),
            (minute, 0, "maxNodes must be"),
            (minute, 1001, "maxNodes must be"),
        ] {
            let built = PlacementState::new(silence, max_nodes);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
        }
    }

    #[test]
    fn a_decoded_state_is_checked_like_a_built_one() {
        let decoded = decode(state(serde_json::json!({"other": 1}))).unwrap();
        assert_eq!(decoded, PlacementState::new(Duration::from_secs(60), 2).unwrap(), "unknown fields are ignored");
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
        for missing in ["silence", "maxNodes"] {
            let mut json = state(serde_json::json!({}));
            json.as_object_mut().unwrap().remove(missing);
            assert!(decode(json).is_err(), "{missing} is required");
        }
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
