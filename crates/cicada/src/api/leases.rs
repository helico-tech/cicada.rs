//! The bodies of the remote turn operations (ADR 0040): `POST /leases`, `/leases/renew`, `/leases/release` and
//! `/leases/complete`, with which a trusted worker outside the database network leases, renews, gives back and
//! completes turns through any server on the store.
//!
//! **Requests** decode strictly: an unknown key anywhere in the envelope, the report, its effects or a policy is an
//! error; states and payloads are plain JSON data. A field is required unless its documentation says it is optional:
//! the arrays of [`TurnEffectsView`] default to empty, and the optional fields of a [`TurnSendView`] follow
//! [`SendRequest`](super::SendRequest) (absent or null means the default; `headers` defaults to `{}` and cannot be null).
//! Every record, in requests and responses alike, is a JSON object: its fields as a positional array are refused, also
//! inside a retry or retention policy.
//!
//! **Responses** are tagged by `result` and tolerate unknown envelope fields, but nothing else: every listed field is
//! required, also a nullable one (it is written as `null`), except the delivery of a rescheduled outcome. An unknown
//! `result`, `kind` or cause, and a retry or retention policy this version cannot read, fail decoding instead of
//! reading as a default. A [`TurnMessageView`] has every field of [`MessageView`](super::MessageView), all required.
//!
//! `epoch`, `version` and `seq` are JSON integers with the full range of `i64`, and counters `u32`: a client must
//! keep every bit (not round them through a floating-point number), and a float or an exponent is refused. Instants
//! and durations use the formats of the rest of the API, durations down to the nanosecond (`PT0.000000002S`). Actor
//! addresses are written in canonical form in the body, never percent-encoded again.

use std::collections::BTreeMap;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::RequestError;
use crate::address::{ActorAddress, TypeName};
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::runtime::{
    CompleteTurnResult, LeaseTurnsResult, RenewTurnResult, StateReport, TurnCompletion, TurnEffects, TurnFailure,
    TurnLease, TurnReport, TurnSend,
};
use crate::store::{ActorCreate, Control, ControlEffect, Delivery, Message, MessageId, NodeId, TurnOutcome};
use crate::time::{Timestamp, serde_duration, serde_instant};
use crate::watch::{FailureCause, WatchChange, WatchTarget};

/// A field that must be present and may be `null`: serde's default for an `Option` would read a missing field as
/// `None`.
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

/// The schema of a field that is required and may be `null`: `T` or `null`. A field typed `Option<T>` would be
/// optional in the schema, and `#[schemars(required)]` would drop the `null`.
struct Nullable<T>(std::marker::PhantomData<T>);

impl<T: JsonSchema> JsonSchema for Nullable<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("Nullable_{}", T::schema_name()).into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "anyOf": [generator.subschema_for::<T>(), { "type": "null" }] })
    }
}

/// A retry or retention policy of a lease body, `null` read as none: a JSON object with no array anywhere in it (no
/// policy holds one), decoded by the policy's own strict decoder. The policies' decoders, shared with the other routes,
/// would also take a record as a positional array.
fn policy<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(deserializer: D) -> Result<Option<T>, D::Error> {
    fn has_array(value: &Value) -> bool {
        match value {
            Value::Array(_) => true,
            Value::Object(o) => o.values().any(has_array),
            _ => false,
        }
    }
    match Option::<Value>::deserialize(deserializer)? {
        None => Ok(None),
        Some(value) if !value.is_object() || has_array(&value) => {
            Err(serde::de::Error::custom("a policy is a JSON object of objects, with no array in it"))
        }
        Some(value) => T::deserialize(value).map(Some).map_err(serde::de::Error::custom),
    }
}

/// A deserializer that hands its visitor only a JSON object: the derived decoder of a record also takes the record's
/// fields as a positional array, which no body of these routes may use.
struct ObjectOnly<D>(D);

/// The visitor of [`ObjectOnly`]: a map goes to `V`, anything else is the error of an unexpected type.
struct MapVisitor<V>(V);

