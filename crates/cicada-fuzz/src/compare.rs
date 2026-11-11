//! Runs one list of store operations on two backends, one operation at a time on one virtual clock, and compares every
//! answer and the final state.
//!
//! What a backend picks itself or words itself does not count as a difference:
//!
//! - `seq` and `epoch` are compared by rank within each backend (see [`Ranker`]). Epochs are ranked per instance, as
//!   only per instance is their order promised (S4).
//! - Errors are compared by kind.
//! - The reasons of `Rejected` and `Conflict` are not compared. They are explanations for people.
//! - Each backend pages with its own cursors, as cursors are opaque and contain `seq`.
//! - Which instances one claim takes among those that are due, and in which order, is free: S6 promises an order per
//!   instance only. The backends would then hold different leases, so a claim always asks for every due instance
//!   ([`ALL_DUE`]), or for none, and the claims are compared sorted by address.
//! - Because of that, the epochs of different instances can come in another order in each backend. An `epoch_delta`
//!   therefore moves by rank among the epochs a backend handed to the same address, not by value: `-1` is the epoch
//!   handed just before the claim's, `1` the one just after, and where there is none, a number never handed to that
//!   address. It reaches the same claim of the same instance in both backends, or no claim in both.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use cicada::address::{ActorAddress, TypeName};
use cicada::retention::{Keep, Retention};
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::{
    ActorCreate, ActorRecord, CancelResult, Claim, CommitRequest, CommitResult, CreateResult, DeleteResult,
    DeliverNowResult, Delivery, EnqueueResult, Message, MessageId, MessageStatus, NodeId, OutcomeRecord, OutcomeStatus,
    OutgoingMessage, PageCursor, RegisterResult, RemoveTypeResult, RenewResult, RunningTurn, StateChange, Store,
    StoreError, StoreResult, TurnOutcome, TypeDef,
};
use cicada::time::{self, Timestamp};
use cicada_store_sqlite::{SqliteOptions, SqliteStore};
use cicada_test::{InMemoryStore, VirtualClock};
use proptest::prelude::*;
use serde_json::{Value, json};
use tempfile::TempDir;

/// The type names operations choose from. The last is not registered at the start.
pub const TYPES: [&str; 3] = ["alpha", "beta", "ghost"];

/// The addresses operations choose from: few, so that conflicts are common. `_` is a wildcard in SQL `LIKE`, and two
/// keys differ only there.
pub const ADDRESSES: [&str; 6] = ["alpha/0", "alpha/1", "alpha/a_b", "alpha/axb", "beta/0", "ghost/0"];

/// The prefixes operations choose from, including SQL wildcards.
pub const PREFIXES: [&str; 10] = ["", "alpha/", "alpha", "alpha/a_", "alpha/a", "b", "%", "alpha/%", "_", "zzz"];

/// How many message ids exist: enough that a list mostly sends new ones and sometimes repeats one.
pub const IDS: usize = 40;

/// The nodes operations choose from.
pub const NODES: [&str; 2] = ["n0", "n1"];

/// The cursors used to probe listings: well-formed, wrong and at the limits of the numbers.
pub const CURSORS: [&str; 10] = [
    "",
    "x",
    "1",
    ":",
    "1:2",
    "-1:-1",
    "1:2:alpha/0",
    "99999999999999999999:1",
    "9223372036854775807:9223372036854775807",
    "253402300799999999:1:alpha/0",
];

/// How a message is due, relative to the store clock when the operation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// Due now.
    Now,
    /// After a delay.
    After(Duration),
    /// At the instant this far ahead.
    AtAhead(Duration),
    /// At the instant this far back.
    AtBehind(Duration),
}

/// What a commit does to the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateOp {
    /// Left alone.
    Unchanged,
    /// Written, from the pool of [`STATES`].
    New(usize),
    /// Stopped.
    Stop,
}

/// How a commit ends the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// `done`.
    Done,
    /// `failed`.
    Failed,
    /// `cancelled`.
    Cancelled,
    /// Back to the mailbox.
    Rescheduled(Due),
}

/// A message to send.
#[derive(Debug, Clone)]
pub struct Send {
    /// Into [`ADDRESSES`].
    pub to: usize,
    /// The id: below [`IDS`].
    pub id: usize,
    /// When it is due.
    pub due: Due,
    /// Into [`PAYLOADS`].
    pub payload: usize,
    /// Into [`ADDRESSES`].
    pub from: Option<usize>,
    /// Counted back from the latest message sent (see [`Side::recent_id`]).
    pub caused_by: Option<usize>,
    /// Attach a retry policy.
    pub retry: bool,
    /// Into the pool of retention policies.
    pub retention: usize,
    /// Set a correlation id and a header.
    pub extras: bool,
}

/// A commit, addressed to one of the claims seen so far.
#[derive(Debug, Clone)]
pub struct Commit {
    /// Which claim, counted back from the latest.
    pub claim: usize,
    /// Moves the claim's epoch by rank among the epochs handed to its address, for a stale or a future one.
    pub epoch_delta: i64,
    /// Commit for another message than the claimed one; an id below [`IDS`].
    pub wrong_message: Option<usize>,
    /// How the message ends.
    pub end: End,
    /// What happens to the state.
    pub state: StateOp,
    /// Instances to create, as `(address, state)`.
    pub creates: Vec<(usize, usize)>,
    /// Messages to send.
    pub sends: Vec<Send>,
    /// Messages to cancel, counted back from the latest message sent.
    pub cancels: Vec<usize>,
}

