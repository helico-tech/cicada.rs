//! The node-down monitor (ADR 0054): a singleton that reads the node table every ten seconds and publishes one
//! [`NodeDown`] through a [`Topic`] for each observed silence of a node row of two minutes or more.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::time::Duration;

use cicada::OpError;
use cicada::prelude::*;
use cicada::runtime::NodeInfo;
use cicada::runtime::limits::MAX_ID_LENGTH;
use cicada::store::NodeId;
use cicada::time::{self, MAX_INSTANT, Timestamp};
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};

use crate::supervisor::retry_policy;
use crate::{StoredMessage, Topic, TopicMsg};

/// How often the monitor reads the node table: its next `Poll` is due this long after a check ends.
pub const POLL_INTERVAL: Duration = Duration::from_secs(10);
/// The longest a read of the node table may take; a slower read counts as the problem `timeout`.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// The most node rows a sample may hold; a larger one counts as the problem `tooManyNodes`. It also bounds `alerted`.
pub const MAX_NODES: usize = 1000;
/// The silence after which a node row is dead: the threshold of the node view (`GET /nodes`).
pub const DEAD_AFTER: Duration = Duration::from_secs(120);

/// The node-down monitor, `cicada.node-down/_`: one for each store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeDownMonitor;

impl ActorRef for NodeDownMonitor {
    const TYPE: &'static str = "cicada.node-down";
    type Message = NodeDownMonitorMsg;
}

/// The messages of a [`NodeDownMonitor`]. `Poll` is its own: it counts only when the monitor sent it itself and it is
/// the armed timer; otherwise it ends `done` and changes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum NodeDownMonitorMsg {
    /// Starts the monitor, or arms its timer again after it was purged, cancelled or failed for good: cancels the
    /// armed `Poll` and arms one at `nextPollAt`, due at once when that has passed or was never set. Keeps `alerted`, so
    /// it never repeats an alert by itself.
    Rearm,
    /// The timer: sent by the monitor to itself, with a retry policy. Before `nextPollAt` it arms itself again at that
    /// time; at or after it, it reads the node table, publishes what is new and arms the next `Poll`.
    Poll,
}

/// Why the last check read no usable sample of the node table. Only the category is kept, never an error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum NodeReadProblem {
    /// The source failed: the store could not be read.
    Unavailable,
    /// The read took longer than five seconds.
    Timeout,
    /// The sample was refused: a node id twice, an id or detail out of bounds, times out of order or past
    /// `9999-12-31`, a sample time before the last one, or a `lastSeen` before the alerted one of the same row.
    Invalid,
    /// The sample held more than 1000 node rows.
    TooManyNodes,
}

/// The row timestamps of a node that was alerted: the same pair seen dead again alerts no more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeDownWatermark {
    /// Store time of the row's first heartbeat.
    pub started_at: Timestamp,
    /// Store time of the row's latest heartbeat.
    pub last_seen: Timestamp,
}

/// A state of a node-down monitor that is out of its bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidNodeDown(String);

/// The state of a [`NodeDownMonitor`]: its topic, fixed at its create, the nodes it alerted, its timer and the result
/// of its last check. Build it with `NodeDownMonitorState::new`; a decoded state is checked the same way.
///
/// On the wire: `{"topic":{"name":"ops"}}`, with `alerted`, `armed`, `nextPollAt`, `lastCheckedAt` and `lastProblem`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct NodeDownMonitorState {
    /// The topic each [`NodeDown`] is published to; its subscribers get it.
    #[schemars(with = "RawTopic")]
    pub topic: Topic,
    /// The dead node rows alerted and still dead at the last successful check, by node id; at most 1000.
    pub alerted: BTreeMap<NodeId, NodeDownWatermark>,
    /// The id of the armed `Poll`, set exactly when `nextPollAt` is. It stays set when the timer goes without running.
    pub armed: Option<MessageId>,
    /// When the armed `Poll` is due.
    pub next_poll_at: Option<Timestamp>,
    /// The sample time of the last successful check.
    pub last_checked_at: Option<Timestamp>,
    /// Why the last check failed; cleared by a successful one.
    pub last_problem: Option<NodeReadProblem>,
}