impl<'de, V: serde::de::Visitor<'de>> serde::de::Visitor<'de> for MapVisitor<V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.expecting(f)?;
        f.write_str(" as a JSON object")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.0.visit_map(map)
    }
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for ObjectOnly<D> {
    type Error = D::Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(MapVisitor(visitor))
    }

    fn deserialize_map<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(MapVisitor(visitor))
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_struct(name, fields, MapVisitor(visitor))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit unit_struct
        newtype_struct seq tuple tuple_struct enum identifier ignored_any
    }
}

/// A record of these bodies, declared once: the public type, with `Serialize` and `JsonSchema` derived, and a private
/// shadow `$wire` that derives the field decoder with `#[serde(remote = ...)]`. The public type's `Deserialize` runs that
/// decoder through [`ObjectOnly`], so a record is decoded only from a JSON object, never from a positional array, and no
/// public method decodes it without that check.
macro_rules! record {
    ($(#[$meta:meta])* pub $kind:ident $name:ident via $wire:ident = $remote:literal $body:tt) => {
        #[derive(Serialize, JsonSchema)]
        $(#[$meta])*
        pub $kind $name $body

        /// The private decoder of the record of the same name, without the object check.
        #[derive(Deserialize, JsonSchema)]
        #[serde(remote = $remote)]
        #[allow(dead_code)]
        $(#[$meta])*
        $kind $wire $body

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                $wire::deserialize(ObjectOnly(deserializer))
            }
        }
    };
}

// ---- requests ----

record! {
/// Body of `POST /leases`: lease up to `limit` due turns of `types` for `owner`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LeaseTurnsRequest via LeaseTurnsRequestWire = "LeaseTurnsRequest" {
    /// The worker that will hold the leases, recorded as their owner: at most 200 characters, no NUL.
    pub owner: NodeId,
    /// Each selected type name with the contract hash the worker was built with; 1 to 100 types (`MAX_LEASE_TYPES`).
    /// A type that is not registered with exactly that hash answers `contractMismatch` and nothing is leased.
    #[schemars(with = "BTreeMap<String, String>")]
    pub types: BTreeMap<TypeName, String>,
    /// The most turns to lease, 1 to 100 (`MAX_LEASE_BATCH`).
    pub limit: usize,
}
}

record! {
/// Body of `POST /leases/renew`: renew the lease of `actor` held under `epoch`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RenewTurnRequest via RenewTurnRequestWire = "RenewTurnRequest" {
    /// The leased actor, as the lease named it.
    pub actor: ActorAddress,
    /// The epoch of the lease.
    pub epoch: i64,
}
}

record! {
/// Body of `POST /leases/release`: give back the lease of `actor` held under `epoch`, without an outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseTurnRequest via ReleaseTurnRequestWire = "ReleaseTurnRequest" {
    /// The leased actor, as the lease named it.
    pub actor: ActorAddress,
    /// The epoch of the lease.
    pub epoch: i64,
}
}

record! {
/// Body of `POST /leases/complete`: complete the leased turn of `actor` under `epoch` on `messageId` with the report of
/// its handler. The server derives the outcome; a report names none.
#[derive(Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompleteTurnRequest via CompleteTurnRequestWire = "CompleteTurnRequest" {
    /// The leased actor, as the lease named it.
    pub actor: ActorAddress,
    /// The epoch of the lease.
    pub epoch: i64,
    /// The leased message.
    pub message_id: MessageId,
    /// How the handler ended.
    pub report: TurnReportView,
}
}

record! {
/// How the handler of a leased turn ended ([`TurnReport`]), tagged by `kind`.
#[derive(Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum TurnReportView via TurnReportViewWire = "TurnReportView" {
    /// The handler returned: its state and its effects, applied together or not at all.
    Returned {
        /// What happens to the actor's own state.
        state: StateReportView,
        /// The effects the handler buffered; required, each of its arrays optional.
        effects: TurnEffectsView,
    },
    /// The handler failed, or the turn failed around it; no effects. Only `handler`, `node` and `timeout` may be
    /// retryable, and `claim-limit` cannot be reported.
    Failed {
        /// Why the message fails.
        cause: FailureCause,
        /// True when the message's own retry policy may reschedule it.
        retryable: bool,
        /// The reason it is recorded with.
        reason: String,
    },
    /// An operator's cancel request came before the handler ended; no effects.
    Cancelled {},
}
}