/// One operation of a list. Indexes pick from the small pools above, so a shrunk list stays valid.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum Op {
    /// Moves the clock; both backends see it.
    Advance(Duration),
    Now,
    Register {
        ty: usize,
        variant: u8,
    },
    RemoveType {
        ty: usize,
    },
    ListTypes,
    Create {
        to: usize,
        state: usize,
    },
    Get {
        to: usize,
    },
    List {
        prefix: usize,
        limit: usize,
    },
    Delete {
        to: usize,
    },
    Enqueue(Send),
    ClaimDue {
        types: Vec<usize>,
        node: usize,
        ttl: Duration,
        limit: usize,
    },
    Renew {
        claim: usize,
        epoch_delta: i64,
        ttl: Duration,
    },
    Release {
        claim: usize,
        epoch_delta: i64,
    },
    NextDueAt {
        types: Vec<usize>,
    },
    Pending {
        to: usize,
        limit: usize,
    },
    ListPending {
        prefix: usize,
        limit: usize,
    },
    DeliverNow {
        id: usize,
    },
    Cancel {
        id: usize,
    },
    SetPaused {
        to: usize,
        paused: bool,
    },
    Purge {
        to: usize,
    },
    Status {
        id: usize,
    },
    Outcomes {
        status: Option<OutcomeStatus>,
        prefix: usize,
        since_back: Option<Duration>,
        limit: usize,
    },
    CausedBy {
        id: usize,
        limit: usize,
    },
    Stats {
        prefix: usize,
    },
    SweepExpired,
    RunningTurns {
        node: Option<usize>,
        prefix: usize,
        limit: usize,
    },
    RunningTurn {
        to: usize,
    },
    RequestCancel {
        to: usize,
    },
    Commit(Commit),
    /// One of the listings, started from a cursor of [`CURSORS`].
    ProbeCursor {
        listing: usize,
        cursor: usize,
        limit: usize,
    },
}

impl Op {
    /// The operation's name.
    pub fn name(&self) -> &'static str {
        match self {
            Op::Advance(_) => "advance",
            Op::Now => "now",
            Op::Register { .. } => "register",
            Op::RemoveType { .. } => "remove_type",
            Op::ListTypes => "list_types",
            Op::Create { .. } => "create",
            Op::Get { .. } => "get",
            Op::List { .. } => "list",
            Op::Delete { .. } => "delete",
            Op::Enqueue(_) => "enqueue",
            Op::ClaimDue { .. } => "claim_due",
            Op::Renew { .. } => "renew",
            Op::Release { .. } => "release",
            Op::NextDueAt { .. } => "next_due_at",
            Op::Pending { .. } => "pending",
            Op::ListPending { .. } => "list_pending",
            Op::DeliverNow { .. } => "deliver_now",
            Op::Cancel { .. } => "cancel",
            Op::SetPaused { .. } => "set_paused",
            Op::Purge { .. } => "purge",
            Op::Status { .. } => "status",
            Op::Outcomes { .. } => "outcomes",
            Op::CausedBy { .. } => "caused_by",
            Op::Stats { .. } => "stats",
            Op::SweepExpired => "sweep_expired",
            Op::RunningTurns { .. } => "running_turns",
            Op::RunningTurn { .. } => "running_turn",
            Op::RequestCancel { .. } => "request_cancel",
            Op::Commit(_) => "commit",
            Op::ProbeCursor { .. } => "probe_cursor",
        }
    }

    /// The names of every operation the generator can produce.
    pub const ALL: [&'static str; 30] = [
        "advance",
        "now",
        "register",
        "remove_type",
        "list_types",
        "create",
        "get",
        "list",
        "delete",
        "enqueue",
        "claim_due",
        "renew",
        "release",
        "next_due_at",
        "pending",
        "list_pending",
        "deliver_now",
        "cancel",
        "set_paused",
        "purge",
        "status",
        "outcomes",
        "caused_by",
        "stats",
        "sweep_expired",
        "running_turns",
        "running_turn",
        "request_cancel",
        "commit",
        "probe_cursor",
    ];
}

// ---- generators ----

fn address() -> impl Strategy<Value = usize> {
    0..ADDRESSES.len()
}

fn id() -> impl Strategy<Value = usize> {
    0..IDS
}

/// Counts back from the latest: usually something recent, sometimes anything.
fn recent() -> impl Strategy<Value = usize> {
    prop_oneof![6 => 0usize..4, 1 => 0usize..64]
}

fn prefix() -> impl Strategy<Value = usize> {
    0..PREFIXES.len()
}

fn limit() -> impl Strategy<Value = usize> {
    prop_oneof![1 => Just(0usize), 3 => Just(1usize), 3 => Just(2usize), 1 => Just(100usize)]
}

fn states() -> impl Strategy<Value = usize> {
    0..STATES.len()
}

fn ttl() -> BoxedStrategy<Duration> {
    prop_oneof![
        1 => Just(Duration::ZERO),
        1 => Just(Duration::from_micros(1)),
        3 => Just(Duration::from_secs(1)),
        12 => Just(Duration::from_secs(30)),
    ]
    .boxed()
}

fn step() -> BoxedStrategy<Duration> {
    prop_oneof![
        3 => Just(Duration::from_micros(1)),
        3 => Just(Duration::from_millis(1)),
        6 => Just(Duration::from_secs(1)),
        2 => Just(Duration::from_secs(5)),
        2 => Just(Duration::from_secs(31)),
        1 => Just(Duration::from_secs(301)),
    ]
    .boxed()
}

fn due() -> BoxedStrategy<Due> {
    let span = prop_oneof![
        1 => Just(Duration::from_nanos(1)),
        1 => Just(Duration::from_micros(1)),
        3 => Just(Duration::from_secs(1)),
        3 => Just(Duration::from_secs(10)),
    ];
    prop_oneof![
        4 => Just(Due::Now),
        3 => span.clone().prop_map(Due::After),
        1 => Just(Due::After(Duration::ZERO)),
        1 => span.clone().prop_map(Due::AtAhead),
        1 => span.prop_map(Due::AtBehind),
    ]
    .boxed()
}