/// A topic in the state, as its ref: `cicada.topic/<name>`.
#[derive(Deserialize, JsonSchema)]
struct RawTopic {
    /// The name of the topic.
    name: String,
}

/// The wire form of a [`NodeDownMonitorState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RawState {
    /// The topic each `NodeDown` is published to: `cicada.topic/<name>`.
    topic: RawTopic,
    /// The dead node rows alerted and still dead at the last successful check, by node id: the `startedAt` and
    /// `lastSeen` of each; at most 1000, absent: none.
    #[serde(default)]
    alerted: BTreeMap<NodeId, NodeDownWatermark>,
    /// The id of the armed `Poll`: present exactly when `nextPollAt` is.
    #[serde(default)]
    armed: Option<MessageId>,
    /// When the armed `Poll` is due: present exactly when `armed` is.
    #[serde(default)]
    next_poll_at: Option<Timestamp>,
    /// The sample time of the last successful check; absent or null before the first.
    #[serde(default)]
    last_checked_at: Option<Timestamp>,
    /// `unavailable`, `timeout`, `invalid` or `tooManyNodes`: why the last check failed; absent or null after a
    /// successful one.
    #[serde(default)]
    last_problem: Option<NodeReadProblem>,
}

impl TryFrom<RawState> for NodeDownMonitorState {
    type Error = InvalidNodeDown;

    fn try_from(raw: RawState) -> Result<Self, InvalidNodeDown> {
        let state = Self {
            topic: Topic::named(raw.topic.name),
            alerted: raw.alerted,
            armed: raw.armed,
            next_poll_at: raw.next_poll_at,
            last_checked_at: raw.last_checked_at,
            last_problem: raw.last_problem,
        };
        state.check()?;
        Ok(state)
    }
}

impl NodeDownMonitorState {
    /// A monitor that publishes to `topic`, not started: send it `Rearm` after the create. Fails for a topic name that
    /// makes no address.
    ///
    /// ```
    /// use cicada_actors::{NodeDownMonitorState, Topic};
    ///
    /// let state = NodeDownMonitorState::new(Topic::named("ops"))?;
    /// assert_eq!(serde_json::to_value(&state)?["topic"], serde_json::json!({"name": "ops"}));
    /// assert!(state.alerted.is_empty() && state.armed.is_none() && state.last_problem.is_none());
    /// assert!(NodeDownMonitorState::new(Topic::named("")).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(topic: Topic) -> Result<Self, InvalidNodeDown> {
        let state = Self {
            topic,
            alerted: BTreeMap::new(),
            armed: None,
            next_poll_at: None,
            last_checked_at: None,
            last_problem: None,
        };
        state.check()?;
        Ok(state)
    }

    fn check(&self) -> Result<(), InvalidNodeDown> {
        self.topic.address().map_err(|e| InvalidNodeDown(format!("the topic makes no address: {e}")))?;
        if self.alerted.len() > MAX_NODES {
            return Err(InvalidNodeDown(format!(
                "alerted holds at most {MAX_NODES} nodes, held {}",
                self.alerted.len()
            )));
        }
        for (node, mark) in &self.alerted {
            check_id(node).map_err(InvalidNodeDown)?;
            if mark.started_at > mark.last_seen || mark.last_seen > MAX_INSTANT {
                return Err(InvalidNodeDown(format!(
                    "the watermark of {node} needs startedAt <= lastSeen <= 9999-12-31"
                )));
            }
        }
        if self.armed.is_some() != self.next_poll_at.is_some() {
            return Err(InvalidNodeDown("armed and nextPollAt are set together or not at all".to_owned()));
        }
        for at in [self.next_poll_at, self.last_checked_at].into_iter().flatten() {
            if at > MAX_INSTANT {
                return Err(InvalidNodeDown(format!("{} is after 9999-12-31", time::format_instant(at))));
            }
        }
        Ok(())
    }
}