record! {
/// What a returned handler did to its actor's own state ([`StateReport`]), tagged by `kind`.
#[derive(Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum StateReportView via StateReportViewWire = "StateReportView" {
    /// State and version stay as they are.
    Unchanged {},
    /// The new state, written with the version increased by 1; required, also when it is `null` (which fails the
    /// message as a state that cannot be stored).
    Set {
        /// The new state.
        value: Value,
    },
    /// The actor and its descendants stop.
    Stop {},
}
}

record! {
/// The effects of a returned handler ([`TurnEffects`]), applied by the commit in the order the core gives them:
/// watches, deletes, creates, purges, pauses and resumes, then sends. Each array is optional and empty by default.
#[derive(Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TurnEffectsView via TurnEffectsViewWire = "TurnEffectsView" {
    /// The messages to send, in order.
    #[serde(default)]
    pub sends: Vec<TurnSendView>,
    /// The actors to create.
    #[serde(default)]
    pub creates: Vec<ActorCreateView>,
    /// Pending messages of the actor to cancel.
    #[serde(default)]
    pub cancels: Vec<MessageId>,
    /// The watches set and removed, with the actor as watcher, in the order of the calls.
    #[serde(default)]
    pub watches: Vec<WatchChangeView>,
    /// The control effects on actors.
    #[serde(default)]
    pub controls: Vec<ControlView>,
}
}

record! {
/// A message a returned handler sends ([`TurnSend`]). The server sets its sender to the leased actor and its cause to
/// the leased message; the id is the worker's and is never generated.
#[derive(Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnSendView via TurnSendViewWire = "TurnSendView" {
    /// The message id, drawn once by the worker and kept when it repeats the report.
    pub id: MessageId,
    /// The target.
    pub to: ActorAddress,
    /// The message type name.
    #[serde(rename = "type")]
    pub message_type: String,
    /// The payload.
    pub payload: Value,
    /// Optional absolute delivery time; not together with [`Self::delay`]. Neither means now.
    #[schemars(with = "Option<Timestamp>")]
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_instant::option")]
    pub deliver_at: Option<Timestamp>,
    /// Optional relative delivery time, as an ISO-8601 duration.
    #[schemars(with = "Option<String>")]
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_duration::option")]
    pub delay: Option<Duration>,
    /// Optional opaque string passed through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Optional headers passed through; `{}` by default, never null.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Optional retry policy of the new message, as for a send.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "policy")]
    pub retry: Option<RetryPolicy>,
    /// Optional retention of the new message's record, as for a send.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "policy")]
    pub retention: Option<Retention>,
}
}

record! {
/// An actor a returned handler creates ([`ActorCreate`]).
#[derive(Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActorCreateView via ActorCreateViewWire = "ActorCreateView" {
    /// The instance to create.
    pub address: ActorAddress,
    /// Its initial state.
    pub state: Value,
    /// True for a child of the leased actor, false for an independent actor.
    pub child: bool,
}
}

record! {
/// A watch a returned handler sets or removes, with the leased actor as watcher ([`WatchChange`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WatchChangeView via WatchChangeViewWire = "WatchChangeView" {
    /// What is watched.
    pub target: WatchTarget,
    /// True to set the watch, false to remove it.
    pub set: bool,
}
}

record! {
/// A control effect of a returned handler on an actor ([`Control`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ControlView via ControlViewWire = "ControlView" {
    /// What happens to the actor.
    pub effect: ControlEffectView,
    /// The actor.
    pub actor: ActorAddress,
}
}

/// What a control effect does ([`ControlEffect`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ControlEffectView {
    /// No new turn is claimed for the actor.
    Pause,
    /// Turns of the actor are claimed again.
    Resume,
    /// Every pending message of the actor except one being handled ends `purged`.
    Purge,
    /// The actor, its mailbox and its descendants go.
    Delete,
}