fn send() -> BoxedStrategy<Send> {
    (
        address(),
        id(),
        due(),
        0..PAYLOADS.len(),
        prop::option::of(address()),
        prop::option::of(recent()),
        any::<bool>(),
        0..RETENTIONS,
        any::<bool>(),
    )
        .prop_map(|(to, id, due, payload, from, caused_by, retry, retention, extras)| Send {
            to,
            id,
            due,
            payload,
            from,
            caused_by,
            retry,
            retention,
            extras,
        })
        .boxed()
}

fn epoch_delta() -> impl Strategy<Value = i64> {
    prop_oneof![8 => Just(0i64), 1 => Just(-1i64), 1 => Just(1i64)]
}

fn commit() -> BoxedStrategy<Commit> {
    let end = prop_oneof![
        5 => Just(End::Done),
        2 => Just(End::Failed),
        1 => Just(End::Cancelled),
        3 => due().prop_map(End::Rescheduled),
    ];
    let state = prop_oneof![
        3 => Just(StateOp::Unchanged),
        3 => states().prop_map(StateOp::New),
        1 => Just(StateOp::Stop),
    ];
    (
        recent(),
        epoch_delta(),
        prop::option::weighted(0.05, id()),
        end,
        state,
        prop::collection::vec((address(), states()), 0..2),
        prop::collection::vec(send(), 0..3),
        prop::collection::vec(recent(), 0..2),
    )
        .prop_map(|(claim, epoch_delta, wrong_message, end, state, creates, sends, cancels)| Commit {
            claim,
            epoch_delta,
            wrong_message,
            end,
            state,
            creates,
            sends,
            cancels,
        })
        .boxed()
}

/// Mostly the registered types together, as a node hosts them; sometimes any.
fn type_set() -> impl Strategy<Value = Vec<usize>> {
    prop_oneof![8 => Just(vec![0usize, 1]), 2 => prop::collection::vec(0..TYPES.len(), 0..3)]
}

/// One operation. Writes that move mailboxes are weighted up, so that lists reach interesting states.
pub fn op() -> BoxedStrategy<Op> {
    prop_oneof![
        10 => step().prop_map(Op::Advance),
        1 => Just(Op::Now),
        1 => (0..TYPES.len(), 0u8..2).prop_map(|(ty, variant)| Op::Register { ty, variant }),
        1 => (0..TYPES.len()).prop_map(|ty| Op::RemoveType { ty }),
        1 => Just(Op::ListTypes),
        3 => (address(), states()).prop_map(|(to, state)| Op::Create { to, state }),
        2 => address().prop_map(|to| Op::Get { to }),
        2 => (prefix(), limit()).prop_map(|(prefix, limit)| Op::List { prefix, limit }),
        1 => address().prop_map(|to| Op::Delete { to }),
        16 => send().prop_map(Op::Enqueue),
        12 => (type_set(), 0..NODES.len(), ttl(), prop_oneof![1 => Just(0usize), 15 => Just(ALL_DUE)])
            .prop_map(|(types, node, ttl, limit)| Op::ClaimDue { types, node, ttl, limit }),
        3 => (recent(), epoch_delta(), ttl())
            .prop_map(|(claim, epoch_delta, ttl)| Op::Renew { claim, epoch_delta, ttl }),
        2 => (recent(), epoch_delta()).prop_map(|(claim, epoch_delta)| Op::Release { claim, epoch_delta }),
        1 => type_set().prop_map(|types| Op::NextDueAt { types }),
        2 => (address(), limit()).prop_map(|(to, limit)| Op::Pending { to, limit }),
        2 => (prefix(), limit()).prop_map(|(prefix, limit)| Op::ListPending { prefix, limit }),
        2 => recent().prop_map(|id| Op::DeliverNow { id }),
        2 => recent().prop_map(|id| Op::Cancel { id }),
        2 => (address(), any::<bool>()).prop_map(|(to, paused)| Op::SetPaused { to, paused }),
        1 => address().prop_map(|to| Op::Purge { to }),
        3 => recent().prop_map(|id| Op::Status { id }),
        2 => (
            prop::option::of(prop_oneof![
                Just(OutcomeStatus::Done),
                Just(OutcomeStatus::Failed),
                Just(OutcomeStatus::Cancelled),
                Just(OutcomeStatus::Purged),
            ]),
            prefix(),
            prop::option::of(step()),
            limit(),
        )
            .prop_map(|(status, prefix, since_back, limit)| Op::Outcomes { status, prefix, since_back, limit }),
        1 => (recent(), limit()).prop_map(|(id, limit)| Op::CausedBy { id, limit }),
        1 => prefix().prop_map(|prefix| Op::Stats { prefix }),
        2 => Just(Op::SweepExpired),
        1 => (prop::option::of(0..NODES.len()), prefix(), limit())
            .prop_map(|(node, prefix, limit)| Op::RunningTurns { node, prefix, limit }),
        1 => address().prop_map(|to| Op::RunningTurn { to }),
        2 => address().prop_map(|to| Op::RequestCancel { to }),
        12 => commit().prop_map(Op::Commit),
        1 => (0..LISTINGS, 0..CURSORS.len(), limit())
            .prop_map(|(listing, cursor, limit)| Op::ProbeCursor { listing, cursor, limit }),
    ]
    .boxed()
}

/// A list of operations.
pub fn ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op(), 1..120)
}

// ---- pools of values ----

/// A claim limit above the number of addresses: every due instance is claimed.
pub const ALL_DUE: usize = 100;