/// A node row that was silent for two minutes or more when the monitor read it: a sampled observation, not proof that
/// the machine failed. Consumers declare it in their family as `NodeDown(cicada_actors::NodeDown)`.
///
/// On the wire, as the payload of the message `NodeDown`: `{"node":"n1","startedAt":…,"lastSeen":…,"detectedAt":…}`,
/// with `hostname`, `cicadaVersion` and `label` when the row has them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeDown {
    /// The node id of the row.
    pub node: NodeId,
    /// Store time of the row's first heartbeat. Not a unique incarnation: two nodes with one id share a row.
    pub started_at: Timestamp,
    /// Store time of the row's latest heartbeat in the sample; it may be stale by the time the alert is handled.
    pub last_seen: Timestamp,
    /// Store time of the sample that saw the row dead, not the time the node died.
    pub detected_at: Timestamp,
    /// The OS host name of the row, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// The core Cicada version of the row, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cicada_version: Option<String>,
    /// The configured label of the row, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The family the monitor publishes: one message `NodeDown`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Alert {
    /// A node row was seen dead.
    NodeDown(NodeDown),
}

/// One sample of the node table: its rows, read at the store time `as_of` or before. Not atomic with the heartbeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSnapshot {
    /// The store time the sample counts silence against.
    pub as_of: Timestamp,
    /// The node rows; their `status` is ignored and computed again against `as_of`.
    pub nodes: Vec<NodeInfo>,
}

/// A read of the node table that failed; it prints why. The monitor keeps only the category `unavailable`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NodeReadError(pub String);

impl From<OpError> for NodeReadError {
    fn from(error: OpError) -> Self {
        Self(error.to_string())
    }
}

/// Where the monitor reads the node table: a read-only view of the store. Every host of the monitor must read the same
/// store as the one the monitor lives in.
pub trait NodeSnapshotSource: Send + Sync + 'static {
    /// One sample of every node row.
    fn snapshot(&self) -> impl Future<Output = Result<NodeSnapshot, NodeReadError>> + Send;
}

/// Reads [`ActorSystem::nodes`], then [`ActorSystem::now`] as the sample time. A system that is built but never
/// started reads the table without a row or heartbeat of its own: build one on the same store for the monitor.
impl NodeSnapshotSource for ActorSystem {
    async fn snapshot(&self) -> Result<NodeSnapshot, NodeReadError> {
        let nodes = self.nodes().await?;
        let as_of = self.now().await?;
        Ok(NodeSnapshot { as_of, nodes })
    }
}

/// The node-down monitor type, with the source it reads. Host it with the source and create the monitor unpinned with
/// a [`NodeDownMonitorState`], then send it `Rearm`; it never stops itself.
///
/// Each due `Poll` reads the source, bounded to five seconds, and checks the whole sample. A row whose `lastSeen` is
/// two minutes or more before the sample time is dead. For each dead row whose `startedAt` and `lastSeen` are not its
/// watermark in `alerted`, it publishes one [`NodeDown`] to the topic (with the crate's shared retry policy) and keeps
/// the pair, so a row seen dead from the start alerts too, and a newer heartbeat or a new row can alert again. Rows
/// that are absent or not dead leave `alerted`, without any recovery message. The publishes, `alerted`, the sample
/// time in `lastCheckedAt` and the next `Poll`, ten seconds after the end of the check, are one commit; competing hosts
/// and a turn replayed after a lost commit share the one lease of the singleton, so an observed pair is published once.
///
/// A failed, slow, invalid or oversized read publishes nothing, keeps `alerted`, sets `lastProblem` and arms the next
/// `Poll`: the turn succeeds, so a watch of the monitor is not told; read `lastProblem`. One bad row refuses the whole
/// sample, so more than 1000 rows suppress every alert until fixed. A topic that is gone rejects the whole commit
/// (`commit`): nothing is published or kept, and the timer is lost; repair the topic and send `Rearm`.
///
/// It sees only what the table holds when it reads: rows removed by a clean stop or the daily pruning before a read
/// leave no trace, a heartbeat can race the alert, and workers have no row. Use it to tell people; never to move,
/// claim or fence work.
#[derive(Debug, Clone, Default)]
pub struct NodeDownMonitorActor<S = ActorSystem> {
    source: S,
}