// ---- responses ----

record! {
/// Response of `POST /leases` (`200`), tagged by `result`.
#[derive(Debug, Clone, PartialEq)]
#[serde(tag = "result", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LeaseTurnsResponse via LeaseTurnsResponseWire = "LeaseTurnsResponse" {
    /// The leased turns, at most the limit; empty when nothing was due (no long poll).
    Leased {
        /// The leases.
        leases: Vec<TurnLeaseView>,
    },
    /// A selected type is not registered with the requested hash; nothing was leased.
    ContractMismatch {
        /// The first such type in name order.
        #[serde(rename = "type")]
        #[schemars(with = "String")]
        type_name: TypeName,
        /// The registered hash, or null when the type is not registered; required.
        #[serde(deserialize_with = "nullable")]
        #[schemars(with = "Nullable<String>")]
        registered: Option<String>,
        /// The hash the worker asked for.
        requested: String,
    },
}
}

record! {
/// A leased turn ([`TurnLease`]): everything a worker needs to run it. The worker keeps its own deadline: its
/// monotonic time when it sent the lease request plus `ttl`, less the smaller of a tenth of it and one second.
#[derive(Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnLeaseView via TurnLeaseViewWire = "TurnLeaseView" {
    /// The leased actor; echo it as it is.
    pub actor: ActorAddress,
    /// The fencing token of the lease.
    pub epoch: i64,
    /// The head message, with `attempt` already counting this lease.
    pub message: TurnMessageView,
    /// The actor's state.
    pub state: Value,
    /// The actor's version.
    pub version: i64,
    /// Store time at the claim; the handler's `now`.
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub claimed_at: Timestamp,
    /// Store time at which the lease expires unless renewed; for display, not a deadline.
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub lease_until: Timestamp,
    /// The TTL the server asked the store for, as an ISO-8601 duration.
    #[schemars(with = "String")]
    #[serde(with = "serde_duration")]
    pub ttl: Duration,
}
}

record! {
/// A message as a worker runs it ([`Message`]): the fields of [`MessageView`](super::MessageView), every one required
/// (nullable ones as `null`, counters also when 0) and the policies decoded strictly.
#[derive(Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnMessageView via TurnMessageViewWire = "TurnMessageView" {
    /// Unique id.
    pub id: MessageId,
    /// The target instance.
    pub to: ActorAddress,
    /// The sending instance, or null.
    #[serde(deserialize_with = "nullable")]
    #[schemars(with = "Nullable<ActorAddress>")]
    pub from: Option<ActorAddress>,
    /// The message type name.
    #[serde(rename = "type")]
    pub message_type: String,
    /// The payload.
    pub payload: Value,
    /// Effective delivery time (store time).
    #[schemars(with = "Timestamp")]
    #[serde(with = "serde_instant")]
    pub deliver_at: Timestamp,
    /// Opaque string passed through, or null.
    #[serde(deserialize_with = "nullable")]
    #[schemars(with = "Nullable<String>")]
    pub correlation_id: Option<String>,
    /// Headers passed through.
    pub headers: BTreeMap<String, String>,
    /// Store-assigned sequence number.
    pub seq: i64,
    /// Number of claims, this lease included.
    pub attempt: u32,
    /// Number of failed runs that were retried.
    pub failures: u32,
    /// Number of claims another claim replaced.
    pub lost_claims: u32,
    /// The error of the most recent retried failure, or null.
    #[serde(deserialize_with = "nullable")]
    #[schemars(with = "Nullable<String>")]
    pub last_error: Option<String>,
    /// The retry policy, or null.
    #[serde(deserialize_with = "policy")]
    #[schemars(with = "Nullable<RetryPolicy>")]
    pub retry: Option<RetryPolicy>,
    /// The message whose turn sent this one, or null.
    #[serde(deserialize_with = "nullable")]
    #[schemars(with = "Nullable<MessageId>")]
    pub caused_by: Option<MessageId>,
    /// The retention of the record, or null.
    #[serde(deserialize_with = "policy")]
    #[schemars(with = "Nullable<Retention>")]
    pub retention: Option<Retention>,
    /// True for a notification of a watch.
    pub notification: bool,
}
}