/// The states operations choose from.
pub const STATES: [fn() -> Option<Value>; 3] = [|| None, || Some(json!({})), || Some(json!({"n": 1, "text": "é"}))];

/// The payloads operations choose from, with values a backend might not round-trip.
pub const PAYLOADS: [fn() -> Value; 7] = [
    || json!({}),
    || json!({"b": 1, "a": 2}),
    || json!({"s": "é\u{0}\u{1F600}"}),
    || json!(null),
    || json!(1.5),
    || json!(9_007_199_254_740_993_i64),
    || json!([1e300, -0.0, {"nested": [true]}]),
];

const RETENTIONS: usize = 5;

fn retention(index: usize) -> Option<Retention> {
    match index {
        0 => None,
        1 => Some(Retention::REMOVE_ON_DONE),
        2 => Some(Retention::REMOVE_ON_FAILED),
        3 => Some(Retention { done: Some(Keep::For { age: Duration::from_secs(4) }), failed: None }),
        _ => Some(Retention { done: Some(Keep::Last { count: 1 }), failed: Some(Keep::Last { count: 1 }) }),
    }
}

fn retry_policy() -> RetryPolicy {
    RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(1)).expect("valid")).expect("valid")
}

fn resolve_due(due: Due, now: Timestamp) -> Delivery {
    match due {
        Due::Now => Delivery::Now,
        Due::After(d) => Delivery::After(d),
        Due::AtAhead(d) => Delivery::At(time::add(now, d).expect("in range")),
        Due::AtBehind(d) => Delivery::At(time::sub(now, d)),
    }
}

fn address_at(index: usize) -> ActorAddress {
    ActorAddress::parse(ADDRESSES[index]).expect("a valid address")
}

fn id_at(index: usize) -> MessageId {
    MessageId::new(format!("m{index}")).expect("a valid id")
}

fn node_at(index: usize) -> NodeId {
    NodeId::new(NODES[index]).expect("a valid id")
}

fn type_at(index: usize) -> TypeName {
    TypeName::parse(TYPES[index]).expect("a valid type")
}

fn types_of(indexes: &[usize]) -> BTreeSet<TypeName> {
    indexes.iter().map(|i| type_at(*i)).collect()
}

fn type_def(ty: usize, variant: u8) -> TypeDef {
    TypeDef { name: type_at(ty), contract: json!({"v": variant}), contract_hash: format!("hash-{variant}") }
}

// ---- rules ----

/// Replaces a number a backend picks itself by the order in which it was first seen, and checks its rank.
///
/// Two backends that pick different numbers still give the same tokens, as long as they reveal the numbers in the same
/// order. When a number is first seen, its token also holds its rank among the numbers seen so far, so a backend that
/// orders its numbers differently gives a different token.
#[derive(Debug, Default)]
pub struct Ranker {
    sorted: Vec<i64>,
    first_seen: HashMap<i64, usize>,
}

impl Ranker {
    /// The token of `value`.
    pub fn token(&mut self, value: i64) -> String {
        if let Some(index) = self.first_seen.get(&value) {
            return format!("#{index}");
        }
        let index = self.first_seen.len();
        self.first_seen.insert(value, index);
        let rank = self.sorted.partition_point(|v| *v < value);
        self.sorted.insert(rank, value);
        format!("#{index}@{rank}")
    }
}

/// The claims of one `claim_due` in the order they are compared and addressed by later operations.
pub fn in_address_order(mut claims: Vec<Claim>) -> Vec<Claim> {
    claims.sort_by(|a, b| a.actor.cmp(&b.actor));
    claims
}

/// The kind of a store error; its text is not compared.
pub fn error_kind(error: &StoreError) -> &'static str {
    match error {
        StoreError::Unavailable(_) => "unavailable",
        StoreError::Invalid(_) => "invalid",
        StoreError::Corrupt(_) => "corrupt",
    }
}

fn answer<T>(result: StoreResult<T>, normal: impl FnOnce(T) -> Value) -> Value {
    match result {
        Ok(value) => normal(value),
        Err(error) => json!({"error": error_kind(&error)}),
    }
}

fn instant(t: Timestamp) -> Value {
    json!(time::format_instant(t))
}

fn opt_instant(t: Option<Timestamp>) -> Value {
    t.map(instant).unwrap_or(Value::Null)
}

// ---- one backend ----

/// One backend and what must be kept per backend while a list runs.
pub struct Side {
    /// The backend.
    pub store: Arc<dyn Store>,
    /// Claims that no commit or release has used yet.
    claims: Vec<Claim>,
    /// Per address, the epochs of every claim, in the order they came.
    handed: HashMap<String, Vec<i64>>,
    sent: Vec<usize>,
    seq: Ranker,
    epochs: HashMap<String, Ranker>,
}

macro_rules! paged {
    ($side:ident, $start:expr, $fetch:expr, $item:expr) => {{
        let mut cursor: Option<PageCursor> = $start;
        let mut pages = Vec::new();
        loop {
            let page = $fetch(cursor.clone()).await;
            match page {
                Err(error) => {
                    pages.push(json!({"error": error_kind(&error)}));
                    break;
                }
                Ok(page) => {
                    let mut items = Vec::new();
                    for item in &page.items {
                        items.push($item($side, item));
                    }
                    pages.push(json!({"items": items, "more": page.next.is_some()}));
                    if page.next.is_none() {
                        break;
                    }
                    if pages.len() > 64 {
                        pages.push(json!("the listing did not end"));
                        break;
                    }
                    cursor = page.next;
                }
            }
        }
        Value::from(pages)
    }};
}