impl<S> NodeDownMonitorActor<S> {
    /// A monitor type that reads `source`.
    pub fn new(source: S) -> Self {
        Self { source }
    }
}

impl<S: NodeSnapshotSource> Actor for NodeDownMonitorActor<S> {
    type Ref = NodeDownMonitor;
    type State = NodeDownMonitorState;

    async fn handle(&self, turn: &mut Turn<Self>, message: NodeDownMonitorMsg) -> HandlerResult {
        match message {
            NodeDownMonitorMsg::Rearm => {
                let due = turn.state().next_poll_at.unwrap_or_else(|| turn.now());
                arm(turn, due)?;
            }
            NodeDownMonitorMsg::Poll => self.poll(turn).await?,
        }
        Ok(())
    }
}

impl<S: NodeSnapshotSource> NodeDownMonitorActor<S> {
    /// Checks the node table when this is the armed timer and it is due; arms it again when it runs early.
    async fn poll(&self, turn: &mut Turn<Self>) -> HandlerResult {
        let message = turn.message();
        let own = message.from.as_ref() == Some(turn.self_address());
        let state = turn.state();
        let (true, Some(due)) = (own && state.armed.as_ref() == Some(&message.id), state.next_poll_at) else {
            return Ok(());
        };
        if turn.now() < due {
            arm(turn, due)?;
            return Ok(());
        }
        let sample = match tokio::time::timeout(READ_TIMEOUT, self.source.snapshot()).await {
            Err(_) => Err(NodeReadProblem::Timeout),
            Ok(Err(_)) => Err(NodeReadProblem::Unavailable),
            Ok(Ok(snapshot)) => check_snapshot(&snapshot, turn.state()).map(|()| snapshot),
        };
        match sample {
            Ok(snapshot) => publish(turn, snapshot)?,
            Err(problem) => turn.state_mut().last_problem = Some(problem),
        }
        let next = match time::add(turn.current_time(), POLL_INTERVAL).map(time::ceil) {
            Ok(next) if next <= MAX_INSTANT => next,
            _ => return Err(HandlerError::permanent("the next poll would be after 9999-12-31")),
        };
        arm(turn, next)?;
        Ok(())
    }
}

/// Publishes a `NodeDown` for each dead row whose watermark is new and keeps the dead rows' watermarks.
fn publish<S: NodeSnapshotSource>(turn: &mut Turn<NodeDownMonitorActor<S>>, snapshot: NodeSnapshot) -> HandlerResult {
    let topic = turn.state().topic.clone();
    let mut alerted = BTreeMap::new();
    for node in snapshot.nodes {
        if time::between(node.last_seen, snapshot.as_of) < DEAD_AFTER {
            continue;
        }
        let mark = NodeDownWatermark { started_at: node.started_at, last_seen: node.last_seen };
        if turn.state().alerted.get(&node.id) != Some(&mark) {
            let alert = Alert::NodeDown(NodeDown {
                node: node.id.clone(),
                started_at: node.started_at,
                last_seen: node.last_seen,
                detected_at: snapshot.as_of,
                hostname: node.hostname,
                cicada_version: node.cicada_version,
                label: node.label,
            });
            let message = StoredMessage::of(&alert).map_err(HandlerError::permanent_from)?;
            let options = SendOptions::new().retry(retry_policy());
            turn.send_with(&topic, TopicMsg::Publish { message }, options)?;
        }
        alerted.insert(node.id, mark);
    }
    let state = turn.state_mut();
    state.alerted = alerted;
    state.last_checked_at = Some(snapshot.as_of);
    state.last_problem = None;
    Ok(())
}