record! {
/// Response of `POST /leases/renew` (`200`), tagged by `result`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RenewTurnResponse via RenewTurnResponseWire = "RenewTurnResponse" {
    /// The lease is extended.
    Renewed {
        /// Store time at which the lease now expires: never earlier than before.
        #[schemars(with = "Timestamp")]
        #[serde(with = "serde_instant")]
        lease_until: Timestamp,
        /// The TTL the server asked the store for.
        #[schemars(with = "String")]
        #[serde(with = "serde_duration")]
        ttl: Duration,
        /// An operator asked to cancel this turn; the lease is extended all the same.
        cancel_requested: bool,
    },
    /// The epoch is no longer current or its lease was released, completed or deleted: abandon the turn.
    LeaseLost,
}
}

record! {
/// Response of `POST /leases/complete` (`200`), tagged by `result`.
#[derive(Debug, Clone, PartialEq)]
#[serde(tag = "result", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CompleteTurnResponse via CompleteTurnResponseWire = "CompleteTurnResponse" {
    /// The commit applied ([`TurnCompletion`]).
    Completed {
        /// How the message ended, or that it was rescheduled.
        outcome: TurnOutcomeView,
        /// Why it failed, for a `failed` outcome; null otherwise, and required.
        #[serde(deserialize_with = "nullable")]
        #[schemars(with = "Nullable<FailureCause>")]
        cause: Option<FailureCause>,
        /// The actor's version after the commit.
        version: i64,
        /// The sends as enqueued.
        sent: Vec<TurnMessageView>,
        /// The cancels that found a pending message.
        cancelled: Vec<MessageId>,
    },
    /// Nothing was applied and no failure was committed: report `failed` with this failure next, and from then on only
    /// that report.
    Rejected {
        /// The failure to report.
        failure: TurnFailureView,
    },
    /// The lease is not held (any more): nothing was applied by this call, which says nothing about an earlier one.
    LeaseLost,
}
}

record! {
/// A failure ([`TurnFailure`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnFailureView via TurnFailureViewWire = "TurnFailureView" {
    /// Why the message fails.
    pub cause: FailureCause,
    /// True when the message's retry policy may reschedule it.
    pub retryable: bool,
    /// The reason.
    pub reason: String,
}
}

record! {
/// How a completed commit ended the message ([`TurnOutcome`]), tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum TurnOutcomeView via TurnOutcomeViewWire = "TurnOutcomeView" {
    /// Outcome `done`.
    Done,
    /// Outcome `cancelled`.
    Cancelled,
    /// Outcome `failed`.
    Failed {
        /// The recorded error.
        error: String,
    },
    /// Put back into the mailbox by its retry policy, without an outcome. The retry is due at `deliverAt`, after
    /// `delay`, or now when neither is given; never both.
    Rescheduled {
        /// The failure, recorded as `lastError`.
        error: String,
        /// Absolute time of the retry.
        #[schemars(with = "Option<Timestamp>")]
        #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_instant::option")]
        deliver_at: Option<Timestamp>,
        /// Delay of the retry.
        #[schemars(with = "Option<String>")]
        #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_duration::option")]
        delay: Option<Duration>,
    },
}
}

// ---- conversions ----

/// The delivery of `deliver_at` and `delay`: both is an error, neither is now.
fn delivery_of(deliver_at: Option<Timestamp>, delay: Option<Duration>) -> Result<Delivery, RequestError> {
    match (deliver_at, delay) {
        (Some(_), Some(_)) => Err(RequestError("Give either deliverAt or delay, not both".to_owned())),
        (Some(at), None) => Ok(Delivery::At(at)),
        (None, Some(after)) => Ok(Delivery::After(after)),
        (None, None) => Ok(Delivery::Now),
    }
}

fn delivery_fields(delivery: Delivery) -> (Option<Timestamp>, Option<Duration>) {
    match delivery {
        Delivery::Now => (None, None),
        Delivery::At(at) => (Some(at), None),
        Delivery::After(after) => (None, Some(after)),
    }
}