impl Side {
    /// A backend at the start of a list.
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            claims: Vec::new(),
            handed: HashMap::new(),
            sent: Vec::new(),
            seq: Ranker::default(),
            epochs: HashMap::new(),
        }
    }

    fn message(&mut self, m: &Message) -> Value {
        json!({
            "id": m.id.as_str(),
            "to": m.to.as_str(),
            "from": m.from.as_ref().map(|a| a.as_str().to_owned()),
            "type": m.message_type,
            "payload": m.payload,
            "deliverAt": instant(m.deliver_at),
            "correlationId": m.correlation_id,
            "headers": m.headers,
            "seq": self.seq.token(m.seq),
            "attempt": m.attempt,
            "failures": m.failures,
            "lastError": m.last_error,
            "retry": m.retry.as_ref().map(|r| r.to_json()),
            "causedBy": m.caused_by.as_ref().map(|i| i.as_str().to_owned()),
            "retention": m.retention.as_ref().map(|r| r.to_json()),
        })
    }

    fn outcome(&mut self, o: &OutcomeRecord) -> Value {
        json!({
            "id": o.message_id.as_str(),
            "actor": o.actor.as_str(),
            "status": o.status.wire_name(),
            "error": o.error,
            "finishedAt": instant(o.finished_at),
            "envelope": self.message(&o.envelope),
        })
    }

    fn status(&mut self, s: &MessageStatus) -> Value {
        match s {
            MessageStatus::Pending(m) => json!({"pending": self.message(m)}),
            MessageStatus::Running { message, node, lease_until } => {
                json!({"running": self.message(message), "node": node.as_str(), "leaseUntil": instant(*lease_until)})
            }
            MessageStatus::Finished(o) => json!({"finished": self.outcome(o)}),
        }
    }

    fn epoch(&mut self, address: &ActorAddress, epoch: i64) -> String {
        self.epochs.entry(address.to_string()).or_default().token(epoch)
    }

    fn claim(&mut self, c: &Claim) -> Value {
        json!({
            "actor": c.actor.as_str(),
            "epoch": self.epoch(&c.actor, c.epoch),
            "message": self.message(&c.message),
            "state": c.state,
            "version": c.version,
            "claimedAt": instant(c.claimed_at),
            "leaseUntil": instant(c.lease_until),
        })
    }

    fn turn(&mut self, t: &RunningTurn) -> Value {
        json!({
            "actor": t.actor.as_str(),
            "message": t.message_id.as_str(),
            "node": t.node.as_str(),
            "leaseUntil": instant(t.lease_until),
            "cancelRequested": t.cancel_requested,
        })
    }

    fn record(r: &ActorRecord) -> Value {
        json!({
            "address": r.address.as_str(),
            "state": r.state,
            "version": r.version,
            "createdAt": instant(r.created_at),
            "paused": r.paused,
        })
    }

    fn type_def(d: &TypeDef) -> Value {
        json!({"name": d.name.as_str(), "contract": d.contract, "hash": d.contract_hash})
    }

    /// The open claim an operation refers to, counted back from the latest, with its epoch moved by `delta`; `None`
    /// when no claim is open. A claim stays open until a commit or a release has used it, so later operations reach a
    /// turn that is still running. Its lease can still have expired: stale epochs come from that and from `delta`.
    ///
    /// `delta` moves by rank among the epochs handed to the claim's address. Past the first or the last of them it
    /// moves on from there by value, to a number never handed to that address.
    fn claimed(&self, back: usize, delta: i64) -> Option<(ActorAddress, i64, MessageId)> {
        if self.claims.is_empty() {
            return None;
        }
        let claim = &self.claims[self.claims.len() - 1 - back % self.claims.len()];
        let handed = &self.handed[claim.actor.as_str()];
        let rank = handed.iter().position(|e| *e == claim.epoch).expect("an open claim's epoch was handed") as i64;
        let (to, last) = (rank + delta, handed.len() as i64 - 1);
        let epoch = if to < 0 {
            handed[0] + to
        } else if to > last {
            handed[last as usize] + (to - last)
        } else {
            handed[to as usize]
        };
        Some((claim.actor.clone(), epoch, claim.message.id.clone()))
    }

    /// The id of a message sent earlier in the list, counted back from the latest; while nothing was sent, some id.
    pub fn recent_id(&self, back: usize) -> MessageId {
        match self.sent.len() {
            0 => id_at(back % IDS),
            sent => id_at(self.sent[sent - 1 - back % sent]),
        }
    }

    fn outgoing(&mut self, send: &Send, now: Timestamp) -> OutgoingMessage {
        let mut message = OutgoingMessage::new(address_at(send.to), "M", PAYLOADS[send.payload]());
        message.id = id_at(send.id);
        message.delivery = resolve_due(send.due, now);
        message.from = send.from.map(address_at);
        message.caused_by = send.caused_by.map(|back| self.recent_id(back));
        message.retry = send.retry.then(retry_policy);
        message.retention = retention(send.retention);
        if send.extras {
            message.correlation_id = Some("corr".to_owned());
            message.headers = BTreeMap::from([("h".to_owned(), "v".to_owned())]);
        }
        self.sent.push(send.id);
        message
    }

    async fn listing(&mut self, listing: usize, cursor: Option<PageCursor>, prefix: &str, limit: usize) -> Value {
        let store = self.store.clone();
        let store = &*store;
        let target = address_at(0);
        let id = id_at(0);
        let epoch = time::from_micros(0);
        let this = &mut *self;
        match listing {
            0 => paged!(
                this,
                cursor,
                |c: Option<PageCursor>| async move { store.list(prefix, c.as_ref(), limit).await },
                |_: &mut Side, a: &ActorAddress| json!(a.as_str())
            ),
            1 => {
                let target = &target;
                paged!(
                    this,
                    cursor,
                    |c: Option<PageCursor>| async move { store.pending(target, c.as_ref(), limit).await },
                    |s: &mut Side, m: &Message| s.message(m)
                )
            }
            2 => paged!(
                this,
                cursor,
                |c: Option<PageCursor>| async move { store.list_pending(prefix, c.as_ref(), limit).await },
                |s: &mut Side, m: &Message| s.message(m)
            ),
            3 => paged!(
                this,
                cursor,
                |c: Option<PageCursor>| async move { store.outcomes(None, prefix, epoch, c.as_ref(), limit).await },
                |s: &mut Side, o: &OutcomeRecord| s.outcome(o)
            ),
            4 => {
                let id = &id;
                paged!(
                    this,
                    cursor,
                    |c: Option<PageCursor>| async move { store.caused_by(id, c.as_ref(), limit).await },
                    |s: &mut Side, m: &MessageStatus| s.status(m)
                )
            }
            _ => paged!(
                this,
                cursor,
                |c: Option<PageCursor>| async move { store.running_turns(None, prefix, c.as_ref(), limit).await },
                |s: &mut Side, t: &RunningTurn| s.turn(t)
            ),
        }
    }

    /// Runs `op` at store time `now` and returns its answer in the form that is compared.
    pub async fn apply(&mut self, op: &Op, now: Timestamp) -> Value {
        let store = self.store.clone();
        let store = &*store;
        match op {
            Op::Advance(_) => Value::Null,
            Op::Now => answer(store.now().await, instant),
            Op::Register { ty, variant } => answer(store.register(type_def(*ty, *variant)).await, |r| match r {
                RegisterResult::Registered(d) => json!({"registered": Self::type_def(&d)}),
                RegisterResult::Conflict { existing, .. } => json!({"conflict": Self::type_def(&existing)}),
            }),
            Op::RemoveType { ty } => answer(store.remove_type(&type_at(*ty)).await, |r| match r {
                RemoveTypeResult::Removed => json!("removed"),
                RemoveTypeResult::Absent => json!("absent"),
                RemoveTypeResult::InUse => json!("in-use"),
            }),
            Op::ListTypes => answer(store.list_types().await, |types| {
                Value::from(types.iter().map(Self::type_def).collect::<Vec<_>>())
            }),
            Op::Create { to, state } => answer(store.create(&address_at(*to), STATES[*state]()).await, |r| match r {
                CreateResult::Created => json!("created"),
                CreateResult::Exists(record) => json!({"exists": Self::record(&record)}),
                CreateResult::NoSuchType => json!("no-such-type"),
            }),
            Op::Get { to } => answer(store.get(&address_at(*to)).await, |r| match r {
                Some(record) => Self::record(&record),
                None => Value::Null,
            }),
            Op::List { prefix, limit } => self.listing(0, None, PREFIXES[*prefix], *limit).await,
            Op::Delete { to } => answer(store.delete(&address_at(*to)).await, |r| match r {
                DeleteResult::Deleted { purged } => json!({"deleted": purged}),
                DeleteResult::Absent => json!("absent"),
            }),
            Op::Enqueue(send) => match store.enqueue(self.outgoing(send, now)).await {
                Ok(EnqueueResult::Enqueued(m)) => json!({"enqueued": self.message(&m)}),
                Ok(EnqueueResult::Duplicate(id)) => json!({"duplicate": id.as_str()}),
                Ok(EnqueueResult::NoSuchInstance) => json!("no-such-instance"),
                Err(error) => json!({"error": error_kind(&error)}),
            },
            Op::ClaimDue { types, node, ttl, limit } => {
                match store.claim_due(&types_of(types), &node_at(*node), *ttl, *limit).await {
                    Ok(claims) => {
                        let claims = in_address_order(claims);
                        let mut answers = Vec::new();
                        for claim in &claims {
                            answers.push(self.claim(claim));
                            self.handed.entry(claim.actor.to_string()).or_default().push(claim.epoch);
                        }
                        self.claims.extend(claims);
                        Value::from(answers)
                    }
                    Err(error) => json!({"error": error_kind(&error)}),
                }
            }
            Op::Renew { claim, epoch_delta, ttl } => match self.claimed(*claim, *epoch_delta) {
                None => json!("no claim"),
                Some((address, epoch, _)) => answer(store.renew(&address, epoch, *ttl).await, |r| match r {
                    RenewResult::Renewed(t) => json!({"renewed": instant(t)}),
                    RenewResult::CancelRequested(t) => json!({"cancel-requested": instant(t)}),
                    RenewResult::LeaseLost => json!("lease-lost"),
                }),
            },
            Op::Release { claim, epoch_delta } => match self.claimed(*claim, *epoch_delta) {
                None => json!("no claim"),
                Some((address, epoch, _)) => {
                    let released = answer(store.release(&address, epoch).await, |()| json!("released"));
                    self.close_claim(&address, epoch);
                    released
                }
            },
            Op::NextDueAt { types } => answer(store.next_due_at(&types_of(types)).await, opt_instant),
            Op::Pending { to, limit } => {
                let store_ref = store;
                let target = address_at(*to);
                let target = &target;
                let this = &mut *self;
                paged!(
                    this,
                    None,
                    |c: Option<PageCursor>| async move { store_ref.pending(target, c.as_ref(), *limit).await },
                    |s: &mut Side, m: &Message| s.message(m)
                )
            }
            Op::ListPending { prefix, limit } => self.listing(2, None, PREFIXES[*prefix], *limit).await,
            Op::DeliverNow { id } => match store.deliver_now(&self.recent_id(*id)).await {
                Ok(DeliverNowResult::Delivered(m)) => json!({"delivered": self.message(&m)}),
                Ok(DeliverNowResult::Running) => json!("running"),
                Ok(DeliverNowResult::AlreadyFinished(o)) => json!({"finished": self.outcome(&o)}),
                Ok(DeliverNowResult::NotFound) => json!("not-found"),
                Err(error) => json!({"error": error_kind(&error)}),
            },
            Op::Cancel { id } => match store.cancel(&self.recent_id(*id)).await {
                Ok(CancelResult::Cancelled(o)) => json!({"cancelled": self.outcome(&o)}),
                Ok(CancelResult::Running) => json!("running"),
                Ok(CancelResult::AlreadyFinished(o)) => json!({"finished": self.outcome(&o)}),
                Ok(CancelResult::NotFound) => json!("not-found"),
                Err(error) => json!({"error": error_kind(&error)}),
            },
            Op::SetPaused { to, paused } => answer(store.set_paused(&address_at(*to), *paused).await, |r| json!(r)),
            Op::Purge { to } => answer(store.purge(&address_at(*to)).await, |r| json!(r)),
            Op::Status { id } => match store.status(&self.recent_id(*id)).await {
                Ok(Some(status)) => self.status(&status),
                Ok(None) => Value::Null,
                Err(error) => json!({"error": error_kind(&error)}),
            },
            Op::Outcomes { status, prefix, since_back, limit } => {
                let store_ref = store;
                let prefix = PREFIXES[*prefix];
                let since = since_back.map_or(time::from_micros(0), |back| time::sub(now, back));
                let this = &mut *self;
                paged!(
                    this,
                    None,
                    |c: Option<PageCursor>| async move {
                        store_ref.outcomes(*status, prefix, since, c.as_ref(), *limit).await
                    },
                    |s: &mut Side, o: &OutcomeRecord| s.outcome(o)
                )
            }
            Op::CausedBy { id, limit } => {
                let store_ref = store;
                let id = self.recent_id(*id);
                let id = &id;
                let this = &mut *self;
                paged!(
                    this,
                    None,
                    |c: Option<PageCursor>| async move { store_ref.caused_by(id, c.as_ref(), *limit).await },
                    |s: &mut Side, m: &MessageStatus| s.status(m)
                )
            }
            Op::Stats { prefix } => answer(
                store.stats(PREFIXES[*prefix]).await,
                |s| json!({"actors": s.actors, "pending": s.pending, "oldestDue": opt_instant(s.oldest_due)}),
            ),
            Op::SweepExpired => answer(store.sweep_expired().await, |n| json!(n)),
            Op::RunningTurns { node, prefix, limit } => {
                let store_ref = store;
                let node = node.map(node_at);
                let node = node.as_ref();
                let prefix = PREFIXES[*prefix];
                let this = &mut *self;
                paged!(
                    this,
                    None,
                    |c: Option<PageCursor>| async move { store_ref.running_turns(node, prefix, c.as_ref(), *limit).await },
                    |s: &mut Side, t: &RunningTurn| s.turn(t)
                )
            }
            Op::RunningTurn { to } => match store.running_turn(&address_at(*to)).await {
                Ok(Some(turn)) => self.turn(&turn),
                Ok(None) => Value::Null,
                Err(error) => json!({"error": error_kind(&error)}),
            },
            Op::RequestCancel { to } => answer(store.request_cancel(&address_at(*to)).await, |r| json!(r)),
            Op::Commit(commit) => self.commit(commit, now).await,
            Op::ProbeCursor { listing, cursor, limit } => {
                self.listing(*listing, Some(PageCursor(CURSORS[*cursor].to_owned())), "", *limit).await
            }
        }
    }

    /// Closes the open claim of `address` with the epoch that was sent: through [`Side::claimed`], the same claim in
    /// both backends, or none in both.
    fn close_claim(&mut self, address: &ActorAddress, epoch: i64) {
        self.claims.retain(|c| !(c.actor == *address && c.epoch == epoch));
    }

    async fn commit(&mut self, commit: &Commit, now: Timestamp) -> Value {
        let Some((address, epoch, claimed)) = self.claimed(commit.claim, commit.epoch_delta) else {
            return json!("no claim");
        };
        let outcome = match &commit.end {
            End::Done => TurnOutcome::Done,
            End::Failed => TurnOutcome::Failed("boom".to_owned()),
            End::Cancelled => TurnOutcome::Cancelled,
            End::Rescheduled(due) => {
                TurnOutcome::Rescheduled { delivery: resolve_due(*due, now), error: "again".to_owned() }
            }
        };
        let sends: Vec<_> = commit.sends.iter().map(|send| self.outgoing(send, now)).collect();
        let cancels: Vec<_> = commit.cancels.iter().map(|back| self.recent_id(*back)).collect();
        let request = CommitRequest {
            address: address.clone(),
            epoch,
            message_id: commit.wrong_message.map_or(claimed, id_at),
            outcome,
            state: match commit.state {
                StateOp::Unchanged => StateChange::Unchanged,
                StateOp::New(state) => StateChange::NewState(STATES[state]()),
                StateOp::Stop => StateChange::Stop,
            },
            creates: commit
                .creates
                .iter()
                .map(|(to, state)| ActorCreate { address: address_at(*to), state: STATES[*state]() })
                .collect(),
            sends,
            cancels,
        };
        match self.store.commit(request).await {
            Ok(CommitResult::Committed { version, sent, cancelled }) => {
                let mut sent_answers = Vec::new();
                for message in &sent {
                    sent_answers.push(self.message(message));
                }
                let cancelled: Vec<_> = cancelled.iter().map(|id| id.as_str().to_owned()).collect();
                self.close_claim(&address, epoch);
                json!({"committed": {"version": version, "sent": sent_answers, "cancelled": cancelled}})
            }
            Ok(CommitResult::LeaseLost) => {
                self.close_claim(&address, epoch);
                json!("lease-lost")
            }
            Ok(CommitResult::Rejected(_)) => json!("rejected"),
            Err(error) => json!({"error": error_kind(&error)}),
        }
    }

    /// Everything the backend holds, in the form that is compared.
    pub async fn final_state(&mut self, now: Timestamp) -> Value {
        let mut state = serde_json::Map::new();
        let steps: Vec<(&str, Op)> = vec![
            ("types", Op::ListTypes),
            ("actors", Op::List { prefix: 0, limit: 2 }),
            ("pending", Op::ListPending { prefix: 0, limit: 2 }),
            ("outcomes", Op::Outcomes { status: None, prefix: 0, since_back: None, limit: 2 }),
            ("stats", Op::Stats { prefix: 0 }),
            ("nextDue", Op::NextDueAt { types: vec![0, 1, 2] }),
            ("running", Op::RunningTurns { node: None, prefix: 0, limit: 2 }),
        ];
        for (name, op) in steps {
            state.insert(name.to_owned(), self.apply(&op, now).await);
        }
        let mut records = Vec::new();
        for to in 0..ADDRESSES.len() {
            records.push(self.apply(&Op::Get { to }, now).await);
        }
        state.insert("records".to_owned(), Value::from(records));
        let mut statuses = Vec::new();
        for id in 0..IDS {
            let answer = match self.store.status(&id_at(id)).await {
                Ok(Some(status)) => self.status(&status),
                Ok(None) => Value::Null,
                Err(error) => json!({"error": error_kind(&error)}),
            };
            statuses.push(answer);
        }
        state.insert("statuses".to_owned(), Value::from(statuses));
        Value::Object(state)
    }
}