/// Refuses a sample as a whole: too many rows, or one that is not consistent in itself or with what was kept.
fn check_snapshot(snapshot: &NodeSnapshot, state: &NodeDownMonitorState) -> Result<(), NodeReadProblem> {
    if snapshot.nodes.len() > MAX_NODES {
        return Err(NodeReadProblem::TooManyNodes);
    }
    let invalid = Err(NodeReadProblem::Invalid);
    if snapshot.as_of > MAX_INSTANT || state.last_checked_at.is_some_and(|last| snapshot.as_of < last) {
        return invalid;
    }
    let mut seen = HashSet::new();
    for node in &snapshot.nodes {
        let details = [&node.hostname, &node.cicada_version, &node.label];
        if !seen.insert(&node.id)
            || check_id(&node.id).is_err()
            || details.into_iter().flatten().any(|d| check_text(d).is_err())
            || node.started_at > node.last_seen
            || node.last_seen > snapshot.as_of
        {
            return invalid;
        }
        if let Some(mark) = state.alerted.get(&node.id)
            && mark.started_at == node.started_at
            && node.last_seen < mark.last_seen
        {
            return invalid;
        }
    }
    Ok(())
}

fn check_id(node: &NodeId) -> Result<(), String> {
    check_text(node.as_str()).map_err(|why| format!("node id {why}"))
}

/// At most 200 characters and no NUL, as the core bounds node ids and details.
fn check_text(text: &str) -> Result<(), String> {
    if text.contains('\0') {
        Err("contains NUL".to_owned())
    } else if text.chars().count() > MAX_ID_LENGTH {
        Err(format!("is longer than {MAX_ID_LENGTH} characters"))
    } else {
        Ok(())
    }
}

/// Cancels the armed timer, unless it is the message of this turn, and arms a `Poll` at `due` with the shared retry
/// policy, keeping its id and time.
fn arm<A>(turn: &mut Turn<A>, due: Timestamp) -> Result<(), TurnError>
where
    A: Actor<Ref = NodeDownMonitor, State = NodeDownMonitorState>,
{
    if let Some(armed) = turn.state().armed.clone()
        && armed != turn.message().id
    {
        turn.cancel(&armed)?;
    }
    let id = turn.send_self_with(NodeDownMonitorMsg::Poll, SendOptions::new().at(due).retry(retry_policy()))?;
    let state = turn.state_mut();
    state.armed = Some(id);
    state.next_poll_at = Some(due);
    Ok(())
}

#[cfg(test)]
mod tests {
    use cicada::runtime::NodeHealth;
    use serde_json::json;

    use super::*;

    const AT: &str = "2026-01-01T00:00:00Z";

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn decode(json: serde_json::Value) -> Result<NodeDownMonitorState, serde_json::Error> {
        serde_json::from_value(json)
    }

    fn with(extra: serde_json::Value) -> serde_json::Value {
        let mut json = json!({"topic": {"name": "ops"}});
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    }

    fn row(id: &str, started_at: &str, last_seen: &str) -> NodeInfo {
        NodeInfo {
            id: NodeId::new(id).unwrap(),
            hosted: Vec::new(),
            started_at: ts(started_at),
            last_seen: ts(last_seen),
            hostname: None,
            cicada_version: None,
            label: None,
            status: NodeHealth::Alive,
            running_turns: Vec::new(),
        }
    }

    fn sample(as_of: &str, nodes: Vec<NodeInfo>) -> NodeSnapshot {
        NodeSnapshot { as_of: ts(as_of), nodes }
    }

    fn fresh() -> NodeDownMonitorState {
        NodeDownMonitorState::new(Topic::named("ops")).unwrap()
    }