impl From<Message> for TurnMessageView {
    fn from(m: Message) -> Self {
        Self {
            id: m.id,
            to: m.to,
            from: m.from,
            message_type: m.message_type,
            payload: m.payload,
            deliver_at: m.deliver_at,
            correlation_id: m.correlation_id,
            headers: m.headers,
            seq: m.seq,
            attempt: m.attempt,
            failures: m.failures,
            lost_claims: m.lost_claims,
            last_error: m.last_error,
            retry: m.retry,
            caused_by: m.caused_by,
            retention: m.retention,
            notification: m.notification,
        }
    }
}

impl From<TurnMessageView> for Message {
    fn from(m: TurnMessageView) -> Self {
        Self {
            id: m.id,
            to: m.to,
            from: m.from,
            message_type: m.message_type,
            payload: m.payload,
            deliver_at: m.deliver_at,
            correlation_id: m.correlation_id,
            headers: m.headers,
            seq: m.seq,
            attempt: m.attempt,
            failures: m.failures,
            lost_claims: m.lost_claims,
            last_error: m.last_error,
            retry: m.retry,
            caused_by: m.caused_by,
            retention: m.retention,
            notification: m.notification,
        }
    }
}

impl From<TurnLease> for TurnLeaseView {
    fn from(l: TurnLease) -> Self {
        Self {
            actor: l.actor,
            epoch: l.epoch,
            message: l.message.into(),
            state: l.state,
            version: l.version,
            claimed_at: l.claimed_at,
            lease_until: l.lease_until,
            ttl: l.ttl,
        }
    }
}

impl From<TurnLeaseView> for TurnLease {
    fn from(l: TurnLeaseView) -> Self {
        Self {
            actor: l.actor,
            epoch: l.epoch,
            message: l.message.into(),
            state: l.state,
            version: l.version,
            claimed_at: l.claimed_at,
            lease_until: l.lease_until,
            ttl: l.ttl,
        }
    }
}

impl From<LeaseTurnsResult> for LeaseTurnsResponse {
    fn from(r: LeaseTurnsResult) -> Self {
        match r {
            LeaseTurnsResult::Leased(leases) => {
                LeaseTurnsResponse::Leased { leases: leases.into_iter().map(Into::into).collect() }
            }
            LeaseTurnsResult::ContractMismatch { type_name, registered, requested } => {
                LeaseTurnsResponse::ContractMismatch { type_name, registered, requested }
            }
        }
    }
}

impl From<LeaseTurnsResponse> for LeaseTurnsResult {
    fn from(r: LeaseTurnsResponse) -> Self {
        match r {
            LeaseTurnsResponse::Leased { leases } => {
                LeaseTurnsResult::Leased(leases.into_iter().map(Into::into).collect())
            }
            LeaseTurnsResponse::ContractMismatch { type_name, registered, requested } => {
                LeaseTurnsResult::ContractMismatch { type_name, registered, requested }
            }
        }
    }
}

impl From<RenewTurnResult> for RenewTurnResponse {
    fn from(r: RenewTurnResult) -> Self {
        match r {
            RenewTurnResult::Renewed { lease_until, ttl, cancel_requested } => {
                RenewTurnResponse::Renewed { lease_until, ttl, cancel_requested }
            }
            RenewTurnResult::LeaseLost => RenewTurnResponse::LeaseLost,
        }
    }
}

impl From<RenewTurnResponse> for RenewTurnResult {
    fn from(r: RenewTurnResponse) -> Self {
        match r {
            RenewTurnResponse::Renewed { lease_until, ttl, cancel_requested } => {
                RenewTurnResult::Renewed { lease_until, ttl, cancel_requested }
            }
            RenewTurnResponse::LeaseLost => RenewTurnResult::LeaseLost,
        }
    }
}

impl From<TurnFailure> for TurnFailureView {
    fn from(f: TurnFailure) -> Self {
        Self { cause: f.cause, retryable: f.retryable, reason: f.reason }
    }
}

