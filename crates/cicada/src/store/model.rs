use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::address::{ActorAddress, TypeName};
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::time::{self, MAX_DELAY, MAX_INSTANT, TimeError, Timestamp};
use crate::watch::WatchChange;

/// An id that is not valid: a [`MessageId`] or [`NodeId`] that is blank. It prints why, e.g. "MessageId must not be
/// blank".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct IdError(String);

macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident, $what:literal) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Wraps `value`; fails when it is blank.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdError(concat!($what, " must not be blank").to_owned()));
                }
                Ok(Self(value))
            }

            /// The id text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, IdError> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({ "type": "string", "minLength": 1 })
            }

            fn inline_schema() -> bool {
                true
            }
        }
    };
}

string_id! {
    /// Unique id of a message. A UUID string by convention; the dedup key for sends.
    MessageId, "MessageId"
}

impl MessageId {
    /// A new random (version 4) UUID id.
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

string_id! {
    /// Identity of a node, recorded as the owner of the leases it holds.
    NodeId, "NodeId"
}

impl NodeId {
    /// A random node id, `node-` plus eight hex digits.
    pub fn random() -> Self {
        Self(format!("node-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]))
    }
}

/// Opaque position in a paged listing; pass [`Page::next`] back to continue.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageCursor(pub String);

impl fmt::Display for PageCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One page of a listing.
#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    /// The items of this page, in the listing's order.
    pub items: Vec<T>,
    /// Cursor for the following page, or `None` when this is the last page.
    pub next: Option<PageCursor>,
}

impl<T> Page<T> {
    /// Default page size for listings.
    pub const DEFAULT_SIZE: usize = 100;

    /// A page of `items` followed by `next`.
    pub fn new(items: Vec<T>, next: Option<PageCursor>) -> Self {
        Self { items, next }
    }
}

/// A message as held by the store: the envelope plus the store-assigned `seq` and `attempt`.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// Unique id; the dedup key for sends.
    pub id: MessageId,
    /// The target instance.
    pub to: ActorAddress,
    /// The sending instance, or `None` for sends from outside an actor.
    pub from: Option<ActorAddress>,
    /// The message type name (the variant name of the message family).
    pub message_type: String,
    /// JSON written by the message's serializer, without the type tag. The store never interprets it.
    pub payload: Value,
    /// Effective delivery time: `max(requested, store now at enqueue)`.
    pub deliver_at: Timestamp,
    /// Optional opaque string, passed through unchanged.
    pub correlation_id: Option<String>,
    /// String map for plugins (e.g. a trace id), passed through unchanged.
    pub headers: BTreeMap<String, String>,
    /// Store-assigned, strictly increasing across the whole store; the tie-breaker for equal `deliver_at`. A
    /// rescheduled message gets a new one.
    pub seq: i64,
    /// Number of times the message has been claimed; 0 while it was never claimed.
    pub attempt: u32,
    /// Number of failed handler runs that were retried under the message's `retry` policy.
    pub failures: u32,
    /// The error of the most recent failed run that was retried.
    pub last_error: Option<String>,
    /// The retry policy its sender attached, or `None` (no retry).
    pub retry: Option<RetryPolicy>,
    /// The id of the message whose turn sent this one; `None` for a message sent from outside an actor.
    pub caused_by: Option<MessageId>,
    /// How long the sender wants the message's record kept once it finished; `None` for the store default.
    pub retention: Option<Retention>,
    /// True for a notification of a watch ([`Failed`](crate::Failed) or [`Stopped`](crate::Stopped)), which only the
    /// store writes and marks; a sender cannot set it. A notification that ends `failed` notifies nobody.
    pub notification: bool,
}

/// When a new message becomes due. The store resolves it against its own clock, rounding up to microseconds; see
/// [`Delivery::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Delivery {
    /// Due at store now.
    #[default]
    Now,
    /// Due at the instant; a past instant is clamped to store now.
    At(Timestamp),
    /// Due at store now + the delay.
    After(Duration),
}