    #[test]
    fn the_minimal_json_is_a_new_monitor_and_unknown_fields_are_ignored() {
        assert_eq!(decode(with(json!({"other": 1}))).unwrap(), fresh());
        assert_eq!(
            serde_json::to_value(fresh()).unwrap(),
            with(json!({"alerted": {}, "armed": null, "nextPollAt": null, "lastCheckedAt": null, "lastProblem": null}))
        );
        assert!(decode(json!({})).is_err(), "topic is required");
        assert!(decode(json!({"topic": "cicada.topic/ops"})).is_err(), "a topic is its ref");
    }

    #[test]
    fn a_state_out_of_bounds_is_refused_on_decode_and_a_full_one_is_taken() {
        let mark = json!({"startedAt": AT, "lastSeen": AT});
        let ok = [
            json!({"armed": "p1", "nextPollAt": AT}),
            json!({"alerted": {"n1": mark}, "lastCheckedAt": AT, "lastProblem": "tooManyNodes"}),
            json!({"alerted": {"n1": {"startedAt": AT, "lastSeen": "9999-12-31T23:59:59.999999Z"}}}),
        ];
        for extra in ok {
            assert!(decode(with(extra.clone())).is_ok(), "{extra}");
        }
        let long = "n".repeat(201);
        let refused = [
            (json!({"topic": {"name": ""}}), "no address"),
            (json!({"armed": "p1"}), "set together"),
            (json!({"nextPollAt": AT}), "set together"),
            (
                json!({"alerted": {"n1": {"startedAt": "2026-01-02T00:00:00Z", "lastSeen": AT}}}),
                "startedAt <= lastSeen",
            ),
            (json!({"alerted": {"a\u{0}b": mark}}), "contains NUL"),
            (json!({"alerted": {long: mark}}), "longer than 200"),
            (json!({"alerted": {"": mark}}), "blank"),
        ];
        for (extra, why) in refused {
            let decoded = decode(with(extra.clone()));
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {extra}");
        }
        assert!(decode(with(json!({"lastProblem": "broken"}))).is_err());
        assert!(decode(with(json!({"lastCheckedAt": "10000-01-01T00:00:00Z"}))).is_err());
    }

    #[test]
    fn alerted_holds_at_most_1000_nodes() {
        let full = |n: usize| {
            let alerted: serde_json::Map<String, serde_json::Value> =
                (0..n).map(|i| (format!("n{i}"), json!({"startedAt": AT, "lastSeen": AT}))).collect();
            decode(with(json!({ "alerted": alerted })))
        };
        assert_eq!(full(MAX_NODES).unwrap().alerted.len(), 1000);
        let over = full(MAX_NODES + 1);
        assert!(over.as_ref().is_err_and(|e| e.to_string().contains("at most 1000")), "{over:?}");
    }

    #[test]
    fn a_consistent_sample_is_taken_and_one_bad_row_refuses_all_of_it() {
        let state = fresh();
        let good = || vec![row("n1", AT, "2026-01-01T00:01:00Z"), row("n2", AT, AT)];
        assert_eq!(check_snapshot(&sample("2026-01-01T00:05:00Z", good()), &state), Ok(()));
        assert_eq!(check_snapshot(&sample("2026-01-01T00:01:00Z", Vec::new()), &state), Ok(()), "empty");
        type Edit = Box<dyn Fn(&mut Vec<NodeInfo>)>;
        let bad: Vec<(&str, Edit)> = vec![
            ("a duplicate id", Box::new(|rows| rows.push(row("n1", AT, AT)))),
            ("startedAt after lastSeen", Box::new(|rows| rows[1].started_at = ts("2026-01-01T00:00:01Z"))),
            ("lastSeen after the sample", Box::new(|rows| rows[0].last_seen = ts("2026-01-01T00:05:01Z"))),
            ("a NUL in an id", Box::new(|rows| rows[0].id = NodeId::new("a\0b").unwrap())),
            ("a long id", Box::new(|rows| rows[0].id = NodeId::new("n".repeat(201)).unwrap())),
            ("a NUL in a label", Box::new(|rows| rows[1].label = Some("x\0".into()))),
            ("a long host name", Box::new(|rows| rows[1].hostname = Some("é".repeat(201)))),
            ("a NUL in a version", Box::new(|rows| rows[1].cicada_version = Some("\0".into()))),
        ];
        for (why, edit) in bad {
            let mut rows = good();
            edit(&mut rows);
            let checked = check_snapshot(&sample("2026-01-01T00:05:00Z", rows), &state);
            assert_eq!(checked, Err(NodeReadProblem::Invalid), "{why}");
        }
        let mut rows = good();
        rows[1].hostname = Some("é".repeat(200));
        rows[1].label = Some(" ".into());
        assert_eq!(check_snapshot(&sample("2026-01-01T00:05:00Z", rows), &state), Ok(()), "at the bounds");
    }

