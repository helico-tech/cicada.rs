//! The JSON bodies of the HTTP contract, shared by the server routes (`cicada-axum`) and the remote client
//! (`cicada-client`), so the two cannot disagree on the format.
//!
//! Field names and order are part of the HTTP contract; every response field is written, absent values as
//! explicit `null`. Instants are ISO-8601 strings in UTC ([`crate::time::format_instant`]), durations ISO-8601
//! durations ([`crate::time::format_duration`]).
//!
//! Request bodies ([`CreateActorRequest`], [`SendRequest`], [`RegisterTypeRequest`]) decode strictly: an unknown key in the envelope, or inside
//! its `retry` and `retention` policies, is an error, so a misspelling is never silently ignored. Payloads and states
//! inside them are plain JSON and stay lenient. Serialized, they leave absent optional fields out, so a node that does
//! not know a field yet still accepts a request that does not set it.
//!
//! The bodies of the remote turn operations (`/leases`, ADR 0040) are stricter in both directions; see [`leases`].

pub mod leases;
mod mappings;

use std::collections::BTreeMap;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::address::ActorAddress;
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::runtime::{BulkSelection, NodeHealth};
use crate::store::{MessageId, NodeId};
use crate::time::{Timestamp, serde_duration, serde_instant};
use crate::watch::WatchTarget;

pub use leases::{
    ActorCreateView, CompleteTurnRequest, CompleteTurnResponse, ControlEffectView, ControlView, LeaseTurnsRequest,
    LeaseTurnsResponse, ReleaseTurnRequest, RenewTurnRequest, RenewTurnResponse, StateReportView, TurnEffectsView,
    TurnFailureView, TurnLeaseView, TurnMessageView, TurnOutcomeView, TurnReportView, TurnSendView, WatchChangeView,
};

/// A request body whose fields contradict each other, e.g. "Give either deliverAt or delay, not both". The HTTP API
/// answers it with `400` and this text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RequestError(String);

/// Body of `PUT /actors/{type}/{key}`. The state is required: a body without it, or an empty body, is a `400`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateActorRequest {
    /// Initial state; never null. A type that keeps no data takes `{}`. It must match the state schema of the type
    /// (`GET /types/{name}`), or the create is refused with `422 invalid-state`.
    pub state: Value,
}

/// Response of `PUT /actors/{type}/{key}`: `201` when [`Self::created`], `200` when the instance already existed (unchanged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateActorResponse {
    /// The instance's canonical address.
    pub address: ActorAddress,
    /// True when this request created the instance.
    pub created: bool,
}

/// Response of `GET /actors/{type}/{key}`: an instance snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActorView {
    /// The instance's canonical address.
    pub address: ActorAddress,
    /// The name of the instance's type, e.g. `sync`.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Current state; never null.
    pub state: Value,
    /// Number of commits that wrote state.
    pub version: i64,
    /// Store time of creation.
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub created_at: Timestamp,
    /// True while the mailbox is paused: messages queue up, no new turn is claimed.
    #[serde(default)]
    pub paused: bool,
    /// The actor whose turn created this one as a child; null for an independent actor. Deleting or stopping the
    /// parent removes this actor too.
    #[serde(default)]
    pub parent: Option<ActorAddress>,
}

/// One page of a listing (`GET /actors`, `GET /actors/{type}/{key}/children`, `GET /actors/{type}/{key}/mailbox`,
/// `GET /messages`, `GET /outcomes`, `GET /turns`, `GET /watches`).
///
/// Paging: pass [`Self::next`] back as the `cursor` query parameter to get the following page; it is null on the last
/// page. The page size is the `limit` query parameter (1..1000, default 100).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "{T}Page")]
pub struct PageView<T> {
    /// Items of this page, in the listing's order.
    pub items: Vec<T>,
    /// Opaque cursor of the next page, or null when this is the last one.
    pub next: Option<String>,
}

/// The retry policy of a served envelope, read leniently: one this version cannot decode reads as none.
fn lenient_retry<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<RetryPolicy>, D::Error> {
    Ok(Option::<Value>::deserialize(deserializer)?.and_then(|v| RetryPolicy::from_stored(&v, "a served message")))
}