impl Delivery {
    /// Due at `instant`; fails when it is later than [`MAX_INSTANT`].
    pub fn at(instant: Timestamp) -> Result<Self, TimeError> {
        if instant > MAX_INSTANT {
            return Err(TimeError(format!(
                "deliverAt {} is later than the supported maximum {}",
                time::format_instant(instant),
                time::format_instant(MAX_INSTANT)
            )));
        }
        Ok(Delivery::At(instant))
    }

    /// Due after `delay`; fails when it is longer than [`MAX_DELAY`].
    pub fn after(delay: Duration) -> Result<Self, TimeError> {
        if delay > MAX_DELAY {
            return Err(TimeError(format!(
                "A delay must not exceed {}, was {}",
                time::format_duration(MAX_DELAY),
                time::format_duration(delay)
            )));
        }
        Ok(Delivery::After(delay))
    }

    /// Resolves the delivery against store time `now`: never earlier than `now`, rounded *up* to microseconds. Fails
    /// when the resolved time is later than [`MAX_INSTANT`].
    pub fn resolve(&self, now: Timestamp) -> Result<Timestamp, TimeError> {
        let resolved = match *self {
            Delivery::Now => now,
            Delivery::At(instant) => time::ceil(instant).max(now),
            Delivery::After(delay) => {
                if delay.is_zero() {
                    now
                } else {
                    time::ceil(time::add(now, delay)?)
                }
            }
        };
        if resolved > MAX_INSTANT {
            return Err(TimeError(format!(
                "Delivery time {} is later than the supported maximum {}",
                time::format_instant(resolved),
                time::format_instant(MAX_INSTANT)
            )));
        }
        Ok(resolved)
    }
}

/// A message to enqueue, before the store assigned `seq`, `attempt` and the effective delivery time.
#[derive(Debug, Clone, PartialEq)]
pub struct OutgoingMessage {
    /// The target instance.
    pub to: ActorAddress,
    /// The message type name.
    pub message_type: String,
    /// The payload.
    pub payload: Value,
    /// Dedup key; generate a fresh one unless retrying a send whose acknowledgement was lost.
    pub id: MessageId,
    /// The sending instance.
    pub from: Option<ActorAddress>,
    /// When it becomes due.
    pub delivery: Delivery,
    /// Opaque string passed through.
    pub correlation_id: Option<String>,
    /// Plugin headers passed through.
    pub headers: BTreeMap<String, String>,
    /// The sender's retry policy.
    pub retry: Option<RetryPolicy>,
    /// The message whose turn sends this one; the runtime sets it for every send of a turn.
    pub caused_by: Option<MessageId>,
    /// How long the record is kept once the message finished.
    pub retention: Option<Retention>,
}

impl OutgoingMessage {
    /// A message of `message_type` with `payload` for `to`, due now, with a fresh id and nothing else set.
    pub fn new(to: ActorAddress, message_type: impl Into<String>, payload: Value) -> Self {
        Self {
            to,
            message_type: message_type.into(),
            payload,
            id: MessageId::random(),
            from: None,
            delivery: Delivery::Now,
            correlation_id: None,
            headers: BTreeMap::new(),
            retry: None,
            caused_by: None,
            retention: None,
        }
    }

    /// The message as the store holds it once enqueued: due at `deliver_at`, with the store-assigned `seq`, never
    /// claimed and never failed; marked as a notification only when the store wrote it as one.
    pub fn into_message(self, deliver_at: Timestamp, seq: i64, notification: bool) -> Message {
        Message {
            id: self.id,
            to: self.to,
            from: self.from,
            message_type: self.message_type,
            payload: self.payload,
            deliver_at,
            correlation_id: self.correlation_id,
            headers: self.headers,
            seq,
            attempt: 0,
            failures: 0,
            last_error: None,
            retry: self.retry,
            caused_by: self.caused_by,
            retention: self.retention,
            notification,
        }
    }
}

/// Terminal status of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutcomeStatus {
    /// The handler returned; effects were applied.
    Done,
    /// The handler failed, the state could not be decoded, the commit was rejected, or the turn timed out.
    Failed,
    /// An administrator (or the actor itself) cancelled the message while it was pending, or an administrator stopped
    /// its running turn.
    Cancelled,
    /// The instance was stopped or deleted while the message was pending.
    Purged,
}