const LISTINGS: usize = 6;

// ---- two backends ----

/// Two backends disagree.
#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    /// What was compared: the number and name of the operation, or `final state`.
    pub place: String,
    /// The answer of the first backend.
    pub first: Value,
    /// The answer of the second backend.
    pub second: Value,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "backends differ at {}\n  first:  {}\n  second: {}", self.place, self.first, self.second)
    }
}

/// Two backends on one virtual clock, each with the same starting content.
pub struct Pair {
    /// The clock of both.
    pub clock: Arc<VirtualClock>,
    /// The first backend.
    pub first: Side,
    /// The second backend.
    pub second: Side,
    _dir: Option<TempDir>,
}

const RETENTION: Duration = Duration::from_secs(300);

impl Pair {
    /// `InMemoryStore` and `SqliteStore`.
    pub async fn memory_and_sqlite() -> Self {
        let clock = Arc::new(VirtualClock::default());
        let memory = Arc::new(InMemoryStore::new(clock.clone(), RETENTION));
        let dir = tempfile::Builder::new().prefix("cicada-fuzz").tempdir().expect("a temp dir");
        let options = SqliteOptions { clock: clock.clone(), retention: RETENTION, ..SqliteOptions::default() };
        let sqlite = SqliteStore::open(dir.path().join("actors.db"), options).await.expect("the store opens");
        Self::start(clock, memory, Arc::new(sqlite), Some(dir)).await
    }