/// The retention policy of a served envelope, read leniently.
fn lenient_retention<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Retention>, D::Error> {
    Ok(Option::<Value>::deserialize(deserializer)?.and_then(|v| Retention::from_stored(&v, "a served message")))
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Body of `POST /actors/{type}/{key}/messages`.
///
/// Delivery time is optional and given either as an absolute [`Self::deliver_at`] or as a relative [`Self::delay`],
/// never both. A delay is an ISO-8601 duration string (`"PT5S"`); a negative one is `400`. A past `deliverAt` means
/// "now".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendRequest {
    /// Dedup key; generated by the server when absent. Reuse it when retrying a send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<MessageId>,
    /// Message type name, as listed in the target type's contract.
    #[serde(rename = "type")]
    pub message_type: String,
    /// The message's JSON (without type tag). Defaults to `{}`. It must match the schema of its message type
    /// (`GET /types/{name}`), or the send is refused with `422 invalid-payload`.
    #[serde(default = "empty_object")]
    pub payload: Value,
    /// Absolute delivery time.
    #[schemars(with = "Option<Timestamp>")]
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_instant::option")]
    pub deliver_at: Option<Timestamp>,
    /// Relative delivery time, as an ISO-8601 duration.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_duration::option")]
    #[schemars(with = "Option<String>")]
    pub delay: Option<Duration>,
    /// Opaque string passed through unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Headers passed through unchanged.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// The message's retry policy, e.g. `{"maxAttempts": 4, "backoff": {"kind": "exponential", "initial": "PT1S",
    /// "max": "PT1M"}}`; absent for no retry. An invalid policy is `400`. Ignored on a duplicate send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
    /// How long the message's record is kept once it finished, e.g. `{"done": {"kind": "remove"}}`; absent for the
    /// store default. An invalid policy is `400`. Ignored on a duplicate send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<Retention>,
}

impl BulkRequest {
    /// A request for these messages.
    pub fn ids(ids: Vec<MessageId>) -> Self {
        Self { ids: Some(ids), ..Self::default() }
    }

    /// A request for every message under `prefix`, of `message_type` when given.
    pub fn filter(prefix: impl Into<String>, message_type: Option<String>) -> Self {
        Self { ids: None, prefix: Some(prefix.into()), message_type }
    }

    /// The selection this request asks for; fails unless it gives either ids or a filter.
    pub fn selection(&self) -> Result<BulkSelection, RequestError> {
        match (&self.ids, &self.prefix, &self.message_type) {
            (None, None, None) => Err(RequestError("Give either ids or a filter (prefix, messageType)".to_owned())),
            (Some(_), None, None) => Ok(BulkSelection::Ids(self.ids.clone().unwrap_or_default())),
            (Some(_), _, _) => {
                Err(RequestError("Give either ids or a filter (prefix, messageType), not both".to_owned()))
            }
            (None, prefix, message_type) => Ok(BulkSelection::Filter {
                prefix: prefix.clone().unwrap_or_default(),
                message_type: message_type.clone(),
            }),
        }
    }
}

impl SendRequest {
    /// A send of `message_type` with `payload`, due now, without id or policies.
    pub fn new(message_type: impl Into<String>, payload: Value) -> Self {
        Self {
            id: None,
            message_type: message_type.into(),
            payload,
            deliver_at: None,
            delay: None,
            correlation_id: None,
            headers: BTreeMap::new(),
            retry: None,
            retention: None,
        }
    }
}

/// Response of `POST /actors/{type}/{key}/messages`: `202` when accepted, `200` when the id was already accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendResponse {
    /// The message id.
    pub id: MessageId,
    /// Effective delivery time (store time); null for a duplicate whose original is no longer known.
    #[schemars(with = "Option<Timestamp>")]
    #[serde(default, with = "serde_instant::option")]
    pub deliver_at: Option<Timestamp>,
}