impl OutcomeStatus {
    /// Lower-case name used on the wire and in the store.
    pub fn wire_name(self) -> &'static str {
        match self {
            OutcomeStatus::Done => "done",
            OutcomeStatus::Failed => "failed",
            OutcomeStatus::Cancelled => "cancelled",
            OutcomeStatus::Purged => "purged",
        }
    }

    /// Looks up a status by its wire name.
    pub fn from_wire_name(name: &str) -> Option<Self> {
        match name {
            "done" => Some(OutcomeStatus::Done),
            "failed" => Some(OutcomeStatus::Failed),
            "cancelled" => Some(OutcomeStatus::Cancelled),
            "purged" => Some(OutcomeStatus::Purged),
            _ => None,
        }
    }
}

impl fmt::Display for OutcomeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire_name())
    }
}

/// How a commit ends the turn's message: with a terminal outcome ([`TurnOutcome::Done`], [`TurnOutcome::Failed`] or
/// [`TurnOutcome::Cancelled`]), or by putting it back into the mailbox ([`TurnOutcome::Rescheduled`]) without an outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// The turn succeeded: outcome `done`.
    Done,
    /// The turn failed: outcome `failed` with the error recorded.
    Failed(String),
    /// An operator stopped the running turn: outcome `cancelled`, no effects, no retry.
    Cancelled,
    /// The handler failed and the dispatcher retries the message under its retry policy: the *same* message goes back
    /// to pending, due at `delivery` resolved against store time, with a new `seq`, `failures` + 1 and `lastError` =
    /// `error`. It gets no outcome.
    Rescheduled {
        /// When the retry is due.
        delivery: Delivery,
        /// The failure, recorded as `lastError`.
        error: String,
    },
}

impl TurnOutcome {
    /// The longest `lastError` the runtime records with a reschedule; longer reasons are truncated.
    pub const MAX_ERROR_LENGTH: usize = 1_000;

    /// The terminal status, or `None` for a reschedule.
    pub fn status(&self) -> Option<OutcomeStatus> {
        match self {
            TurnOutcome::Done => Some(OutcomeStatus::Done),
            TurnOutcome::Failed(_) => Some(OutcomeStatus::Failed),
            TurnOutcome::Cancelled => Some(OutcomeStatus::Cancelled),
            TurnOutcome::Rescheduled { .. } => None,
        }
    }

    /// The error recorded with this outcome.
    pub fn error(&self) -> Option<&str> {
        match self {
            TurnOutcome::Done | TurnOutcome::Cancelled => None,
            TurnOutcome::Failed(e) | TurnOutcome::Rescheduled { error: e, .. } => Some(e),
        }
    }

    /// True for a terminal outcome.
    pub fn is_final(&self) -> bool {
        !matches!(self, TurnOutcome::Rescheduled { .. })
    }
}

/// A retained terminal outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeRecord {
    /// The message.
    pub message_id: MessageId,
    /// Its target.
    pub actor: ActorAddress,
    /// The terminal status.
    pub status: OutcomeStatus,
    /// The error of a failed message.
    pub error: Option<String>,
    /// Store time of the outcome.
    pub finished_at: Timestamp,
    /// The message as it was when it left the mailbox.
    pub envelope: Message,
}

/// Where a message is in its life cycle.
#[derive(Debug, Clone, PartialEq)]
pub enum MessageStatus {
    /// Waiting in the mailbox (possibly not yet due, rescheduled after a failure, or its last lease expired).
    Pending(Message),
    /// The head message of an instance whose lease is valid at store now: a turn is handling it.
    Running {
        /// The message.
        message: Message,
        /// The node holding the lease.
        node: NodeId,
        /// Lease expiry (store time).
        lease_until: Timestamp,
    },
    /// Left the mailbox with a retained outcome.
    Finished(OutcomeRecord),
}

impl MessageStatus {
    /// The message: as pending or running, or as it was when it left the mailbox.
    pub fn envelope(&self) -> &Message {
        match self {
            MessageStatus::Pending(m) | MessageStatus::Running { message: m, .. } => m,
            MessageStatus::Finished(o) => &o.envelope,
        }
    }
}