    /// Two `InMemoryStore`s, to test the comparison itself.
    pub async fn memory_twice() -> Self {
        let clock = Arc::new(VirtualClock::default());
        let first = Arc::new(InMemoryStore::new(clock.clone(), RETENTION));
        let second = Arc::new(InMemoryStore::new(clock.clone(), RETENTION));
        Self::start(clock, first, second, None).await
    }

    /// Any two stores on `clock`.
    pub async fn of(clock: Arc<VirtualClock>, first: Arc<dyn Store>, second: Arc<dyn Store>) -> Self {
        Self::start(clock, first, second, None).await
    }

    async fn start(
        clock: Arc<VirtualClock>,
        first: Arc<dyn Store>,
        second: Arc<dyn Store>,
        dir: Option<TempDir>,
    ) -> Self {
        let mut pair = Self { clock, first: Side::new(first), second: Side::new(second), _dir: dir };
        let setup = [
            Op::Register { ty: 0, variant: 0 },
            Op::Register { ty: 1, variant: 0 },
            Op::Create { to: 0, state: 1 },
            Op::Create { to: 1, state: 0 },
            Op::Create { to: 2, state: 1 },
            Op::Create { to: 3, state: 1 },
            Op::Create { to: 4, state: 2 },
        ];
        for op in &setup {
            pair.step(0, op).await.expect("the backends agree on the setup");
        }
        pair
    }