/// A message envelope as served by the mailbox, status and outcome endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    /// Unique id; the dedup key for sends.
    pub id: MessageId,
    /// The target instance.
    pub to: ActorAddress,
    /// The sending instance, or null for a send from outside an actor.
    pub from: Option<ActorAddress>,
    /// The message type name.
    #[serde(rename = "type")]
    pub message_type: String,
    /// The payload (without type tag).
    pub payload: Value,
    /// Effective delivery time (store time).
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub deliver_at: Timestamp,
    /// Opaque string passed through.
    pub correlation_id: Option<String>,
    /// Headers passed through, such as those an observer adds.
    pub headers: BTreeMap<String, String>,
    /// Store-assigned sequence number; the tie-breaker for equal `deliverAt`.
    pub seq: i64,
    /// Number of claims, including crash recoveries and runs before a reschedule.
    pub attempt: u32,
    /// Number of failed runs that were retried under [`Self::retry`]; 0 when it never failed.
    #[serde(default)]
    pub failures: u32,
    /// Number of claims that another claim replaced although their turn neither committed nor gave the lease back
    /// (their node died, their lease ran out, or their commit did not get through). A node fails the message at its
    /// claim limit (ADR 0025). Never goes back; 0 when a view without it is read.
    #[serde(default)]
    pub lost_claims: u32,
    /// The error of the most recent retried failure, or null.
    #[serde(default)]
    pub last_error: Option<String>,
    /// The retry policy the sender attached, or null (no retry). Decoded tolerantly: a policy this version cannot read
    /// (e.g. a backoff kind of a newer node) is null rather than failing the whole response.
    #[serde(default, deserialize_with = "lenient_retry")]
    pub retry: Option<RetryPolicy>,
    /// The message whose turn sent this one; null for a message sent from outside an actor.
    #[serde(default)]
    pub caused_by: Option<MessageId>,
    /// How long the sender wants the record kept once the message finished, or null (store default). Decoded
    /// tolerantly like [`Self::retry`].
    #[serde(default, deserialize_with = "lenient_retention")]
    pub retention: Option<Retention>,
    /// True for a notification of a watch (`Failed` or `Stopped`), which only the store writes; a sender cannot set it.
    #[serde(default)]
    pub notification: bool,
}

/// A stored watch (`GET /watches`, `PUT /watches`): `watcher` is told of the events of `target`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WatchView {
    /// What is watched: an address (`type/key`), a type prefix that ends at a dot (`billing` matches `billing` and
    /// `billing.invoices`), or `*` for every type.
    pub target: WatchTarget,
    /// The actor that is told.
    pub watcher: ActorAddress,
}

/// Body of `PUT /watches`. The watcher must exist and its type must declare the messages `Failed` and `Stopped`; the
/// target need not exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchRequest {
    /// What to watch: an address, a type prefix or `*`; anything else is `400`.
    pub target: WatchTarget,
    /// The actor to tell.
    pub watcher: ActorAddress,
}

/// A message of a mailbox listing (`GET /actors/{type}/{key}/mailbox`): the envelope and whether it is running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MailboxMessageView {
    /// The envelope.
    #[serde(flatten)]
    pub message: MessageView,
    /// `running` for the message a turn is handling, otherwise `pending`.
    pub state: MessageState,
}

/// Where a message is in its life cycle, as served by `GET /messages/{id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MessageState {
    /// Waiting in the mailbox.
    Pending,
    /// A turn is handling it under a valid lease.
    Running,
    /// Handled; effects applied.
    Done,
    /// Ended as failed; see the error.
    Failed,
    /// Cancelled by an administrator.
    Cancelled,
    /// Removed because its instance was stopped or deleted.
    Purged,
}

impl MessageState {
    /// True for the four terminal states.
    pub fn is_finished(self) -> bool {
        !matches!(self, MessageState::Pending | MessageState::Running)
    }
}

/// Response of `GET /messages/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageStatusView {
    /// The message id.
    pub id: MessageId,
    /// Where the message is in its life cycle.
    pub status: MessageState,
    /// Error of a failed message.
    #[serde(default)]
    pub error: Option<String>,
    /// Number of failed runs of this message that were retried under its retry policy: a `pending` message with
    /// `failures > 0` is waiting for its next attempt.
    #[serde(default)]
    pub failures: u32,
    /// The error of the most recent retried failure, or null.
    #[serde(default)]
    pub last_error: Option<String>,
    /// Store time of the outcome, for finished messages.
    #[schemars(with = "Option<Timestamp>")]
    #[serde(default, with = "serde_instant::option")]
    pub finished_at: Option<Timestamp>,
    /// Node holding the lease, for running messages.
    #[serde(default)]
    pub node: Option<String>,
    /// Lease expiry (store time), for running messages.
    #[schemars(with = "Option<Timestamp>")]
    #[serde(default, with = "serde_instant::option")]
    pub lease_until: Option<Timestamp>,
    /// The envelope; for finished messages as it left the mailbox.
    pub message: MessageView,
}

/// A turn running now, as listed by `GET /turns` and served by `GET /actors/{type}/{key}/turn`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunningTurnView {
    /// The instance.
    pub actor: ActorAddress,
    /// The message being handled.
    pub message_id: MessageId,
    /// The node holding the lease.
    pub node: NodeId,
    /// Lease expiry (store time).
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub lease_until: Timestamp,
    /// True when an operator asked to cancel this turn.
    pub cancel_requested: bool,
}