impl From<TurnFailureView> for TurnFailure {
    fn from(f: TurnFailureView) -> Self {
        Self { cause: f.cause, retryable: f.retryable, reason: f.reason }
    }
}

impl From<TurnOutcome> for TurnOutcomeView {
    fn from(o: TurnOutcome) -> Self {
        match o {
            TurnOutcome::Done => TurnOutcomeView::Done,
            TurnOutcome::Cancelled => TurnOutcomeView::Cancelled,
            TurnOutcome::Failed(error) => TurnOutcomeView::Failed { error },
            TurnOutcome::Rescheduled { delivery, error } => {
                let (deliver_at, delay) = delivery_fields(delivery);
                TurnOutcomeView::Rescheduled { error, deliver_at, delay }
            }
        }
    }
}

impl TryFrom<TurnOutcomeView> for TurnOutcome {
    type Error = RequestError;

    /// Fails for a rescheduled outcome with both a `deliverAt` and a `delay`.
    fn try_from(o: TurnOutcomeView) -> Result<Self, RequestError> {
        Ok(match o {
            TurnOutcomeView::Done => TurnOutcome::Done,
            TurnOutcomeView::Cancelled => TurnOutcome::Cancelled,
            TurnOutcomeView::Failed { error } => TurnOutcome::Failed(error),
            TurnOutcomeView::Rescheduled { error, deliver_at, delay } => {
                TurnOutcome::Rescheduled { delivery: delivery_of(deliver_at, delay)?, error }
            }
        })
    }
}

impl From<CompleteTurnResult> for CompleteTurnResponse {
    fn from(r: CompleteTurnResult) -> Self {
        match r {
            CompleteTurnResult::Completed(TurnCompletion { outcome, cause, version, sent, cancelled }) => {
                CompleteTurnResponse::Completed {
                    outcome: outcome.into(),
                    cause,
                    version,
                    sent: sent.into_iter().map(Into::into).collect(),
                    cancelled,
                }
            }
            CompleteTurnResult::Rejected(failure) => CompleteTurnResponse::Rejected { failure: failure.into() },
            CompleteTurnResult::LeaseLost => CompleteTurnResponse::LeaseLost,
        }
    }
}

impl TryFrom<CompleteTurnResponse> for CompleteTurnResult {
    type Error = RequestError;

    /// Fails for an outcome that cannot be one: a reschedule with two delivery times, or a `cause` given for anything
    /// but a `failed` outcome or missing for one.
    fn try_from(r: CompleteTurnResponse) -> Result<Self, RequestError> {
        Ok(match r {
            CompleteTurnResponse::Completed { outcome, cause, version, sent, cancelled } => {
                let outcome = TurnOutcome::try_from(outcome)?;
                if matches!(outcome, TurnOutcome::Failed(_)) != cause.is_some() {
                    return Err(RequestError("A completed turn has a cause exactly when it failed".to_owned()));
                }
                CompleteTurnResult::Completed(TurnCompletion {
                    outcome,
                    cause,
                    version,
                    sent: sent.into_iter().map(Into::into).collect(),
                    cancelled,
                })
            }
            CompleteTurnResponse::Rejected { failure } => CompleteTurnResult::Rejected(failure.into()),
            CompleteTurnResponse::LeaseLost => CompleteTurnResult::LeaseLost,
        })
    }
}

impl From<ControlEffect> for ControlEffectView {
    fn from(e: ControlEffect) -> Self {
        match e {
            ControlEffect::Pause => ControlEffectView::Pause,
            ControlEffect::Resume => ControlEffectView::Resume,
            ControlEffect::Purge => ControlEffectView::Purge,
            ControlEffect::Delete => ControlEffectView::Delete,
        }
    }
}

impl From<ControlEffectView> for ControlEffect {
    fn from(e: ControlEffectView) -> Self {
        match e {
            ControlEffectView::Pause => ControlEffect::Pause,
            ControlEffectView::Resume => ControlEffect::Resume,
            ControlEffectView::Purge => ControlEffect::Purge,
            ControlEffectView::Delete => ControlEffect::Delete,
        }
    }
}