    /// Runs one operation on both backends and compares the answers.
    pub async fn step(&mut self, number: usize, op: &Op) -> Result<(), Box<Difference>> {
        if let Op::Advance(by) = op {
            self.clock.advance_by(*by);
            return Ok(());
        }
        let now = cicada::store::Clock::now(&*self.clock);
        let first = self.first.apply(op, now).await;
        let second = self.second.apply(op, now).await;
        if first == second {
            return Ok(());
        }
        Err(Box::new(Difference { place: format!("operation {number} ({op:?})"), first, second }))
    }

    /// Compares what both backends hold.
    pub async fn compare_final_state(&mut self) -> Result<(), Box<Difference>> {
        let now = cicada::store::Clock::now(&*self.clock);
        let first = self.first.final_state(now).await;
        let second = self.second.final_state(now).await;
        if first == second {
            return Ok(());
        }
        Err(Box::new(Difference { place: "final state".to_owned(), first, second }))
    }

    /// Runs the whole list and compares the final state.
    pub async fn run(mut self, ops: &[Op]) -> Result<(), Box<Difference>> {
        let result = async {
            for (number, op) in ops.iter().enumerate() {
                self.step(number, op).await?;
            }
            self.compare_final_state().await
        }
        .await;
        self.first.store.close().await;
        self.second.store.close().await;
        result
    }
}