/// A node, as listed by `GET /nodes`. Display only; leases decide who runs what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    /// The node.
    pub id: NodeId,
    /// The names of the types it hosts; empty for a send-only node.
    pub hosted: Vec<String>,
    /// Store time of its first heartbeat.
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub started_at: Timestamp,
    /// Store time of its latest heartbeat.
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub last_seen: Timestamp,
    /// `alive` (seen under 30 s ago), `suspect` (under 2 min ago) or `dead`, in the store's clock.
    pub status: NodeHealth,
    /// The number of its turns running now.
    pub running_turns: usize,
}

/// Response of `GET /nodes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NodeListView {
    /// Every node, by id.
    pub items: Vec<NodeView>,
}

/// Response of `GET /nodes/{node}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NodeDetailView {
    /// The node.
    #[serde(flatten)]
    pub node: NodeView,
    /// Its turns running now.
    pub turns: Vec<RunningTurnView>,
}

/// Response of `POST /actors/{type}/{key}/mailbox/purge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PurgeResponse {
    /// The number of messages that received outcome `purged`.
    pub purged: u64,
}

/// Body of `POST /messages/bulk-cancel` and `POST /messages/bulk-retry`: either [`Self::ids`] or a filter, a
/// [`Self::prefix`] and/or a [`Self::message_type`], not both and not neither.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BulkRequest {
    /// The messages to act on, at most 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<MessageId>>,
    /// Only messages of actors whose address starts with this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// Only messages of this type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_type: Option<String>,
}

/// Response of `POST /messages/bulk-cancel` and `POST /messages/bulk-retry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BulkResponse {
    /// How many messages were acted on; ones that were not eligible are not counted.
    pub affected: u64,
    /// True when matches remain beyond the cap of 1000 per call.
    pub more: bool,
}

/// A retained outcome, as listed by `GET /outcomes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeView {
    /// The message id.
    pub id: MessageId,
    /// The message's target.
    pub actor: ActorAddress,
    /// The terminal status.
    pub status: MessageState,
    /// The error of a failed message.
    pub error: Option<String>,
    /// Store time of the outcome.
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub finished_at: Timestamp,
    /// The envelope as it left the mailbox.
    pub message: MessageView,
}

/// Response of `GET /stats`: what is stored under an address prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatsView {
    /// The number of actors.
    pub actors: u64,
    /// The number of pending messages, including one being handled and delayed ones.
    pub pending: u64,
    /// The earliest delivery time of a pending message that is due now; null when none is due.
    #[schemars(with = "Option<Timestamp>")]
    #[serde(default, with = "serde_instant::option")]
    pub oldest_due: Option<Timestamp>,
}

/// A registered type's contract, as served by `GET /types`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TypeView {
    /// The type's name, e.g. `sync`.
    pub name: String,
    /// The contract hash (`sha256:…`).
    pub hash: String,
    /// JSON Schema of the state.
    pub state: Value,
    /// JSON Schema per message type name.
    pub messages: BTreeMap<String, Value>,
}

/// Body of `PUT /types/{name}`: the schemas of a contract to register without hosting its type. Both fields are
/// required; the name comes from the path and the server computes the hash, so neither is a field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterTypeRequest {
    /// JSON Schema of the state: an object or a boolean.
    pub state: Value,
    /// JSON Schema per message type name, each an object or a boolean; may be empty.
    pub messages: BTreeMap<String, Value>,
}

/// Response of `GET /types`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TypeListView {
    /// Every registered type, by name.
    pub items: Vec<TypeView>,
}

/// Response of `GET /health` when healthy (`200`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HealthView {
    /// Always `up`; an unhealthy node answers `503` with a problem.
    pub status: String,
    /// This node's id.
    pub node: String,
    /// Life-cycle state of the node's runtime (`STARTED`, …).
    pub state: String,
}

/// An RFC 9457 problem detail, the body of every error response (`application/problem+json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    /// A URI from [`problem_types`] identifying the kind of problem.
    #[serde(rename = "type")]
    pub problem_type: String,
    /// Short, human-readable summary of the kind of problem.
    pub title: String,
    /// The HTTP status code.
    pub status: u16,
    /// Explanation of this occurrence.
    #[serde(default)]
    pub detail: Option<String>,
    /// The request URI (path and query) of this occurrence.
    #[serde(default)]
    pub instance: Option<String>,
}