impl From<TurnSend> for TurnSendView {
    fn from(s: TurnSend) -> Self {
        let (deliver_at, delay) = delivery_fields(s.delivery);
        Self {
            id: s.id,
            to: s.to,
            message_type: s.message_type,
            payload: s.payload,
            deliver_at,
            delay,
            correlation_id: s.correlation_id,
            headers: s.headers,
            retry: s.retry,
            retention: s.retention,
        }
    }
}

impl TryFrom<TurnSendView> for TurnSend {
    type Error = RequestError;

    /// Fails when both `deliverAt` and `delay` are given.
    fn try_from(s: TurnSendView) -> Result<Self, RequestError> {
        Ok(Self {
            delivery: delivery_of(s.deliver_at, s.delay)?,
            to: s.to,
            message_type: s.message_type,
            payload: s.payload,
            id: s.id,
            correlation_id: s.correlation_id,
            headers: s.headers,
            retry: s.retry,
            retention: s.retention,
        })
    }
}

impl From<TurnEffects> for TurnEffectsView {
    fn from(e: TurnEffects) -> Self {
        Self {
            sends: e.sends.into_iter().map(Into::into).collect(),
            creates: e
                .creates
                .into_iter()
                .map(|c| ActorCreateView { address: c.address, state: c.state, child: c.child })
                .collect(),
            cancels: e.cancels,
            watches: e.watches.into_iter().map(|w| WatchChangeView { target: w.target, set: w.set }).collect(),
            controls: e.controls.into_iter().map(|c| ControlView { effect: c.effect.into(), actor: c.actor }).collect(),
        }
    }
}

impl TryFrom<TurnEffectsView> for TurnEffects {
    type Error = RequestError;

    /// Fails when a send has both delivery times.
    fn try_from(e: TurnEffectsView) -> Result<Self, RequestError> {
        Ok(Self {
            watches: e.watches.into_iter().map(|w| WatchChange { target: w.target, set: w.set }).collect(),
            creates: e
                .creates
                .into_iter()
                .map(|c| ActorCreate { address: c.address, state: c.state, child: c.child })
                .collect(),
            sends: e.sends.into_iter().map(TurnSend::try_from).collect::<Result<_, _>>()?,
            cancels: e.cancels,
            controls: e.controls.into_iter().map(|c| Control { effect: c.effect.into(), actor: c.actor }).collect(),
        })
    }
}

impl From<StateReport> for StateReportView {
    fn from(s: StateReport) -> Self {
        match s {
            StateReport::Unchanged => StateReportView::Unchanged {},
            StateReport::Set(value) => StateReportView::Set { value },
            StateReport::Stop => StateReportView::Stop {},
        }
    }
}

impl From<StateReportView> for StateReport {
    fn from(s: StateReportView) -> Self {
        match s {
            StateReportView::Unchanged {} => StateReport::Unchanged,
            StateReportView::Set { value } => StateReport::Set(value),
            StateReportView::Stop {} => StateReport::Stop,
        }
    }
}

impl From<TurnReport> for TurnReportView {
    fn from(r: TurnReport) -> Self {
        match r {
            TurnReport::Returned { state, effects } => {
                TurnReportView::Returned { state: state.into(), effects: effects.into() }
            }
            TurnReport::Failed(TurnFailure { cause, retryable, reason }) => {
                TurnReportView::Failed { cause, retryable, reason }
            }
            TurnReport::Cancelled => TurnReportView::Cancelled {},
        }
    }
}

impl TryFrom<TurnReportView> for TurnReport {
    type Error = RequestError;

    /// Fails when a send has both delivery times.
    fn try_from(r: TurnReportView) -> Result<Self, RequestError> {
        Ok(match r {
            TurnReportView::Returned { state, effects } => {
                TurnReport::Returned { state: state.into(), effects: effects.try_into()? }
            }
            TurnReportView::Failed { cause, retryable, reason } => {
                TurnReport::Failed(TurnFailure { cause, retryable, reason })
            }
            TurnReportView::Cancelled {} => TurnReport::Cancelled,
        })
    }
}

#[cfg(test)]
mod tests;