    #[test]
    fn more_than_1000_rows_are_too_many_and_a_sample_time_past_the_bound_is_invalid() {
        let state = fresh();
        let rows = |n: usize| (0..n).map(|i| row(&format!("n{i}"), AT, AT)).collect::<Vec<_>>();
        assert_eq!(check_snapshot(&sample(AT, rows(MAX_NODES)), &state), Ok(()));
        assert_eq!(check_snapshot(&sample(AT, rows(MAX_NODES + 1)), &state), Err(NodeReadProblem::TooManyNodes));
        let late = NodeSnapshot { as_of: Timestamp::MAX_UTC, nodes: Vec::new() };
        assert_eq!(check_snapshot(&late, &state), Err(NodeReadProblem::Invalid));
    }

    #[test]
    fn a_sample_time_or_alerted_last_seen_that_goes_back_is_invalid() {
        let mut state = fresh();
        state.last_checked_at = Some(ts("2026-01-01T00:05:00Z"));
        state.alerted.insert(
            NodeId::new("n1").unwrap(),
            NodeDownWatermark { started_at: ts(AT), last_seen: ts("2026-01-01T00:02:00Z") },
        );
        let at = |as_of: &str, rows| check_snapshot(&sample(as_of, rows), &state);
        assert_eq!(at("2026-01-01T00:04:59Z", Vec::new()), Err(NodeReadProblem::Invalid), "as_of goes back");
        assert_eq!(at("2026-01-01T00:05:00Z", Vec::new()), Ok(()), "the same sample time");
        let back = vec![row("n1", AT, "2026-01-01T00:01:59Z")];
        assert_eq!(at("2026-01-01T00:06:00Z", back), Err(NodeReadProblem::Invalid), "lastSeen goes back");
        let new_row = vec![row("n1", "2025-12-31T00:00:00Z", "2026-01-01T00:01:00Z")];
        assert_eq!(at("2026-01-01T00:06:00Z", new_row), Ok(()), "another startedAt is another row");
    }

    #[test]
    fn a_node_down_omits_absent_details_and_reads_back() {
        let down = NodeDown {
            node: NodeId::new("n1").unwrap(),
            started_at: ts(AT),
            last_seen: ts("2026-01-01T00:01:00Z"),
            detected_at: ts("2026-01-01T00:03:00Z"),
            hostname: None,
            cicada_version: Some("0.20.2".into()),
            label: None,
        };
        let written = serde_json::to_value(&down).unwrap();
        assert_eq!(
            written,
            json!({"node": "n1", "startedAt": AT, "lastSeen": "2026-01-01T00:01:00Z",
                   "detectedAt": "2026-01-01T00:03:00Z", "cicadaVersion": "0.20.2"})
        );
        assert_eq!(serde_json::from_value::<NodeDown>(written).unwrap(), down);
        let stored = StoredMessage::of(&Alert::NodeDown(down)).unwrap();
        assert_eq!(stored.message_type, "NodeDown");
        assert_eq!(stored.payload["node"], "n1");
    }
}