/// The problem `type` URIs the HTTP API uses.
pub mod problem_types {
    /// 400: malformed JSON body (also unknown keys in a request envelope), query parameter or path; a delivery time
    /// out of range; an envelope field that is too long or contains a NUL character; an invalid retry policy.
    pub const BAD_REQUEST: &str = "urn:cicada:problem:bad-request";
    /// 404: no registered type matches the address (create), or no type has this name (remove).
    pub const NO_SUCH_TYPE: &str = "urn:cicada:problem:no-such-type";
    /// 404: the instance does not exist.
    pub const NO_SUCH_INSTANCE: &str = "urn:cicada:problem:no-such-instance";
    /// 404: unknown message id, or its outcome is past retention.
    pub const NO_SUCH_MESSAGE: &str = "urn:cicada:problem:no-such-message";
    /// 404: no node with this id is in the node table.
    pub const NO_SUCH_NODE: &str = "urn:cicada:problem:no-such-node";
    /// 404: no turn is running on the instance (there is no valid lease).
    pub const NO_RUNNING_TURN: &str = "urn:cicada:problem:no-running-turn";
    /// 404: the watch is not stored.
    pub const NO_SUCH_WATCH: &str = "urn:cicada:problem:no-such-watch";
    /// 404: route or resource not found.
    pub const NOT_FOUND: &str = "urn:cicada:problem:not-found";
    /// 405: the method is not defined on this path.
    pub const METHOD_NOT_ALLOWED: &str = "urn:cicada:problem:method-not-allowed";
    /// 409: the message is being handled and cannot be cancelled.
    pub const MESSAGE_RUNNING: &str = "urn:cicada:problem:message-running";
    /// 409: the message already has an outcome.
    pub const MESSAGE_FINISHED: &str = "urn:cicada:problem:message-finished";
    /// 409: the message id was already accepted for a different message (another target or type).
    pub const MESSAGE_ID_CONFLICT: &str = "urn:cicada:problem:message-id-conflict";
    /// 409: the message cannot be retried: it is pending, running, done or purged.
    pub const MESSAGE_NOT_RETRYABLE: &str = "urn:cicada:problem:message-not-retryable";
    /// 409: actors of the type exist, so its registration cannot be removed.
    pub const TYPE_IN_USE: &str = "urn:cicada:problem:type-in-use";
    /// 409: the type is registered with a different contract hash; the detail names the type and both hashes.
    pub const CONTRACT_CONFLICT: &str = "urn:cicada:problem:contract-conflict";
    /// 422: the address cannot be an instance of its type: its ref does not accept the name (hosting node).
    pub const INVALID_ADDRESS: &str = "urn:cicada:problem:invalid-address";
    /// 413: the request body is larger than the node accepts.
    pub const REQUEST_TOO_LARGE: &str = "urn:cicada:problem:request-too-large";
    /// 500: an unexpected error in the node; not retryable as such.
    pub const INTERNAL_ERROR: &str = "urn:cicada:problem:internal-error";
    /// 422: the state of a create is `null`, does not match the registered state schema (every node; the detail names
    /// no place, the same on every node), or matches and cannot be decoded (hosting node); the detail says which check
    /// refused it.
    pub const INVALID_STATE: &str = "urn:cicada:problem:invalid-state";
    /// 422: the target type does not accept this message type name.
    pub const UNKNOWN_MESSAGE_TYPE: &str = "urn:cicada:problem:unknown-message-type";
    /// 422: the payload does not match the registered schema of its message type (every node; the detail names no
    /// place, the same on every node), or matches and cannot be decoded (hosting node); the detail says which check
    /// refused it.
    pub const INVALID_PAYLOAD: &str = "urn:cicada:problem:invalid-payload";
    /// 422: the watcher's type may not watch: its message family does not declare `Failed` and `Stopped` with the
    /// core's types.
    pub const NOT_A_WATCHER: &str = "urn:cicada:problem:not-a-watcher";
    /// 422: a contract to register has a schema that is not an object or a boolean, nested deeper than 120 levels or
    /// with a NUL character, or a message name beyond the send limits.
    pub const INVALID_CONTRACT: &str = "urn:cicada:problem:invalid-contract";
    /// 503: the store is unreachable or failed.
    pub const STORE_UNAVAILABLE: &str = "urn:cicada:problem:store-unavailable";
    /// 503: the node is stopping or stopped, or its store is unreachable (`/health`).
    pub const UNHEALTHY: &str = "urn:cicada:problem:unhealthy";
}

#[cfg(test)]
mod tests;