/// A turn granted by `claim_due`: the lease for `actor` and everything needed to run the handler.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    /// The claimed instance.
    pub actor: ActorAddress,
    /// Fencing token; pass it to renew, release and commit.
    pub epoch: i64,
    /// The head message, with `attempt` already incremented for this claim.
    pub message: Message,
    /// The instance's state.
    pub state: Value,
    /// The instance's version.
    pub version: i64,
    /// Store time at the claim; the handler's `now`.
    pub claimed_at: Timestamp,
    /// Store time at which the lease expires unless renewed.
    pub lease_until: Timestamp,
}

/// A snapshot of an instance.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorRecord {
    /// The instance.
    pub address: ActorAddress,
    /// Its state.
    pub state: Value,
    /// Starts at 0 and increases by 1 with every commit that writes state.
    pub version: i64,
    /// Store time of creation.
    pub created_at: Timestamp,
    /// True while the mailbox is paused: messages queue up, no new turn is claimed.
    pub paused: bool,
    /// The actor whose turn created this one as a child, or `None` for an independent actor. It never changes, and
    /// removing the parent removes this actor too.
    pub parent: Option<ActorAddress>,
}

/// What [`ActorSystem::stats`](crate::ActorSystem::stats) counts under an address prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreStats {
    /// The number of instances.
    pub actors: u64,
    /// The number of pending messages, including one being handled and delayed ones.
    pub pending: u64,
    /// The earliest `deliver_at` of a pending message that is due at store now; `None` when none is due.
    pub oldest_due: Option<Timestamp>,
}

/// The row of one node in the node table: display only, never used to decide who runs what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRecord {
    /// The node.
    pub id: NodeId,
    /// The names of the types it hosts; empty for a send-only node.
    pub hosted: Vec<TypeName>,
    /// Store time of its first heartbeat since it last had no row.
    pub started_at: Timestamp,
    /// Store time of its latest heartbeat.
    pub last_seen: Timestamp,
}

/// A turn in progress, as a snapshot: an instance with a lease that is valid at store now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningTurn {
    /// The instance.
    pub actor: ActorAddress,
    /// The message being handled.
    pub message_id: MessageId,
    /// The node holding the lease.
    pub node: NodeId,
    /// Lease expiry (store time).
    pub lease_until: Timestamp,
    /// True when an operator asked to cancel this turn ([`ActorSystem::cancel_turn`](crate::ActorSystem::cancel_turn)).
    pub cancel_requested: bool,
}

/// The published definition of an actor type.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    /// The type's name; its identity in the store.
    pub name: TypeName,
    /// The published contract (name, message type names, JSON Schemas). Opaque to the store.
    pub contract: Value,
    /// Stable hash of the contract; registrations conflict when it differs.
    pub contract_hash: String,
}

/// A create buffered by a turn and applied by the commit before the turn's sends.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorCreate {
    /// The instance to create.
    pub address: ActorAddress,
    /// Its initial state.
    pub state: Value,
    /// True for a child of the committing instance (`Turn::create_child`), false for an independent instance.
    pub child: bool,
}

/// What a control effect does to its target (ADR 0024): what the same call from outside a turn does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ControlEffect {
    /// No new turn is claimed for the target; its messages queue up, and a running turn finishes.
    Pause,
    /// Turns of the target are claimed again.
    Resume,
    /// Every pending message of the target ends `purged`, except one that is being handled; the target stays.
    Purge,
    /// The target, its mailbox and its descendants go; a turn running on one of them commits nothing.
    Delete,
}

impl ControlEffect {
    /// The lower-case name, as the node's log writes it.
    pub fn name(self) -> &'static str {
        match self {
            ControlEffect::Pause => "pause",
            ControlEffect::Resume => "resume",
            ControlEffect::Purge => "purge",
            ControlEffect::Delete => "delete",
        }
    }
}

impl fmt::Display for ControlEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A control effect of a turn on an actor, buffered by the turn and applied by its commit (ADR 0024). The actor need
/// not exist: then the effect does nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Control {
    /// What happens to the actor.
    pub effect: ControlEffect,
    /// The actor.
    pub actor: ActorAddress,
}

/// What a commit does to the committing instance's state.
#[derive(Debug, Clone, PartialEq)]
pub enum StateChange {
    /// Leave state and version untouched.
    Unchanged,
    /// Write the state and increase the version by 1.
    NewState(Value),
    /// Return the instance and every descendant to absent and purge their remaining mailboxes.
    Stop,
}

/// The single atomic write that ends a turn; see [`super::Backend::commit`].
#[derive(Debug, Clone, PartialEq)]
pub struct CommitRequest {
    /// The id of this event, drawn by the runtime for every commit: the store names the notifications the commit
    /// writes after it, `<event>.0`, `<event>.1`, … A repeated commit keeps it.
    pub event: MessageId,
    /// The committing instance.
    pub address: ActorAddress,
    /// The claim's epoch.
    pub epoch: i64,
    /// The claimed message.
    pub message_id: MessageId,
    /// How the message ends.
    pub outcome: TurnOutcome,
    /// What happens to the state.
    pub state: StateChange,
    /// Instances to create, after the deletes and before the sends.
    pub creates: Vec<ActorCreate>,
    /// Messages to enqueue, in order.
    pub sends: Vec<OutgoingMessage>,
    /// Pending messages of this instance to cancel.
    pub cancels: Vec<MessageId>,
    /// The watches the turn set and removed, with the committing instance as watcher, in the order of its calls;
    /// applied before the creates and the sends.
    pub watches: Vec<WatchChange>,
    /// The control effects of the turn: pauses, resumes, purges and deletes of other actors, and pauses, resumes and
    /// purges of its own. Applied by kind, in the order of [`super::Backend::commit`], not in the order of the calls.
    pub controls: Vec<Control>,
}

impl CommitRequest {
    /// A commit of `outcome` without effects, with a new random event id.
    pub fn new(address: ActorAddress, epoch: i64, message_id: MessageId, outcome: TurnOutcome) -> Self {
        Self {
            event: MessageId::random(),
            address,
            epoch,
            message_id,
            outcome,
            state: StateChange::Unchanged,
            creates: Vec::new(),
            sends: Vec::new(),
            cancels: Vec::new(),
            watches: Vec::new(),
            controls: Vec::new(),
        }
    }

    /// The addresses this commit deletes.
    pub fn deletes(&self) -> impl Iterator<Item = &ActorAddress> {
        self.controls.iter().filter(|c| c.effect == ControlEffect::Delete).map(|c| &c.actor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_deliveries() {
        let now: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        assert_eq!(Delivery::Now.resolve(now).unwrap(), now);
        let past: Timestamp = "2025-01-01T00:00:00Z".parse().unwrap();
        assert_eq!(Delivery::At(past).resolve(now).unwrap(), now);
        assert_eq!(
            Delivery::After(Duration::from_nanos(1)).resolve(now).unwrap(),
            time::from_micros(time::to_micros(now) + 1)
        );
        assert!(Delivery::at(time::from_micros(time::to_micros(MAX_INSTANT) + 1)).is_err());
        assert!(Delivery::after(MAX_DELAY + Duration::from_secs(1)).is_err());
        let near_end = time::from_micros(time::to_micros(MAX_INSTANT) - 10);
        assert!(Delivery::After(Duration::from_secs(1)).resolve(near_end).is_err());
    }

    #[test]
    fn ids_reject_blank() {
        assert!(MessageId::new(" ").is_err());
        assert!(serde_json::from_str::<MessageId>("\"\"").is_err());
        assert_eq!(MessageId::new("a").unwrap().as_str(), "a");
    }

    #[test]
    fn a_blank_id_is_an_id_error() {
        let e: IdError = MessageId::new("   ").unwrap_err();
        assert_eq!(e.to_string(), "MessageId must not be blank");
        assert_eq!(NodeId::try_from(String::new()).unwrap_err().to_string(), "NodeId must not be blank");
        let source: &dyn std::error::Error = &e;
        assert_eq!(anyhow::Error::from(e.clone()).to_string(), source.to_string());
    }
}
