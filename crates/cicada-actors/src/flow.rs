//! Durable flows (ADR 0031, ADR 0044): a multi-step job written as ordinary async code. Each run is one actor that
//! replays the results its journal holds and executes one new step, sleep, send, request or wait per turn, saving it
//! and arming the next `Continue` in the same commit; a sleep's `Continue` is due at its saved wake time, a send's or a
//! request's create and message commit with its entry, and a wait for replies that have not come parks the run with no
//! `Continue` until a `Reply` turn closes it, or, with a deadline, arms a `Continue` due then that closes it as timed
//! out. A wait names one request or joins up to 100, for all their replies or the first. A `Cancel` ends a running run
//! in one turn of its own: it cancels the armed `Continue`, marks the run cancelled and keeps its progress; the wire
//! also holds the cleanup of a cancelling run for a later release, which this one refuses.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp};
use cicada::turn::TurnError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::supervisor::retry_policy;

/// The most entries a journal may hold.
const MAX_ENTRIES: usize = 1_000;
/// The most replies a run keeps: one per request, of at most 2000 sends.
const MAX_REPLIES: usize = 2_000;
/// The most requests a wait names.
const MAX_WAIT_REQUESTS: usize = 100;
/// The longest step id, in characters.
const MAX_ID_LENGTH: usize = 100;
/// The deepest result: the state, the journal and the entry add three levels to the core's 64.
const MAX_RESULT_DEPTH: usize = 61;

/// A flow: a host-named actor type whose runs execute [`Flow::run`] one step at a time.
///
/// `run` is replayed from the top on every execution, so the code outside step closures must be deterministic and
/// free of external effects: it may only decide which step comes next from the input and earlier results. Each
/// closure executes at least once; it receives a stable idempotency key, `<run address>/<step id>`. A changed step
/// order, id or result type can break the replay of running runs without changing the contract: drain them first, or
/// give the new code a new type name.
///
/// ```
/// use cicada::prelude::*;
/// use cicada_actors::{Flow, FlowRun, Step};
///
/// # async fn create_account(user: &str, key: &str) -> Result<String, std::io::Error> { Ok(format!("a-{user}")) }
/// # async fn send_welcome(account: &str, key: &str) -> Result<(), std::io::Error> { Ok(()) }
/// /// No Clone and no Debug: a flow holds its clients.
/// struct Onboarding;
///
/// impl Flow for Onboarding {
///     const TYPE: &'static str = "onboarding";
///     type Input = String;
///
///     async fn run(&self, step: &mut Step, user: String) -> HandlerResult {
///         let account: String = step.run("account", |key| async move { Ok(create_account(&user, &key).await?) }).await?;
///         step.run("welcome", |key| async move { Ok(send_welcome(&account, &key).await?) }).await?;
///         Ok(())
///     }
/// }
///
/// let run = FlowRun::<Onboarding>::named("u1");
/// assert_eq!(run.address()?.as_str(), "onboarding/u1");
/// assert_eq!(format!("{:?}", run.clone()), r#"FlowRun("onboarding/u1")"#);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait Flow: Send + Sync + 'static {
    /// The type name of the runs, chosen by the host; unlike the fixed names of the other actors of this crate.
    const TYPE: &'static str;
    /// The input of a run, fixed at its create; its schema is part of the contract.
    type Input: Serialize + DeserializeOwned + JsonSchema + Clone + Send + Sync + 'static;

    /// The retry policy of every `Continue` the runner sends itself, and so of every step and of a send's check of the
    /// registry; by default the crate's shared policy (7 attempts, exponential from 1 s to 32 s). A message of
    /// [`Step::send`] has none.
    fn retry() -> RetryPolicy {
        retry_policy()
    }

    /// The flow's code. It calls [`Step::run`] for each external step, [`Step::sleep`] for each durable sleep,
    /// [`Step::send`] for each message to an actor, [`Step::request`] for each message that a reply answers, and
    /// [`Step::wait`] or [`Step::wait_timeout`] for that reply, or a join such as [`Step::wait_all`] or
    /// [`Step::wait_first`] for several, and passes their errors on with `?`; it returns `Ok(())` once every step is
    /// done.
    fn run(&self, step: &mut Step, input: Self::Input) -> impl Future<Output = HandlerResult> + Send;
}

/// A run of the flow `F`, `<F::TYPE>/<key>`: a typed ref whose one string is the key. It needs no `Clone` or `Debug`
/// of `F`.
#[derive(Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct FlowRun<F> {
    /// The key of the run; never reused for another job, also after a delete, as its step keys would repeat.
    pub key: String,
    #[serde(skip)]
    flow: PhantomData<fn() -> F>,
}

impl<F> FlowRun<F> {
    /// The run keyed `key`.
    pub fn named(key: impl Into<String>) -> Self {
        Self { key: key.into(), flow: PhantomData }
    }
}

impl<F> Clone for FlowRun<F> {
    fn clone(&self) -> Self {
        Self::named(self.key.clone())
    }
}

impl<F: Flow> fmt::Debug for FlowRun<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.address() {
            Ok(address) => f.debug_tuple("FlowRun").field(&address.as_str()).finish(),
            Err(_) => f.debug_tuple("FlowRun").field(&self.key).finish(),
        }
    }
}

impl<F: Flow> ActorRef for FlowRun<F> {
    const TYPE: &'static str = F::TYPE;
    type Message = FlowMsg;
}

/// The messages of a [`FlowRun`]. A host or a worker of 0.18.3 or earlier refuses `Cancel` for good, although the
/// contract is the same (ADR 0044): upgrade every host and worker of a flow type before sending it. `Reply` is what
/// [`ReplyTo::send`] sends; a host or a worker of 0.18.0 refuses it for good, so upgrade them all before a flow requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum FlowMsg {
    /// Starts or recovers a running run; a done or cancelled run ignores it. Sent by a host, it cancels the armed
    /// continuation and arms a fresh one with the flow's retry policy, without running flow code; the armed one replays
    /// the flow and executes its next step.
    Continue,
    /// The reply of a target to a request of the run, sent with [`ReplyTo::send`]. The run keeps the first one from the
    /// request's target and ignores any other, without running flow code.
    Reply {
        /// The id of the request's message, saved with its send entry.
        request: MessageId,
        /// The reply's JSON, `null` included; required, so a missing value is not `null`.
        value: Value,
    },
    /// Ends a running run, without running flow code: in its turn's one commit it cancels the armed `Continue`, marks
    /// the run cancelled and clears `armed`, keeping the input, the journal and the replies; a done or cancelled run
    /// ignores it. It waits for the turn before it in the mailbox, does not undo effects already made and runs no
    /// cleanup. Sending it only queues it: read the run's status to learn that it was cancelled.
    Cancel,
}

/// The phase of a run. This release executes running runs and cancels them; `cancelling` is on the wire for a later
/// release (ADR 0044).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FlowStatus {
    /// Steps remain, or the flow has not yet returned.
    #[default]
    Running,
    /// A cancelled run whose cleanup has not finished; refused for good by this release.
    Cancelling,
    /// The flow returned `Ok` after replaying every saved entry; the run keeps its state until it is deleted.
    Done,
    /// A run ended by `Cancel`; it keeps its state until it is deleted, and ignores `Continue` and `Reply`.
    Cancelled,
}

/// How a send entry created its target before its message: as an independent actor or as a child of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CreateKind {
    /// Created with no parent, as [`Turn::create_raw`] does: an actor that exists already is left untouched.
    Independent,
    /// Created as a child of the run, as [`Turn::create_child_raw`] does: an actor owned elsewhere rejects the commit.
    Child,
}

/// Whether a wait needs every listed reply or the first one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum WaitMode {
    /// Every request's reply.
    All,
    /// The first accepted reply of any request.
    First,
}

/// How a wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum WaitOutcome {
    /// The replies it needed arrived.
    Received,
    /// Its deadline passed first.
    TimedOut,
}

/// A reply a run kept for a request of one of its send entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowReply {
    /// The id of the request's message.
    pub request: MessageId,
    /// The reply's JSON, `null` included; required, so a missing value is not `null`.
    pub value: Value,
}

/// One saved entry of a journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JournalEntry {
    /// The saved result of a step.
    Step {
        /// The step id: 1 through 100 characters of `A-Za-z0-9._~-:`, unique in its journal.
        #[schemars(length(min = 1, max = 100), regex(pattern = r"^[A-Za-z0-9._~:-]+$"))]
        id: String,
        /// The JSON result of its closure, `null` included.
        result: Value,
    },
    /// A durable sleep of [`Step::sleep`] until a store time: the flow goes on in the first turn at or after it.
    Sleep {
        /// The sleep id, from the same namespace as the step ids.
        #[schemars(length(min = 1, max = 100), regex(pattern = r"^[A-Za-z0-9._~:-]+$"))]
        id: String,
        /// When the sleep ends; at most `MAX_INSTANT`.
        until: Timestamp,
    },
    /// A message of [`Step::send`], enqueued in the commit that saved this entry, after the create of its target if
    /// any. Neither the message nor the initial state is saved.
    #[serde(rename_all = "camelCase")]
    Send {
        /// The send id, from the same namespace as the step ids.
        #[schemars(length(min = 1, max = 100), regex(pattern = r"^[A-Za-z0-9._~:-]+$"))]
        id: String,
        /// Where the message went.
        to: ActorAddress,
        /// The id of the message; unique among the sends of both journals.
        message: MessageId,
        /// How the target was created in the same commit; absent or null: not created.
        #[serde(default)]
        create: Option<CreateKind>,
        /// Whether the message is a request of [`Step::request`] that a reply answers; absent: false.
        #[serde(default)]
        reply: bool,
    },
    /// A wait for the replies of earlier requests: one request, or a join of several or of mode first, without or
    /// with a deadline. Its saved selection, not the replies kept later, is what replay returns.
    #[serde(rename_all = "camelCase")]
    Wait {
        /// The wait id, from the same namespace as the step ids.
        #[schemars(length(min = 1, max = 100), regex(pattern = r"^[A-Za-z0-9._~:-]+$"))]
        id: String,
        /// The message ids of the requests it waits for: 1 through 100, unique.
        #[schemars(length(min = 1, max = 100))]
        requests: Vec<MessageId>,
        /// Every reply or the first one.
        mode: WaitMode,
        /// Its deadline; absent or null: none. At most `MAX_INSTANT`.
        #[serde(default)]
        until: Option<Timestamp>,
        /// How it ended; absent or null: it is open.
        #[serde(default)]
        outcome: Option<WaitOutcome>,
        /// The requests whose replies it selected, in the order of `requests`; absent: none. At most 100.
        #[serde(default)]
        #[schemars(length(max = 100))]
        received: Vec<MessageId>,
    },
}

impl JournalEntry {
    /// The id of the entry.
    pub fn id(&self) -> &str {
        match self {
            JournalEntry::Step { id, .. }
            | JournalEntry::Sleep { id, .. }
            | JournalEntry::Send { id, .. }
            | JournalEntry::Wait { id, .. } => id,
        }
    }

    /// The kind in messages: a send that awaits a reply is a request.
    fn kind(&self) -> &'static str {
        match self {
            JournalEntry::Step { .. } => "step",
            JournalEntry::Sleep { .. } => "sleep",
            JournalEntry::Send { reply: false, .. } => "send",
            JournalEntry::Send { reply: true, .. } => "request",
            JournalEntry::Wait { .. } => "wait",
        }
    }
}

/// The state of a run: its input, the journal of saved results, its status, the id of its armed `Continue`, the
/// replies it kept and the journal of its cleanup.
///
/// On the wire: `{"input":{"user":"u1"},"journal":[{"kind":"step","id":"account","result":"a1"}],
/// "status":"running","armed":"m1","replies":[],"cleanup":[]}`; `journal`, `status`, `armed`, `replies` and `cleanup`
/// default to empty, running, null, empty and empty, so a state of an earlier release decodes. A decoded state is
/// checked (ADR 0044): each journal holds at most 1000 entries with unique valid ids, sleeps and deadlines end at most
/// at `MAX_INSTANT`, send message ids are unique across both journals, waits and replies name earlier requests
/// consistently, at most 2000 replies, and the status agrees with `armed`, `cleanup` and any open wait. The schema
/// describes field types, presence, lengths, patterns and names; the checks across fields are typed checks, not part of
/// it. A journal given at the create is trusted as progress: replay checks its order and result types against the
/// flow's code, not whether its effects happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    rename_all = "camelCase",
    try_from = "RawState<I>",
    bound(serialize = "I: Serialize", deserialize = "I: DeserializeOwned")
)]
pub struct FlowState<I> {
    /// The input of the run.
    pub input: I,
    /// The saved entries, in the order the flow reached them.
    pub journal: Vec<JournalEntry>,
    /// The phase of the run.
    pub status: FlowStatus,
    /// The `Continue` that executes the next step; none before the start, after recovery is needed, and once done.
    pub armed: Option<MessageId>,
    /// The replies kept for requests of either journal.
    pub replies: Vec<FlowReply>,
    /// The entries of the cleanup of a cancelled run; empty while running and once done.
    pub cleanup: Vec<JournalEntry>,
}

/// The wire form of a [`FlowState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", bound(deserialize = "I: DeserializeOwned"))]
struct RawState<I> {
    /// The input of the run.
    input: I,
    /// The saved entries, in the order the flow reached them; at most 1000, with unique ids; absent: none.
    #[serde(default)]
    #[schemars(length(max = 1000))]
    journal: Vec<JournalEntry>,
    /// Running, cancelling, done or cancelled; absent: running.
    #[serde(default)]
    status: FlowStatus,
    /// The id of the armed `Continue`; absent or null when none is armed, and always on a done or cancelled run.
    #[serde(default)]
    armed: Option<MessageId>,
    /// The replies kept for requests, one per request; at most 2000; absent: none.
    #[serde(default)]
    #[schemars(length(max = 2000))]
    replies: Vec<FlowReply>,
    /// The cleanup's entries, with ids of their own; at most 1000; absent: none.
    #[serde(default)]
    #[schemars(length(max = 1000))]
    cleanup: Vec<JournalEntry>,
}

impl<I> TryFrom<RawState<I>> for FlowState<I> {
    type Error = String;

    fn try_from(raw: RawState<I>) -> Result<Self, String> {
        let state = Self {
            input: raw.input,
            journal: raw.journal,
            status: raw.status,
            armed: raw.armed,
            replies: raw.replies,
            cleanup: raw.cleanup,
        };
        state.check()?;
        Ok(state)
    }
}

impl<I> FlowState<I> {
    /// A run of `input` that has saved nothing, is running and has no armed continuation: create it, then send it a
    /// `Continue`.
    ///
    /// ```
    /// use cicada_actors::FlowState;
    ///
    /// let state = FlowState::new(serde_json::json!({"user": "u1"}));
    /// assert_eq!(serde_json::to_value(&state)?, serde_json::json!({
    ///     "input": {"user": "u1"}, "journal": [], "status": "running", "armed": null, "replies": [], "cleanup": []}));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(input: I) -> Self {
        Self {
            input,
            journal: Vec::new(),
            status: FlowStatus::Running,
            armed: None,
            replies: Vec::new(),
            cleanup: Vec::new(),
        }
    }

    /// The saved result of the step `id` of the main journal as a `T`: `None` when no step of that id is saved (a
    /// sleep or a send is no step), and `Some(Err(..))` when it does not decode as a `T`.
    ///
    /// ```
    /// use cicada_actors::FlowState;
    ///
    /// let state: FlowState<String> = serde_json::from_value(serde_json::json!({"input": "u1",
    ///     "journal": [{"kind": "step", "id": "account", "result": "a1"}]}))?;
    /// assert_eq!(state.result::<String>("account").unwrap()?, "a1");
    /// assert!(state.result::<u32>("account").unwrap().is_err());
    /// assert!(state.result::<String>("welcome").is_none());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn result<T: DeserializeOwned>(&self, id: &str) -> Option<Result<T, serde_json::Error>> {
        self.journal.iter().find_map(|entry| match entry {
            JournalEntry::Step { id: saved, result } if saved == id => Some(T::deserialize(result)),
            _ => None,
        })
    }

    /// The typed checks of ADR 0044, beyond what the schema says.
    fn check(&self) -> Result<(), String> {
        if self.replies.len() > MAX_REPLIES {
            return Err(format!("a run keeps at most {MAX_REPLIES} replies, had {}", self.replies.len()));
        }
        let mut messages = HashSet::new();
        let mut sends = HashMap::new();
        for (phase, journal) in [(Phase::Main, &self.journal), (Phase::Cleanup, &self.cleanup)] {
            check_entries(phase, journal, &mut messages, &mut sends)?;
        }
        let mut replied = HashSet::new();
        for reply in &self.replies {
            if !replied.insert(&reply.request) {
                return Err(format!("the run keeps two replies to {}", reply.request));
            }
        }
        let main = check_waits(Phase::Main, &self.journal, &sends, &replied, None)?;
        let cleanup = check_waits(Phase::Cleanup, &self.cleanup, &sends, &replied, Some(&main))?;
        if let Some(reply) = self.replies.iter().find(|r| !sends.get(&r.request).is_some_and(|at| at.reply)) {
            return Err(format!("the reply to {} names no request of the run", reply.request));
        }
        let open = |waits: &Waits| waits.open.is_some();
        match self.status {
            FlowStatus::Running if !self.cleanup.is_empty() => Err("a running run has no cleanup entries".to_owned()),
            FlowStatus::Done if self.armed.is_some() => Err("a done run has no armed continuation".to_owned()),
            FlowStatus::Done if !self.cleanup.is_empty() => Err("a done run has no cleanup entries".to_owned()),
            FlowStatus::Done if open(&main) => Err("a done run has no open wait".to_owned()),
            FlowStatus::Cancelled if self.armed.is_some() => {
                Err("a cancelled run has no armed continuation".to_owned())
            }
            FlowStatus::Cancelled if open(&cleanup) => Err("a cancelled run has no open cleanup wait".to_owned()),
            _ => Ok(()),
        }
    }
}

/// The journal of a run's main code, or of its cleanup: two namespaces of ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Main,
    Cleanup,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Phase::Main => "the journal",
            Phase::Cleanup => "the cleanup journal",
        })
    }
}

/// Where a send entry is: its phase, its place in that journal and whether it is a request.
#[derive(Debug, Clone, Copy)]
struct SendAt {
    phase: Phase,
    index: usize,
    reply: bool,
}

/// The waits of one journal: the requests each wait names, with whether that wait is closed, and the open wait.
#[derive(Debug, Default)]
struct Waits<'s> {
    requests: HashMap<&'s MessageId, bool>,
    open: Option<&'s str>,
}

/// Checks the count, ids, times and send message ids of one journal, and notes where each send is.
fn check_entries<'s>(
    phase: Phase,
    journal: &'s [JournalEntry],
    messages: &mut HashSet<&'s MessageId>,
    sends: &mut HashMap<&'s MessageId, SendAt>,
) -> Result<(), String> {
    if journal.len() > MAX_ENTRIES {
        return Err(format!("{phase} holds at most {MAX_ENTRIES} entries, had {}", journal.len()));
    }
    let late = |what: &str, id: &str| format!("{what} `{id}` must not end after {}", time::format_instant(MAX_INSTANT));
    let mut ids = HashSet::new();
    for (index, entry) in journal.iter().enumerate() {
        check_id(entry.id())?;
        if !ids.insert(entry.id()) {
            return Err(format!("{phase} holds the id `{}` twice", entry.id()));
        }
        match entry {
            JournalEntry::Sleep { id, until } if *until > MAX_INSTANT => return Err(late("sleep", id)),
            JournalEntry::Wait { id, until: Some(until), .. } if *until > MAX_INSTANT => return Err(late("wait", id)),
            JournalEntry::Send { id, message, reply, .. } => {
                if !messages.insert(message) {
                    return Err(format!("send `{id}` repeats the message id {message} of another send"));
                }
                sends.insert(message, SendAt { phase, index, reply: *reply });
            }
            _ => {}
        }
    }
    Ok(())
}

/// Checks the waits of one journal (ADR 0044): requests, their reuse, the open wait and each saved selection. A
/// cleanup wait may also name a main request that no closed main wait names; one that an open main wait names is the
/// interrupted wait's request, which only a cancelling or cancelled run has, as only those have cleanup entries.
fn check_waits<'s>(
    phase: Phase,
    journal: &'s [JournalEntry],
    sends: &HashMap<&'s MessageId, SendAt>,
    replied: &HashSet<&'s MessageId>,
    main: Option<&Waits<'s>>,
) -> Result<Waits<'s>, String> {
    let mut waits = Waits::default();
    for (index, entry) in journal.iter().enumerate() {
        if let Some(open) = waits.open {
            return Err(format!(
                "{} `{}` of {phase} follows the open wait `{open}`, which must be the last entry",
                entry.kind(),
                entry.id()
            ));
        }
        let JournalEntry::Wait { id, requests, mode, until, outcome, received } = entry else {
            continue;
        };
        let refuse = |problem: String| Err(format!("wait `{id}` of {phase} {problem}"));
        if requests.is_empty() || requests.len() > MAX_WAIT_REQUESTS {
            return refuse(format!("names 1 through {MAX_WAIT_REQUESTS} requests, had {}", requests.len()));
        }
        if received.len() > MAX_WAIT_REQUESTS {
            return refuse(format!("selects at most {MAX_WAIT_REQUESTS} replies, had {}", received.len()));
        }
        let mut listed = HashSet::new();
        for request in requests {
            if !listed.insert(request) {
                return refuse(format!("names the request {request} twice"));
            }
            let earlier = match sends.get(request) {
                Some(at) if !at.reply => false,
                Some(at) if at.phase == phase => at.index < index,
                Some(_) => main.is_some_and(|main| main.requests.get(request) != Some(&true)),
                None => false,
            };
            if !earlier {
                let what = match (main, sends.get(request)) {
                    (Some(_), Some(at)) if at.reply && at.phase == Phase::Main => "a request a closed main wait names",
                    _ => "no earlier request of its journal",
                };
                return refuse(format!("names {request}, {what}"));
            }
            if waits.requests.insert(request, outcome.is_some()).is_some() {
                return refuse(format!("names the request {request} of another wait"));
            }
        }
        if outcome.is_none() {
            if !received.is_empty() {
                return refuse("is open but selects replies".to_owned());
            }
            waits.open = Some(id);
            continue;
        }
        let mut positions = Vec::with_capacity(received.len());
        for request in received {
            match requests.iter().position(|r| r == request) {
                Some(at) if positions.last().is_none_or(|last| *last < at) => positions.push(at),
                Some(_) => return refuse("selects replies out of the order of its requests, or twice".to_owned()),
                None => return refuse(format!("selects {request}, which it does not name")),
            }
            if !replied.contains(request) {
                return refuse(format!("selects {request}, whose reply the run does not keep"));
            }
        }
        let fits = match (outcome, mode) {
            (Some(WaitOutcome::Received), WaitMode::All) => received.len() == requests.len(),
            (Some(WaitOutcome::Received), WaitMode::First) => received.len() == 1,
            (Some(WaitOutcome::TimedOut), _) if until.is_none() => return refuse("timed out with no deadline".into()),
            (Some(WaitOutcome::TimedOut), WaitMode::All) => received.len() < requests.len(),
            (Some(WaitOutcome::TimedOut), WaitMode::First) => received.is_empty(),
            (None, _) => unreachable!("an open wait went on above"),
        };
        if !fits {
            return refuse(format!(
                "selects {} of {} replies, which its outcome and mode do not allow",
                received.len(),
                requests.len()
            ));
        }
    }
    Ok(waits)
}

/// Checks a step id: 1 through 100 characters of `A-Za-z0-9._~-:`.
fn check_id(id: &str) -> Result<(), String> {
    let valid = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-' | ':');
    if id.is_empty() || id.len() > MAX_ID_LENGTH || !id.chars().all(valid) {
        return Err(format!("a step id is 1 through {MAX_ID_LENGTH} characters of A-Za-z0-9._~-:, was {id:?}"));
    }
    Ok(())
}

/// Where a [`Step::send`] goes: a borrowed ref, and for a send that creates its target first, the borrowed initial
/// state and whether the target is independent or a child of the run. It converts and serializes nothing: a send does
/// that only for a new entry, never on replay.
///
/// A plain `&ref` converts into a target that exists already; [`Target::create`] and [`Target::child`] create it in the
/// same commit, before the message, by the rules of [`Turn::create_raw`] and [`Turn::create_child_raw`].
pub struct Target<'a, R, S: ?Sized = ()> {
    actor: &'a R,
    create: Option<(&'a S, CreateKind)>,
}

impl<'a, R> Target<'a, R> {
    /// The actor `actor`, which must exist when the send commits; the same as `&actor`.
    pub fn existing(actor: &'a R) -> Self {
        Self { actor, create: None }
    }
}

impl<'a, R, S: ?Sized> Target<'a, R, S> {
    /// The actor `actor`, created as an independent actor with `state` in the same commit; an actor that exists already
    /// keeps its state. The state is still checked for `null`, a NUL character and its depth.
    pub fn create(actor: &'a R, state: &'a S) -> Self {
        Self { actor, create: Some((state, CreateKind::Independent)) }
    }

    /// The actor `actor`, created as a child of the run with `state` in the same commit: deleting the run removes it. A
    /// child of the run that exists already keeps its state; an actor that exists with another parent, or with none,
    /// rejects the commit.
    pub fn child(actor: &'a R, state: &'a S) -> Self {
        Self { actor, create: Some((state, CreateKind::Child)) }
    }
}

impl<'a, R> From<&'a R> for Target<'a, R> {
    fn from(actor: &'a R) -> Self {
        Self::existing(actor)
    }
}

impl<R, S: ?Sized> fmt::Debug for Target<'_, R, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target").field("create", &self.create.map(|(_, kind)| kind)).finish_non_exhaustive()
    }
}

/// A request of [`Step::request`]: the id of its message, typed by the reply `T` that [`Step::wait`] or a join decodes.
/// It has no public constructor and is not serialized: a flow gets it from `Step::request` on every execution. A clone
/// is cheap, so named handles go into the array or `Vec` a join borrows.
pub struct Request<T> {
    id: MessageId,
    reply: PhantomData<fn() -> T>,
}

impl<T> Request<T> {
    fn new(id: MessageId) -> Self {
        Self { id, reply: PhantomData }
    }

    /// The id of the request's message, saved with its send entry and named by its reply.
    pub fn id(&self) -> &MessageId {
        &self.id
    }
}

impl<T> Clone for Request<T> {
    fn clone(&self) -> Self {
        Self::new(self.id.clone())
    }
}

impl<T> fmt::Debug for Request<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Request").field(&self.id.as_str()).finish()
    }
}

/// How a wait with a deadline ended, as committed in its journal entry, or one slot of [`Step::wait_all_timeout`]: data
/// to branch on, also when `T` is an `Option`. A timeout is no error, so the flow's retry policy never sees it.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waited<T> {
    /// The target's reply, decoded.
    Received(T),
    /// The deadline passed before the reply was handled; the target is not stopped and may still be working.
    TimedOut,
}

/// Where a target sends the reply `T` to a request of a flow: the run's address and the request's message id. A field
/// `reply: ReplyTo<JobResult>` of the target's message carries it and types the flow's [`Request`].
///
/// On the wire `{"run": "build/b1", "request": "m1"}`, whatever `T` is: both fields are required, unknown fields are
/// ignored and the schema is the same for every `T`. It is a route, not an identity: the run accepts a reply only
/// from the actor its request went to.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub struct ReplyTo<T> {
    /// The run that waits for the reply.
    pub run: ActorAddress,
    /// The id of the request's message.
    pub request: MessageId,
    #[serde(skip)]
    reply: PhantomData<fn() -> T>,
}

impl<T> ReplyTo<T> {
    fn new(run: ActorAddress, request: MessageId) -> Self {
        Self { run, request, reply: PhantomData }
    }
}

impl<T: Serialize> ReplyTo<T> {
    /// Sends `value` to the run as `FlowMsg::Reply` in the target's turn: it commits with the turn's other effects or
    /// not at all.
    ///
    /// The value is encoded and checked now: one that does not encode, holds a NUL or is more than 61 arrays and objects
    /// deep returns [`TurnError::Invalid`] and buffers nothing. The future borrows only `turn`. The reply is due now,
    /// carries the turn's correlation id, the observers' headers and the default retention, and has no retry policy;
    /// a raw send's errors come back unchanged. A run that does not exist at commit rejects the whole commit of the
    /// target's turn, as for any send. The run keeps only the first reply that comes from its request's target.
    ///
    /// ```
    /// use cicada::prelude::*;
    /// use cicada_actors::ReplyTo;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Job(String);
    ///
    /// impl ActorRef for Job {
    ///     const TYPE: &'static str = "jobs";
    ///     type Message = JobMsg;
    /// }
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// #[serde(tag = "type")]
    /// enum JobMsg {
    ///     Start { size: u32, reply: ReplyTo<u32> },
    /// }
    ///
    /// struct JobActor;
    ///
    /// impl Actor for JobActor {
    ///     type Ref = Job;
    ///     type State = NoState;
    ///
    ///     async fn handle(&self, turn: &mut Turn<Self>, message: JobMsg) -> HandlerResult {
    ///         let JobMsg::Start { size, reply } = message;
    ///         reply.send(turn, &(size * 2)).await?;
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub fn send<'t, A: Actor>(
        &self,
        turn: &'t mut Turn<A>,
        value: &T,
    ) -> impl Future<Output = Result<MessageId, TurnError>> + Send + use<'t, A, T> {
        let encoded = self.encode(value);
        async move {
            let (run, encoded) = encoded?;
            let mut options = SendOptions::new();
            options.correlation_id = turn.message().correlation_id.clone();
            turn.send_raw(&run, &encoded.message_type, encoded.payload, options).await
        }
    }

    /// The run and the encoded `Reply` of `value`, checked.
    fn encode(&self, value: &T) -> Result<(ActorAddress, cicada::message::EncodedMessage), TurnError> {
        let request = &self.request;
        let value = serde_json::to_value(value)
            .map_err(|e| TurnError::Invalid(format!("the reply to request {request} does not encode: {e}")))?;
        check_result(&value, 0).map_err(|problem| {
            TurnError::Invalid(format!("the reply to request {request} cannot be sent: {problem}"))
        })?;
        let encoded = cicada::message::encode(&FlowMsg::Reply { request: request.clone(), value })
            .map_err(|e| TurnError::Invalid(format!("the reply to request {request} does not encode: {e}")))?;
        Ok((self.run.clone(), encoded))
    }
}

impl<T> Clone for ReplyTo<T> {
    fn clone(&self) -> Self {
        Self::new(self.run.clone(), self.request.clone())
    }
}

impl<T> fmt::Debug for ReplyTo<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplyTo").field("run", &self.run.as_str()).field("request", &self.request.as_str()).finish()
    }
}

impl<T> PartialEq for ReplyTo<T> {
    fn eq(&self, other: &Self) -> bool {
        self.run == other.run && self.request == other.request
    }
}

impl<T> Eq for ReplyTo<T> {}

/// Where a target sends the reply to a request of a flow: the run that waits and the id of the request's message. The
/// schema of every [`ReplyTo`], whatever its reply type.
#[derive(JsonSchema)]
#[schemars(rename = "ReplyTo")]
#[allow(dead_code)]
struct ReplyRoute {
    /// The run that waits for the reply.
    run: ActorAddress,
    /// The id of the request's message.
    request: MessageId,
}

impl<T> JsonSchema for ReplyTo<T> {
    fn schema_name() -> Cow<'static, str> {
        "ReplyTo".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::ReplyTo").into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        ReplyRoute::json_schema(generator)
    }
}

/// The actor effects of a new send entry, applied by the runner in the commit that saves the entry.
struct Outbox {
    message_type: String,
    payload: Value,
    /// The initial state of a create; the entry says which kind.
    state: Option<Value>,
}

/// What the first new entry, the first sleep not yet due, the first open wait, or the first misuse, of an execution
/// decided. It wins over what the flow returns.
enum Latch {
    /// A step's closure was invoked and has not finished, a send's or a request's arguments are being converted, or a
    /// wait's saved reply is being decoded: of the entry with this id; if this is left, the closure's future was
    /// dropped, or the closure, a conversion or a decoder panicked.
    InProgress(&'static str, String),
    /// A new entry to save with the next continuation, which a sleep's wake time delays and an open wait leaves out,
    /// and a send's or a request's actor effects.
    New(JournalEntry, Option<Box<Outbox>>),
    /// A saved sleep whose wake time has not come, or a saved open wait whose deadline has not come and whose reply has
    /// not: the next continuation is due then, and nothing is saved.
    Waiting(Timestamp),
    /// The saved open wait at this index of the journal, closed with this outcome and selection: received when the run
    /// keeps enough replies, timed out with the kept ones when its deadline has passed without them. Close it and arm
    /// now.
    Close(usize, WaitOutcome, Vec<MessageId>),
    /// A saved open wait without a deadline whose replies have not come: no continuation is armed, and nothing is saved.
    Parked,
    /// The closure's own error, with its retryability.
    Failed(HandlerError),
    /// A replay or result that cannot be saved; refused for good.
    Permanent(String),
}

/// The private error by which the helpers of [`Step`] suspend a flow: passed on with `?`, it ends the execution.
#[derive(Debug)]
struct Suspend;

impl fmt::Display for Suspend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "the flow is suspended after a new step, send, request or wait, at a sleep or an open wait, or after a \
             wait's decision; pass this error on with `?`",
        )
    }
}

impl std::error::Error for Suspend {}

/// The steps of one execution of a flow: it replays saved results, elapsed sleeps, saved sends and requests and closed
/// waits in journal order and executes at most one new step, sleep, send, request or wait. A flow sees it only through
/// [`Flow::run`]; it gives no access to the turn: a send's effects are buffered by the runner.
pub struct Step {
    journal: Vec<JournalEntry>,
    cursor: usize,
    seen: HashSet<String>,
    /// The replies the run keeps, by request, in the order they were accepted.
    replies: Vec<(MessageId, Value)>,
    address: ActorAddress,
    /// Store time at the claim: what a sleep counts from and is compared with.
    now: Timestamp,
    latch: Option<Latch>,
}

impl fmt::Debug for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Step")
            .field("run", &self.address.as_str())
            .field("cursor", &self.cursor)
            .finish_non_exhaustive()
    }
}

impl Step {
    fn new(journal: Vec<JournalEntry>, address: &ActorAddress, now: Timestamp) -> Self {
        Self {
            journal,
            cursor: 0,
            seen: HashSet::new(),
            replies: Vec::new(),
            address: address.clone(),
            now,
            latch: None,
        }
    }

    /// The step `id`. When the journal holds its result, returns it decoded as a `T` without calling `call`.
    /// Otherwise calls `call` with the idempotency key `<run address>/<id>`, saves its result and the next
    /// continuation in this turn's commit, and returns an error that suspends the flow: pass it on with `?`.
    ///
    /// The first new step of an execution, or its first misuse, decides the turn: a later step invokes nothing, and
    /// catching the error changes nothing. A closure's error fails the turn with its own retryability; a result that
    /// does not encode, holds a NUL or is more than 61 levels deep, a step id that is invalid or repeated, a step that
    /// does not match the saved entry at its place, a saved result that does not decode as a `T`, and a new step after
    /// 1000 entries fail it for good. A future of this call dropped while its closure runs (a timeout around it, a
    /// `select!`), or a panic caught around it, fails the turn for good: put a timeout inside the closure. A business
    /// outcome to branch on is a successful `T`, such as a `Result<T, String>`.
    ///
    /// ```
    /// use cicada::prelude::*;
    /// use cicada_actors::{Flow, Step};
    ///
    /// struct Pay;
    ///
    /// impl Flow for Pay {
    ///     const TYPE: &'static str = "pay";
    ///     type Input = u64;
    ///
    ///     async fn run(&self, step: &mut Step, cents: u64) -> HandlerResult {
    ///         // A declined card is data to branch on; an outage is an error, retried by the flow's policy.
    ///         let charged: Result<String, String> = step.run("charge", |key| async move {
    ///             if cents == 0 { Err(HandlerError::msg("the payment service is down")) } else { Ok(Ok(key)) }
    ///         }).await?;
    ///         if charged.is_err() {
    ///             step.run("notify-declined", |_key| async { Ok(()) }).await?;
    ///         }
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub async fn run<T, C, Fut>(&mut self, id: &str, call: C) -> Result<T, HandlerError>
    where
        T: Serialize + DeserializeOwned + Send,
        C: FnOnce(String) -> Fut + Send,
        Fut: Future<Output = Result<T, HandlerError>> + Send,
    {
        if self.latch.is_some() {
            return Err(HandlerError::new(Suspend));
        }
        match self.replay(id, "step") {
            Err(reason) => return Err(self.refuse(reason)),
            Ok(Some(JournalEntry::Step { result, .. })) => {
                return T::deserialize(&result).map_err(|e| {
                    self.refuse(format!("the saved result of step `{id}` does not decode as the type asked for: {e}"))
                });
            }
            Ok(Some(_)) => unreachable!("replay matches the kind"),
            Ok(None) => {}
        }
        if let Err(reason) = self.room(id, "step") {
            return Err(self.refuse(reason));
        }
        self.latch = Some(Latch::InProgress("step", id.to_owned()));
        let outcome = call(format!("{}/{id}", self.address)).await;
        let latch = match outcome {
            Err(e) => Latch::Failed(e),
            Ok(value) => match encode(id, &value) {
                Ok(result) => Latch::New(JournalEntry::Step { id: id.to_owned(), result }, None),
                Err(reason) => Latch::Permanent(reason),
            },
        };
        self.latch = Some(latch);
        Err(HandlerError::new(Suspend))
    }

    /// The durable sleep `id`: the flow continues once store time reaches a wake time saved in the journal, and no
    /// handler stays open meanwhile. Pass its error on with `?`.
    ///
    /// The first time, it saves the wake time, the claim's store time plus `duration` rounded up to microseconds, and
    /// suspends the flow: the commit arms the next `Continue` due then. A zero duration saves and suspends as well, so
    /// every new sleep is a commit. On replay it returns `Ok(())` once store time has reached the saved wake time;
    /// before that it suspends again and arms a `Continue` due at the same time, saving nothing, so an early `Continue`,
    /// retry, redrive or deliver-now cannot shorten the sleep. Replay ignores `duration`: the saved time decides, also
    /// when the code changed it. The wake time is a lower bound: a paused run, an outage or a cancelled continuation
    /// makes it later, and a late wake does not sleep again; a run whose continuation was cancelled, purged or failed
    /// waits for a public `Continue`.
    ///
    /// Sleep ids share the namespace and the limit of 1000 entries of the step ids. A new sleep longer than
    /// [`MAX_DELAY`], or that would end after [`MAX_INSTANT`], an invalid or repeated id, a saved entry at its place that
    /// is not this sleep, and a new sleep after 1000 entries fail the turn for good. Like [`Step::run`], after the first
    /// new step or sleep of an execution, or its first misuse, it does nothing.
    ///
    /// ```
    /// use std::time::Duration;
    /// use cicada::prelude::*;
    /// use cicada_actors::{Flow, Step};
    ///
    /// struct Reminder;
    ///
    /// impl Flow for Reminder {
    ///     const TYPE: &'static str = "reminder";
    ///     type Input = String;
    ///
    ///     async fn run(&self, step: &mut Step, user: String) -> HandlerResult {
    ///         step.run("signed-up", |_key| async { Ok(()) }).await?;
    ///         step.sleep("cool-off", Duration::from_secs(24 * 3600)).await?;
    ///         step.run("remind", |key| async move { Ok(format!("{user}: {key}")) }).await?;
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub async fn sleep(&mut self, id: &str, duration: Duration) -> HandlerResult {
        if self.latch.is_some() {
            return Err(HandlerError::new(Suspend));
        }
        let latch = match self.replay(id, "sleep") {
            Err(reason) => Latch::Permanent(reason),
            Ok(Some(JournalEntry::Sleep { until, .. })) if self.now >= until => return Ok(()),
            Ok(Some(JournalEntry::Sleep { until, .. })) => Latch::Waiting(until),
            Ok(Some(_)) => unreachable!("replay matches the kind"),
            Ok(None) => match self.room(id, "sleep").and_then(|()| wake_time("sleep", id, self.now, duration)) {
                Ok(until) => Latch::New(JournalEntry::Sleep { id: id.to_owned(), until }, None),
                Err(reason) => Latch::Permanent(reason),
            },
        };
        self.latch = Some(latch);
        Err(HandlerError::new(Suspend))
    }

    /// The send `id`: `message` for `to`, with an optional create of the target first. When the journal holds this
    /// send, returns `Ok(())` and does nothing else. Otherwise it prepares the send, and the runner commits the create,
    /// the message, the send entry and the next continuation together; it returns an error that suspends the flow: pass
    /// it on with `?`.
    ///
    /// `to` is a `&ref` of an actor that exists, or a [`Target::create`] or [`Target::child`]. The message is due now,
    /// carries the correlation id of the executing `Continue` and has no retry policy: a failure of the target's
    /// handler ends it, whatever the flow's policy, and saved progress stays. A send enqueues its message once per
    /// committed entry; the target executes it under the ordinary at-least-once rules, and the flow learns nothing of
    /// its outcome.
    ///
    /// Replay compares the id and the kind only. It never converts `to` or `message`, computes an address or serializes
    /// the message or the state: the saved entry says where the message went, also when the code now names another
    /// target. The code that builds the arguments still runs on every replay, so it must be deterministic and free of
    /// effects, and dropping them runs their ordinary `Drop`.
    ///
    /// A target that is the run itself, or has no address, a message or a state that does not encode, an invalid or
    /// repeated id, a saved entry at its place that is not this send (a request included), and a new send after 1000
    /// entries fail the turn for good. When the send commits, the core checks the rest: a type that is not registered
    /// or does not accept the message, or an initial state that is `null`, holds a NUL or is too deep, fails the turn
    /// for good, also when the target exists already; a failed read of the type registry is an ordinary failure of
    /// cause `node`, retried by the flow's policy; a target that does not exist, or a child create of an actor owned
    /// elsewhere, rejects the commit, and the run waits for a public `Continue`. A panic while the arguments are
    /// converted or encoded, if caught, fails the turn for good. Like [`Step::run`], after the first new step, sleep or
    /// send of an execution, or its first misuse, it does nothing.
    ///
    /// ```
    /// use cicada::prelude::*;
    /// use cicada_actors::{Flow, Step, Target};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Job(String);
    ///
    /// impl ActorRef for Job {
    ///     const TYPE: &'static str = "jobs";
    ///     type Message = JobMsg;
    /// }
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// #[serde(tag = "type")]
    /// enum JobMsg {
    ///     Start { size: u32 },
    ///     Stop,
    /// }
    ///
    /// #[derive(Serialize)]
    /// struct JobState {
    ///     size: u32,
    /// }
    ///
    /// struct Build;
    ///
    /// impl Flow for Build {
    ///     const TYPE: &'static str = "build";
    ///     type Input = String;
    ///
    ///     async fn run(&self, step: &mut Step, name: String) -> HandlerResult {
    ///         let job = Job(name);
    ///         // Creates the job as a child of the run and starts it, in the commit that saves the entry.
    ///         step.send("start", Target::child(&job, &JobState { size: 3 }), JobMsg::Start { size: 3 }).await?;
    ///         step.run("wait-for-it", |_key| async { Ok(()) }).await?;
    ///         step.send("stop", &job, JobMsg::Stop).await?;
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub async fn send<'a, R, S>(
        &mut self,
        id: &str,
        to: impl Into<Target<'a, R, S>> + Send,
        message: impl Into<R::Message> + Send,
    ) -> HandlerResult
    where
        R: ActorRef + Sync + 'a,
        S: Serialize + Sync + ?Sized + 'a,
    {
        if self.latch.is_some() {
            return Err(HandlerError::new(Suspend));
        }
        match self.replay(id, "send") {
            Err(reason) => return Err(self.refuse(reason)),
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
        }
        if let Err(reason) = self.room(id, "send") {
            return Err(self.refuse(reason));
        }
        self.latch = Some(Latch::InProgress("send", id.to_owned()));
        let latch = match self.prepare(id, "send", to, |_| message.into()) {
            Ok((entry, outbox)) => Latch::New(entry, Some(Box::new(outbox))),
            Err(reason) => Latch::Permanent(reason),
        };
        self.latch = Some(latch);
        Err(HandlerError::new(Suspend))
    }

    /// The request `id`: a send whose message `build` makes from a [`ReplyTo`] route, so that its target can reply
    /// with a `T`, which [`Step::wait`] returns. When the journal holds this request, returns its [`Request`] and does
    /// nothing else. Otherwise it prepares the send like [`Step::send`], with a fresh message id that the route names,
    /// and the runner commits the create, the message, the request entry and the next continuation together; it returns
    /// an error that suspends the flow: pass it on with `?`.
    ///
    /// The route is `{run, request}`: the run's address and the id of the request's message. The run keeps the first
    /// reply that comes from the request's target, also before the flow waits for it, and ignores every other `Reply`.
    /// Replay compares the id and the kind only: a request is not a send. It never converts `to`, calls `build` or
    /// serializes the message or the state. Everything else is as for [`Step::send`]: the message has no retry policy,
    /// carries the executing `Continue`'s correlation id, and its target, message and state are checked the same way; a
    /// caught panic in `build` fails the turn for good like one in a conversion.
    ///
    /// ```
    /// use cicada::prelude::*;
    /// use cicada_actors::{Flow, ReplyTo, Step, Target};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Job(String);
    ///
    /// impl ActorRef for Job {
    ///     const TYPE: &'static str = "jobs";
    ///     type Message = JobMsg;
    /// }
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// #[serde(tag = "type")]
    /// enum JobMsg {
    ///     Start { size: u32, reply: ReplyTo<JobResult> },
    /// }
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// struct JobResult {
    ///     artifact: String,
    /// }
    ///
    /// struct Build;
    ///
    /// impl Flow for Build {
    ///     const TYPE: &'static str = "build";
    ///     type Input = String;
    ///
    ///     async fn run(&self, step: &mut Step, name: String) -> HandlerResult {
    ///         let job = Job(name);
    ///         let start = Target::child(&job, &NoState {});
    ///         let request = step.request("start", start, |reply| JobMsg::Start { size: 3, reply }).await?;
    ///         let done = step.wait("done", &request).await?;
    ///         step.run("publish", |_key| async move { Ok(done.artifact) }).await?;
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub async fn request<'a, R, S, T, B>(
        &mut self,
        id: &str,
        to: impl Into<Target<'a, R, S>> + Send,
        build: B,
    ) -> Result<Request<T>, HandlerError>
    where
        R: ActorRef + Sync + 'a,
        S: Serialize + Sync + ?Sized + 'a,
        B: FnOnce(ReplyTo<T>) -> R::Message + Send,
    {
        if self.latch.is_some() {
            return Err(HandlerError::new(Suspend));
        }
        match self.replay(id, "request") {
            Err(reason) => return Err(self.refuse(reason)),
            Ok(Some(JournalEntry::Send { message, .. })) => return Ok(Request::new(message)),
            Ok(Some(_)) => unreachable!("replay matches the kind"),
            Ok(None) => {}
        }
        if let Err(reason) = self.room(id, "request") {
            return Err(self.refuse(reason));
        }
        self.latch = Some(Latch::InProgress("request", id.to_owned()));
        let run = self.address.clone();
        let latch = match self.prepare(id, "request", to, |message| build(ReplyTo::new(run, message.clone()))) {
            Ok((entry, outbox)) => Latch::New(entry, Some(Box::new(outbox))),
            Err(reason) => Latch::Permanent(reason),
        };
        self.latch = Some(latch);
        Err(HandlerError::new(Suspend))
    }

    /// The wait `id` for the reply to `request`: returns the reply decoded as a `T` once the journal holds this wait
    /// closed with it. Otherwise it saves the wait and suspends the flow, and no handler stays open meanwhile: pass its
    /// error on with `?`.
    ///
    /// A new wait whose reply the run keeps already (it came early) saves the wait closed and arms the next `Continue`,
    /// which returns the value; one whose reply has not come saves the wait open and arms nothing, and the run is
    /// parked: the `Reply` turn that brings the reply closes the wait and arms the `Continue`, with the reply's
    /// correlation id. Either way the value reaches the code after the wait only in a later execution, once the
    /// outcome is committed. A parked run sends no polling message; a public `Continue` replays it and parks it again,
    /// saving nothing. The wait has no timeout: a target that fails or is deleted without replying leaves the run
    /// parked; [`Step::wait_timeout`] gives up at a deadline.
    ///
    /// The request must be one of this run's earlier requests that no other wait names. Wait ids share the namespace
    /// and the limit of 1000 entries of the step ids. Another request, an invalid or repeated id, a saved entry at its
    /// place that is not this wait (a wait for other requests or of mode first, or a wait with a deadline, which only
    /// the timed helpers replay, included), and a new wait after 1000 entries fail the turn for good. A saved reply that
    /// does not decode as a `T` fails it for good and keeps the outcome: fix the code and send a public `Continue`. A
    /// panic in a custom `Deserialize`, if caught, fails it for good too. Like [`Step::run`], after the first new entry
    /// of an execution, or its first misuse, it does nothing.
    ///
    /// A wait for one request is the [`Step::wait_all`] of that one request: the two save the same entry and replay
    /// each other's.
    pub async fn wait<T: DeserializeOwned + Send>(
        &mut self,
        id: &str,
        request: &Request<T>,
    ) -> Result<T, HandlerError> {
        let closed = self.decide(id, "Step::wait", &[request.id()], WaitMode::All, None)?;
        match self.decode::<T>(id, closed)? {
            (WaitOutcome::Received, mut values) if values.len() == 1 => Ok(values.remove(0).1),
            _ => Err(self.refuse(format!("wait `{id}` is saved timed out without a deadline"))),
        }
    }

    /// The wait `id` for the reply to `request`, which gives up at a deadline: returns [`Waited::Received`] with the
    /// reply decoded as a `T`, or [`Waited::TimedOut`], once the journal holds this wait closed with that outcome.
    /// Otherwise it saves the wait or its decision and suspends the flow: pass its error on with `?`. A timeout is
    /// data to branch on, not an error.
    ///
    /// The deadline is saved with the new wait: the store time of the claim of the turn that creates the wait, plus
    /// `timeout` rounded up to microseconds; not the time of the request's turn. Replay uses the saved deadline and
    /// ignores `timeout`, also when the code changed it. A new wait whose reply came early saves itself closed as
    /// received and arms the next `Continue` due now, also for a zero timeout; one whose reply has not come saves itself
    /// open and arms a `Continue` due at the deadline, and no handler stays open meanwhile. A new wait never times out
    /// in its own turn: a zero timeout commits it open first.
    ///
    /// The outcome is decided once and committed before any later code runs. A `Reply` turn that brings the reply
    /// closes the wait as received, cancels the deadline's `Continue` and arms one due now with the reply's correlation
    /// id, without reading the clock: a reply handled before the timeout is decided wins, also after the deadline. A
    /// `Continue` that executes the flow at or after the deadline with no reply kept closes the wait as timed out,
    /// keeping its own correlation id; one before the deadline arms a `Continue` due at the same deadline and saves
    /// nothing, so a poke, a retry or a deliver-now cannot shorten it. Which of a reply and the deadline's `Continue`
    /// wins is the order in which the run's mailbox handles them. Once committed the outcome never changes: a later
    /// reply is ignored, and a retry of a later step replays the same outcome without asking again. A timeout stops
    /// nothing of the target's work and cleans nothing up. The deadline is a lower bound: a pause, an outage or a
    /// delayed commit makes the timeout later, and a run whose deadline `Continue` was cancelled, purged or failed stays
    /// open, with no polling, until a reply or a public `Continue`.
    ///
    /// A `timeout` longer than [`MAX_DELAY`], or a deadline after [`MAX_INSTANT`], fails the turn for good, checked
    /// before an early reply is used. A saved wait without a deadline, which only the untimed helpers replay, and every
    /// misuse of `wait` fail it for good as they fail `wait`; so do the decoding failures of a received reply, which
    /// keep the outcome. A timed wait for one request is the [`Step::wait_all_timeout`] of that one request. Upgrade
    /// every host and worker of the flow type before a flow waits with a deadline: an older release refuses the reply
    /// to such a wait for good, although the contract is the same.
    ///
    /// ```
    /// use std::time::Duration;
    /// use cicada::prelude::*;
    /// use cicada_actors::{Flow, ReplyTo, Step, Target, Waited};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Job(String);
    ///
    /// impl ActorRef for Job {
    ///     const TYPE: &'static str = "jobs";
    ///     type Message = JobMsg;
    /// }
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// #[serde(tag = "type")]
    /// enum JobMsg {
    ///     Start { reply: ReplyTo<String> },
    /// }
    ///
    /// struct Build;
    ///
    /// impl Flow for Build {
    ///     const TYPE: &'static str = "build";
    ///     type Input = String;
    ///
    ///     async fn run(&self, step: &mut Step, name: String) -> HandlerResult {
    ///         let job = Job(name);
    ///         let start = Target::child(&job, &NoState {});
    ///         let request = step.request("start", start, |reply| JobMsg::Start { reply }).await?;
    ///         match step.wait_timeout("done", &request, Duration::from_secs(3600)).await? {
    ///             Waited::Received(artifact) => step.run("publish", |_key| async move { Ok(artifact) }).await?,
    ///             Waited::TimedOut => step.run("report-stuck", |key| async move { Ok(key) }).await?,
    ///         };
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub async fn wait_timeout<T: DeserializeOwned + Send>(
        &mut self,
        id: &str,
        request: &Request<T>,
        timeout: Duration,
    ) -> Result<Waited<T>, HandlerError> {
        let closed = self.decide(id, "Step::wait_timeout", &[request.id()], WaitMode::All, Some(timeout))?;
        Ok(match self.decode::<T>(id, closed)? {
            (WaitOutcome::Received, mut values) if values.len() == 1 => Waited::Received(values.remove(0).1),
            _ => Waited::TimedOut,
        })
    }

    /// The join `id` that waits for the replies to every one of `requests`: returns them decoded as `T`s, in the order
    /// of `requests`, once the journal holds this wait closed with all of them. Otherwise it saves the wait or its
    /// decision and suspends the flow: pass its error on with `?`. One wait entry holds the whole join, so 100
    /// requests and their join take 101 entries of the journal.
    ///
    /// `requests` is a borrowed slice of 1 through 100 distinct earlier requests of this run that no other wait names;
    /// a `Vec` or an array of handles works, cloned from named ones when need be, and an enum gives replies of several
    /// shapes one type `T`. The helper neither sorts nor deduplicates it: its positions are the positions of the result,
    /// and replay compares the saved list with it in order. Replies are kept as they come, before the wait too. A new
    /// join whose replies are all kept commits closed and arms the next `Continue`; otherwise it commits open and the
    /// run parks, sending nothing. Each `Reply` turn keeps its value; the one that brings the last missing reply closes
    /// the join and arms a `Continue` due now with its correlation id, without flow code; the earlier ones leave
    /// `armed` as it was. The values reach the code after the join only in a later execution, once the outcome is
    /// committed, and only those are decoded, in request order.
    ///
    /// An empty list, 101 handles, a handle given twice, an unknown request or one another wait names, a saved entry at
    /// its place for other requests, another order, mode first or a deadline, and every misuse of [`Step::wait`] fail the
    /// turn for good, as do the decoding failures of a saved reply, which keep the outcome. A join of one request is
    /// the [`Step::wait`] of that request. Upgrade every host and worker of the flow type before a flow joins more than
    /// one request: 0.18.2 refuses a reply to such a wait for good, although the contract is the same.
    ///
    /// ```
    /// use cicada::prelude::*;
    /// use cicada_actors::{Flow, ReplyTo, Step, Target};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Job(String);
    ///
    /// impl ActorRef for Job {
    ///     const TYPE: &'static str = "jobs";
    ///     type Message = JobMsg;
    /// }
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// #[serde(tag = "type")]
    /// enum JobMsg {
    ///     Build { target: String, reply: ReplyTo<String> },
    /// }
    ///
    /// struct Release;
    ///
    /// impl Flow for Release {
    ///     const TYPE: &'static str = "release";
    ///     type Input = Vec<String>;
    ///
    ///     async fn run(&self, step: &mut Step, targets: Vec<String>) -> HandlerResult {
    ///         let mut builds = Vec::new();
    ///         for target in &targets {
    ///             let job = Job(target.clone());
    ///             let build = |reply| JobMsg::Build { target: target.clone(), reply };
    ///             builds.push(step.request(&format!("build-{target}"), Target::child(&job, &NoState {}), build).await?);
    ///         }
    ///         // One artifact per target, in the order of `targets`, whichever build finished first.
    ///         let artifacts = step.wait_all("built", &builds).await?;
    ///         step.run("publish", |_key| async move { Ok(artifacts) }).await?;
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub async fn wait_all<T: DeserializeOwned + Send>(
        &mut self,
        id: &str,
        requests: &[Request<T>],
    ) -> Result<Vec<T>, HandlerError> {
        let asked: Vec<&MessageId> = requests.iter().map(Request::id).collect();
        let closed = self.decide(id, "Step::wait_all", &asked, WaitMode::All, None)?;
        match self.decode::<T>(id, closed)? {
            (WaitOutcome::Received, values) if values.len() == asked.len() => {
                Ok(values.into_iter().map(|(_, value)| value).collect())
            }
            _ => Err(self.refuse(format!("wait `{id}` is saved timed out without a deadline"))),
        }
    }

    /// The join `id` that waits for the replies to every one of `requests` until a deadline: returns one slot per
    /// request, in the order of `requests`, once the journal holds this wait closed: [`Waited::Received`] with the
    /// decoded reply, or [`Waited::TimedOut`] for a request whose reply had not come when the join timed out. Every
    /// slot is `Received` when all replies came in time; at least one `TimedOut` means the join timed out, and the
    /// other slots keep the replies that had come. Otherwise it saves the wait or its decision and suspends the flow:
    /// pass its error on with `?`.
    ///
    /// The deadline is saved and replayed as for [`Step::wait_timeout`], checked before any early reply is used, and a
    /// reply that does not complete the join changes neither the deadline nor its `Continue`. The last missing reply
    /// closes the join as received and cancels the deadline's `Continue`; an execution at or after the deadline that
    /// finds replies missing closes it as timed out with the replies kept then, before code after the join runs; a
    /// zero timeout commits the join open first. The rules of [`Step::wait_all`] apply to `requests`, and a saved
    /// entry without a deadline does not replay here. A timeout stops none of the targets.
    ///
    /// ```
    /// use std::time::Duration;
    /// use cicada::prelude::*;
    /// use cicada_actors::{Flow, ReplyTo, Step, Waited};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Probe(String);
    ///
    /// impl ActorRef for Probe {
    ///     const TYPE: &'static str = "probes";
    ///     type Message = ProbeMsg;
    /// }
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// #[serde(tag = "type")]
    /// enum ProbeMsg {
    ///     Check { reply: ReplyTo<bool> },
    /// }
    ///
    /// struct Health;
    ///
    /// impl Flow for Health {
    ///     const TYPE: &'static str = "health";
    ///     type Input = (String, String);
    ///
    ///     async fn run(&self, step: &mut Step, (east, west): (String, String)) -> HandlerResult {
    ///         let east = step.request("east", &Probe(east), |reply| ProbeMsg::Check { reply }).await?;
    ///         let west = step.request("west", &Probe(west), |reply| ProbeMsg::Check { reply }).await?;
    ///         let slots = step.wait_all_timeout("checked", &[east, west], Duration::from_secs(60)).await?;
    ///         // [Received(true), TimedOut]: east answered in time, west did not and may still be working.
    ///         let silent: Vec<usize> = (0..slots.len()).filter(|&i| slots[i] == Waited::TimedOut).collect();
    ///         step.run("report", |_key| async move { Ok(silent) }).await?;
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub async fn wait_all_timeout<T: DeserializeOwned + Send>(
        &mut self,
        id: &str,
        requests: &[Request<T>],
        timeout: Duration,
    ) -> Result<Vec<Waited<T>>, HandlerError> {
        let asked: Vec<&MessageId> = requests.iter().map(Request::id).collect();
        let closed = self.decide(id, "Step::wait_all_timeout", &asked, WaitMode::All, Some(timeout))?;
        let (_, values) = self.decode::<T>(id, closed)?;
        let mut slots: Vec<Waited<T>> = asked.iter().map(|_| Waited::TimedOut).collect();
        for (at, value) in values {
            slots[at] = Waited::Received(value);
        }
        Ok(slots)
    }

    /// The join `id` that waits for the first reply to any of `requests`: returns the position of the winning request
    /// in `requests` and its reply decoded as a `T`, once the journal holds this wait closed. Otherwise it saves the
    /// wait or its decision and suspends the flow: pass its error on with `?`.
    ///
    /// The winner is the listed request whose reply the run accepted first, in the order its replies are kept, also
    /// when several replies came before the wait: not the first in `requests`. A `Reply` turn that brings a reply to
    /// any listed request closes the join and arms a `Continue` due now with its correlation id, without flow code.
    /// Once committed the winner never changes. Closing the join closes every listed request: a later reply to a losing
    /// one is ignored, the losing targets are not stopped and keep working, and no later wait can name those requests;
    /// send new work to clean up after them. Only the winner's reply is decoded. A first wait of one request is still a
    /// first wait, which [`Step::wait`] does not replay. The rules of [`Step::wait_all`] apply to `requests`.
    ///
    /// ```
    /// use cicada::prelude::*;
    /// use cicada_actors::{Flow, ReplyTo, Step};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Supplier(String);
    ///
    /// impl ActorRef for Supplier {
    ///     const TYPE: &'static str = "suppliers";
    ///     type Message = SupplierMsg;
    /// }
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// #[serde(tag = "type")]
    /// enum SupplierMsg {
    ///     Quote { item: String, reply: ReplyTo<u64> },
    /// }
    ///
    /// struct Buy;
    ///
    /// impl Flow for Buy {
    ///     const TYPE: &'static str = "buy";
    ///     type Input = String;
    ///
    ///     async fn run(&self, step: &mut Step, item: String) -> HandlerResult {
    ///         let suppliers = [Supplier("acme".into()), Supplier("globex".into())];
    ///         let mut quotes = Vec::new();
    ///         for supplier in &suppliers {
    ///             let quote = |reply| SupplierMsg::Quote { item: item.clone(), reply };
    ///             quotes.push(step.request(&format!("quote-{}", supplier.0), supplier, quote).await?);
    ///         }
    ///         let (winner, cents) = step.wait_first("quoted", &quotes).await?;
    ///         let chosen = suppliers[winner].0.clone();
    ///         step.run("order", |_key| async move { Ok((chosen, cents)) }).await?;
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub async fn wait_first<T: DeserializeOwned + Send>(
        &mut self,
        id: &str,
        requests: &[Request<T>],
    ) -> Result<(usize, T), HandlerError> {
        let asked: Vec<&MessageId> = requests.iter().map(Request::id).collect();
        let closed = self.decide(id, "Step::wait_first", &asked, WaitMode::First, None)?;
        match self.decode::<T>(id, closed)? {
            (WaitOutcome::Received, mut values) if values.len() == 1 => Ok(values.remove(0)),
            _ => Err(self.refuse(format!("wait `{id}` is saved timed out without a deadline"))),
        }
    }

    /// The join `id` that waits for the first reply to any of `requests` until a deadline: returns
    /// [`Waited::Received`] with the position of the winning request and its decoded reply, as [`Step::wait_first`]
    /// chooses them, or [`Waited::TimedOut`], with no winner, when no listed reply was handled before an execution at or
    /// after the deadline. The deadline is saved and replayed as for [`Step::wait_timeout`], and the rules of
    /// [`Step::wait_first`] apply. A timeout closes every listed request and stops none of the targets.
    pub async fn wait_first_timeout<T: DeserializeOwned + Send>(
        &mut self,
        id: &str,
        requests: &[Request<T>],
        timeout: Duration,
    ) -> Result<Waited<(usize, T)>, HandlerError> {
        let asked: Vec<&MessageId> = requests.iter().map(Request::id).collect();
        let closed = self.decide(id, "Step::wait_first_timeout", &asked, WaitMode::First, Some(timeout))?;
        Ok(match self.decode::<T>(id, closed)? {
            (WaitOutcome::Received, mut values) if values.len() == 1 => Waited::Received(values.remove(0)),
            _ => Waited::TimedOut,
        })
    }

    /// The wait engine of every wait helper (ADR 0044): replays the saved wait `id` of `mode` for the requests `asked`,
    /// with a deadline when it has a `timeout`, or latches its new entry, its decision, its re-armed deadline or its
    /// parking. Returns only a committed decision; `helper` names the caller in diagnostics.
    fn decide(
        &mut self,
        id: &str,
        helper: &str,
        asked: &[&MessageId],
        mode: WaitMode,
        timeout: Option<Duration>,
    ) -> Result<Closed, HandlerError> {
        if self.latch.is_some() {
            return Err(HandlerError::new(Suspend));
        }
        let saved = match self.replay(id, "wait") {
            Err(reason) => return Err(self.refuse(reason)),
            Ok(saved) => saved,
        };
        let Some(JournalEntry::Wait { requests, mode: saved_mode, until, outcome, received, .. }) = saved else {
            let latch = match self.new_wait(id, asked, mode, timeout) {
                Ok(entry) => Latch::New(entry, None),
                Err(reason) => Latch::Permanent(reason),
            };
            self.latch = Some(latch);
            return Err(HandlerError::new(Suspend));
        };
        if saved_mode != mode || requests.len() != asked.len() || requests.iter().zip(asked).any(|(s, a)| s != *a) {
            let saved: Vec<&MessageId> = requests.iter().collect();
            return Err(self.refuse(format!(
                "wait `{id}` is saved for {}, not for {}",
                listed(&saved, saved_mode),
                listed(asked, mode)
            )));
        }
        match (until, timeout) {
            (Some(_), None) => {
                return Err(self.refuse(format!(
                    "wait `{id}` is saved with a deadline, which {helper} cannot replay: the code called a wait with a \
                     timeout there"
                )));
            }
            (None, Some(_)) => {
                return Err(self.refuse(format!(
                    "wait `{id}` is saved without a deadline, which {helper} cannot replay: the code called a wait \
                     without a timeout there"
                )));
            }
            _ => {}
        }
        if let Some(outcome) = outcome {
            let mut selected = Vec::with_capacity(received.len());
            for request in &received {
                let (Some(at), Some(value)) = (requests.iter().position(|r| r == request), self.reply(request)) else {
                    return Err(self.refuse(format!("wait `{id}` is saved closed without the reply to {request}")));
                };
                selected.push((at, request.clone(), value.clone()));
            }
            return Ok(Closed { outcome, selected });
        }
        let index = self.cursor - 1;
        let accepted = self.accepted();
        let latch = match select(&requests, mode, &accepted) {
            Some(received) => Latch::Close(index, WaitOutcome::Received, received),
            None => match until {
                Some(until) if self.now >= until => {
                    Latch::Close(index, WaitOutcome::TimedOut, timed_out(&requests, mode, &accepted))
                }
                Some(until) => Latch::Waiting(until),
                None => Latch::Parked,
            },
        };
        self.latch = Some(latch);
        Err(HandlerError::new(Suspend))
    }

    /// Decodes the selected replies of the committed decision of wait `id` as `T`s, in request order, under the
    /// in-progress marker, which stays if a decoder panics and is caught; returns the outcome and each value with its
    /// position in the request list.
    fn decode<T: DeserializeOwned>(
        &mut self,
        id: &str,
        closed: Closed,
    ) -> Result<(WaitOutcome, Vec<(usize, T)>), HandlerError> {
        self.latch = Some(Latch::InProgress("wait", id.to_owned()));
        let mut values = Vec::with_capacity(closed.selected.len());
        for (at, request, value) in closed.selected {
            match T::deserialize(&value) {
                Ok(value) => values.push((at, value)),
                Err(e) => {
                    return Err(self.refuse(format!(
                        "the saved reply of wait `{id}` does not decode as the type asked for, at position {at} \
                         (request {request}): {e}; the outcome stays saved: fix the code and send a public Continue"
                    )));
                }
            }
        }
        self.latch = None;
        Ok((closed.outcome, values))
    }

    /// The entry of a new wait `id` of `mode` for `asked`: checked for room, its list, its requests and, when it has a
    /// `timeout`, its deadline, in that order and before an early reply is used; closed as received when the run keeps
    /// enough replies.
    fn new_wait(
        &self,
        id: &str,
        asked: &[&MessageId],
        mode: WaitMode,
        timeout: Option<Duration>,
    ) -> Result<JournalEntry, String> {
        self.room(id, "wait")?;
        if asked.is_empty() || asked.len() > MAX_WAIT_REQUESTS {
            return Err(format!("wait `{id}` names 1 through {MAX_WAIT_REQUESTS} requests, had {}", asked.len()));
        }
        let mut listed = HashSet::new();
        if let Some(twice) = asked.iter().find(|request| !listed.insert(**request)) {
            return Err(format!("wait `{id}` names the request {twice} twice"));
        }
        self.unused(id, asked)?;
        let until = timeout.map(|timeout| wake_time("wait", id, self.now, timeout)).transpose()?;
        let requests: Vec<MessageId> = asked.iter().map(|request| (*request).clone()).collect();
        let selected = select(&requests, mode, &self.accepted());
        Ok(JournalEntry::Wait {
            id: id.to_owned(),
            requests,
            mode,
            until,
            outcome: selected.is_some().then_some(WaitOutcome::Received),
            received: selected.unwrap_or_default(),
        })
    }

    /// Checks that a new wait `id` names only earlier requests of the journal that no other wait names.
    fn unused(&self, id: &str, asked: &[&MessageId]) -> Result<(), String> {
        let mut known = HashSet::new();
        let mut waited = HashMap::new();
        for entry in &self.journal {
            match entry {
                JournalEntry::Send { message, reply: true, .. } => {
                    known.insert(message);
                }
                JournalEntry::Wait { id: other, requests, .. } => {
                    waited.extend(requests.iter().map(|request| (request, other.as_str())));
                }
                _ => {}
            }
        }
        for request in asked {
            if !known.contains(request) {
                return Err(format!("wait `{id}` names {request}, no earlier request of the journal"));
            }
            if let Some(other) = waited.get(request) {
                return Err(format!("wait `{id}` names the request {request} of the wait `{other}`"));
            }
        }
        Ok(())
    }

    /// The kept reply to `request`.
    fn reply(&self, request: &MessageId) -> Option<&Value> {
        self.replies.iter().find_map(|(kept, value)| (kept == request).then_some(value))
    }

    /// The requests of the kept replies, in the order they were accepted.
    fn accepted(&self) -> Vec<&MessageId> {
        self.replies.iter().map(|(request, _)| request).collect()
    }
    /// Converts and encodes a new send or request, in the order of ADR 0044: the target and its address, a fresh
    /// message id, the message `make` builds for that id, then the initial state. A panic here leaves the entry's
    /// in-progress latch.
    fn prepare<'a, R, S>(
        &self,
        id: &str,
        kind: &str,
        to: impl Into<Target<'a, R, S>>,
        make: impl FnOnce(&MessageId) -> R::Message,
    ) -> Result<(JournalEntry, Outbox), String>
    where
        R: ActorRef + 'a,
        S: Serialize + ?Sized + 'a,
    {
        let target = to.into();
        let address = target.actor.address().map_err(|e| format!("the target of {kind} `{id}` has no address: {e}"))?;
        if address == self.address {
            return Err(format!("{kind} `{id}` names the run itself, {address}; a flow sends nothing to itself"));
        }
        let message = MessageId::random();
        let encoded = cicada::message::encode(&make(&message))
            .map_err(|e| format!("the message of {kind} `{id}` does not encode: {e}"))?;
        let state = match target.create {
            None => None,
            Some((state, _)) => Some(
                serde_json::to_value(state)
                    .map_err(|e| format!("the initial state of {kind} `{id}` does not encode: {e}"))?,
            ),
        };
        let entry = JournalEntry::Send {
            id: id.to_owned(),
            to: address,
            message,
            create: target.create.map(|(_, kind)| kind),
            reply: kind == "request",
        };
        Ok((entry, Outbox { message_type: encoded.message_type, payload: encoded.payload, state }))
    }

    /// Checks that the journal has room for the new entry `id`.
    fn room(&self, id: &str, kind: &str) -> Result<(), String> {
        if self.journal.len() >= MAX_ENTRIES {
            return Err(format!(
                "{kind} `{id}` would be entry {} of a journal that holds at most {MAX_ENTRIES}",
                MAX_ENTRIES + 1
            ));
        }
        Ok(())
    }

    /// Matches a call of `id` of `kind` against the next saved entry: that entry, or none at the end of the journal.
    fn replay(&mut self, id: &str, kind: &str) -> Result<Option<JournalEntry>, String> {
        check_id(id)?;
        if !self.seen.insert(id.to_owned()) {
            return Err(format!("the flow calls the id `{id}` twice"));
        }
        let Some(entry) = self.journal.get(self.cursor) else {
            return Ok(None);
        };
        match entry {
            saved if saved.kind() == kind && saved.id() == id => {
                let saved = saved.clone();
                self.cursor += 1;
                Ok(Some(saved))
            }
            other => Err(format!(
                "the flow calls {kind} `{id}` where its journal holds {} `{}` at entry {}: {}",
                other.kind(),
                other.id(),
                self.cursor + 1,
                unreplayable(other)
            )),
        }
    }

    /// Latches a permanent refusal and returns the suspension.
    fn refuse(&mut self, reason: String) -> HandlerError {
        self.latch = Some(Latch::Permanent(reason));
        HandlerError::new(Suspend)
    }
}

/// The wake time of a new sleep, or the deadline of a new wait, of `kind`: `now + duration`, rounded up to
/// microseconds; the duration at most [`MAX_DELAY`] and the time at most [`MAX_INSTANT`].
fn wake_time(kind: &str, id: &str, now: Timestamp, duration: Duration) -> Result<Timestamp, String> {
    if duration > MAX_DELAY {
        return Err(format!(
            "{kind} `{id}` lasts {}, longer than the most a {kind} may last, {}",
            time::format_duration(duration),
            time::format_duration(MAX_DELAY)
        ));
    }
    let until = time::add(now, duration).map(time::ceil).map_err(|e| format!("{kind} `{id}` cannot end: {e}"))?;
    if until > MAX_INSTANT {
        return Err(format!(
            "{kind} `{id}` would end at {}, after {}",
            time::format_instant(until),
            time::format_instant(MAX_INSTANT)
        ));
    }
    Ok(until)
}

/// Encodes a new result and checks it the way the core checks a state, with the three levels the state adds.
fn encode<T: Serialize>(id: &str, value: &T) -> Result<Value, String> {
    let result = serde_json::to_value(value).map_err(|e| format!("the result of step `{id}` does not encode: {e}"))?;
    check_result(&result, 0).map_err(|problem| format!("the result of step `{id}` cannot be saved: {problem}"))?;
    Ok(result)
}

/// Checks that no string or key holds a NUL and that `value`, at `depth`, is at most [`MAX_RESULT_DEPTH`] containers
/// deep; it stops at the first level beyond, so its recursion is bounded.
fn check_result(value: &Value, depth: usize) -> Result<(), String> {
    let deeper = || format!("it is more than {MAX_RESULT_DEPTH} arrays and objects deep");
    match value {
        Value::String(s) if s.contains('\0') => Err("a string holds a NUL character".to_owned()),
        Value::Array(items) => {
            if depth >= MAX_RESULT_DEPTH {
                return Err(deeper());
            }
            items.iter().try_for_each(|item| check_result(item, depth + 1))
        }
        Value::Object(map) => {
            if depth >= MAX_RESULT_DEPTH {
                return Err(deeper());
            }
            map.iter().try_for_each(|(key, item)| {
                if key.contains('\0') {
                    return Err("a key holds a NUL character".to_owned());
                }
                check_result(item, depth + 1)
            })
        }
        _ => Ok(()),
    }
}

/// The runner of the flow `F`: each run is one actor of type `F::TYPE`, hosted like any other.
///
/// A public `Continue` to a running run cancels its armed continuation, if any, and arms a fresh one due now with
/// `F::retry()` and its own correlation id, without running flow code; so a host starts a run, or recovers one whose
/// continuation was cancelled, purged or failed, with a `Continue`, and a poke resets the attempts. Only the armed
/// `Continue` executes the flow: it replays the journal, and the first new step's result commits with the next armed
/// `Continue`, which keeps the correlation id. A new sleep commits with an armed `Continue` due at its wake time, and
/// a replayed sleep not yet due arms one due at that same time and saves nothing. A new send or request commits its
/// create, its message and its entry with the next armed `Continue`. A new wait for which the run keeps enough replies
/// (every one of an all wait, any one of a first wait) commits closed with the next armed `Continue`; one without them
/// commits open and clears `armed`, and a replayed open wait clears it again, saving nothing else. A new wait with a
/// deadline and too few replies commits open with an armed `Continue` due at its deadline; replayed before the
/// deadline, it arms one due then and saves nothing; replayed at or after it with too few replies kept, it commits timed
/// out, selecting the kept replies of an all wait, with the next armed `Continue`, due now; a replayed open wait for
/// which the run keeps enough replies commits received the same way, deadline or not. A flow that returns `Ok` after
/// replaying every
/// entry marks the run done and clears `armed`; a done or cancelled run ignores `Continue` and keeps its state until it
/// is deleted. An execution that fails commits nothing; its message follows its retry policy, and its watchers are told
/// as for any actor. The runner installs no watch.
///
/// A `Reply` runs no flow code. A running run ignores it, writing nothing, unless it names a request of the run, comes
/// from the address the request went to, and its request has no kept reply and no closed wait; only then is its value
/// checked (no NUL, at most 61 levels deep, or it fails for good) and kept, after the replies kept before it. When the
/// open wait at the end of the journal names its request and now has enough replies, the same commit closes that wait
/// as received with its selection (every request in request order, or the listed request accepted earliest), deadline
/// or not and without reading the clock, cancels the armed continuation, if any (a deadline's), and arms a fresh one
/// due now with `F::retry()` and the reply's correlation id; otherwise `armed` stays, also for a partial reply of a
/// join. A done or cancelled run ignores every `Reply`.
///
/// A `Cancel` runs no flow code either. To a running run it cancels the armed continuation, unless that is the `Cancel`
/// itself, marks the run cancelled and clears `armed`, in one commit; the input, both journals, the replies and an open
/// wait stay as they were, and it sends nothing. It does not preempt a turn already running, which may commit first. A
/// done or cancelled run ignores it, writing nothing. This release refuses every message to a cancelling run, with a
/// diagnostic that names the later release it needs (ADR 0044). A host or a worker of 0.18.3 or earlier refuses
/// `Cancel` for good: upgrade every host and worker of a flow type before sending it. A host or a worker of 0.18.1
/// refuses the reply to a wait with a deadline, and one of 0.18.2 the reply to a join of several requests or of mode
/// first, for good, although the contract is the same: upgrade them all before a flow waits that way.
pub struct FlowActor<F> {
    flow: F,
}

impl<F: Flow> FlowActor<F> {
    /// The runner of `flow`.
    pub fn new(flow: F) -> Self {
        Self { flow }
    }
}

impl<F: Flow> Actor for FlowActor<F> {
    type Ref = FlowRun<F>;
    type State = FlowState<F::Input>;

    async fn handle(&self, turn: &mut Turn<Self>, message: FlowMsg) -> HandlerResult {
        match turn.state().status {
            FlowStatus::Running => {}
            FlowStatus::Done | FlowStatus::Cancelled => return Ok(()),
            FlowStatus::Cancelling => {
                return Err(HandlerError::permanent(later_release("the cleanup of a cancelling run")));
            }
        }
        let reply = match message {
            FlowMsg::Continue => None,
            FlowMsg::Reply { request, value } => Some((request, value)),
            FlowMsg::Cancel => return cancel(turn),
        };
        if let Some((request, value)) = reply {
            return accept::<F>(turn, request, value);
        }
        if turn.state().armed.as_ref() != Some(&turn.message().id) {
            if let Some(old) = turn.state().armed.clone() {
                turn.cancel(&old)?;
            }
            return arm(turn, None);
        }
        let state = turn.state();
        let mut step = Step::new(state.journal.clone(), turn.self_address(), turn.now());
        step.replies = state.replies.iter().map(|r| (r.request.clone(), r.value.clone())).collect();
        let returned = self.flow.run(&mut step, state.input.clone()).await;
        match step.latch.take() {
            Some(Latch::InProgress("step", id)) => Err(HandlerError::permanent(format!(
                "step `{id}` was abandoned while its closure ran (its future was dropped, or it panicked); a timeout \
                 belongs inside the closure"
            ))),
            Some(Latch::InProgress("wait", id)) => Err(HandlerError::permanent(format!(
                "wait `{id}` panicked while its saved reply was decoded, and the flow caught the panic; the outcome \
                 stays saved"
            ))),
            Some(Latch::InProgress(kind, id)) => Err(HandlerError::permanent(format!(
                "{kind} `{id}` panicked while its target, message or initial state was converted, built or encoded, \
                 and the flow caught the panic"
            ))),
            Some(Latch::Failed(e)) => Err(e),
            Some(Latch::Permanent(reason)) => Err(HandlerError::permanent(reason)),
            Some(Latch::New(entry, outbox)) => {
                let (wake, park) = match &entry {
                    JournalEntry::Sleep { until, .. } => (Some(*until), false),
                    JournalEntry::Wait { outcome: None, until, .. } => (*until, until.is_none()),
                    _ => (None, false),
                };
                if let Some(outbox) = outbox {
                    apply(turn, &entry, *outbox).await?;
                }
                turn.state_mut().journal.push(entry);
                if park {
                    turn.state_mut().armed = None;
                    return Ok(());
                }
                arm(turn, wake)
            }
            Some(Latch::Waiting(until)) => arm(turn, Some(until)),
            Some(Latch::Close(index, outcome, selected)) => {
                close(&mut turn.state_mut().journal[index], outcome, selected);
                arm(turn, None)
            }
            Some(Latch::Parked) => {
                turn.state_mut().armed = None;
                Ok(())
            }
            None => {
                returned?;
                if step.cursor < step.journal.len() {
                    let next = &step.journal[step.cursor];
                    return Err(HandlerError::permanent(format!(
                        "the flow returned before reaching its saved {} `{}`: {}",
                        next.kind(),
                        next.id(),
                        unreplayable(next)
                    )));
                }
                let state = turn.state_mut();
                state.status = FlowStatus::Done;
                state.armed = None;
                Ok(())
            }
        }
    }
}

/// Handles a `Reply` to a running run, without flow code (ADR 0044): the checks that ignore it come before the check of
/// its value, and nothing is written unless it is kept.
fn accept<F: Flow>(turn: &mut Turn<FlowActor<F>>, request: MessageId, value: Value) -> HandlerResult {
    let state = turn.state();
    let target = state.journal.iter().chain(&state.cleanup).find_map(|entry| match entry {
        JournalEntry::Send { to, message, reply: true, .. } if *message == request => Some(to),
        _ => None,
    });
    let Some(target) = target else {
        return Ok(());
    };
    if turn.message().from.as_ref() != Some(target) || state.replies.iter().any(|r| r.request == request) {
        return Ok(());
    }
    let closed = |entry: &JournalEntry| matches!(entry, JournalEntry::Wait { requests, outcome: Some(_), .. } if requests.contains(&request));
    if state.journal.iter().chain(&state.cleanup).any(closed) {
        return Ok(());
    }
    check_result(&value, 0).map_err(|problem| {
        HandlerError::permanent(format!("the reply to request {request} cannot be kept: {problem}"))
    })?;
    let lists = |entry: &JournalEntry| matches!(entry, JournalEntry::Wait { requests, outcome: None, .. } if requests.contains(&request));
    let open = state.journal.last().is_some_and(lists);
    let current = turn.message().id.clone();
    let state = turn.state_mut();
    state.replies.push(FlowReply { request, value });
    if !open {
        return Ok(());
    }
    let accepted: Vec<&MessageId> = state.replies.iter().map(|r| &r.request).collect();
    let selected = match state.journal.last() {
        Some(JournalEntry::Wait { requests, mode, .. }) => select(requests, *mode, &accepted),
        _ => None,
    };
    let (Some(selected), Some(last)) = (selected, state.journal.last_mut()) else {
        return Ok(()); // a partial reply of a join: its deadline and armed continuation stay
    };
    close(last, WaitOutcome::Received, selected);
    if let Some(old) = state.armed.clone().filter(|old| *old != current) {
        turn.cancel(&old)?;
    }
    arm(turn, None)
}

/// Handles a `Cancel` to a running run, without flow code (ADR 0044): cancels the armed continuation unless it is the
/// current message, marks the run cancelled and clears `armed`, in the turn's one commit. The journals, the replies and
/// an open wait stay as they were.
fn cancel<F: Flow>(turn: &mut Turn<FlowActor<F>>) -> HandlerResult {
    if let Some(old) = turn.state().armed.clone().filter(|old| *old != turn.message().id) {
        turn.cancel(&old)?;
    }
    let state = turn.state_mut();
    state.status = FlowStatus::Cancelled;
    state.armed = None;
    Ok(())
}

/// Closes an open wait with `decided` and the replies it selected.
fn close(entry: &mut JournalEntry, decided: WaitOutcome, selected: Vec<MessageId>) {
    if let JournalEntry::Wait { outcome, received, .. } = entry {
        *outcome = Some(decided);
        *received = selected;
    }
}

/// A committed decision of a wait, as replay returns it: its outcome and, in the order of its requests, the position,
/// the request and the kept value of each reply it selected.
struct Closed {
    outcome: WaitOutcome,
    selected: Vec<(usize, MessageId, Value)>,
}

/// What a wait of `mode` over `requests` selects once the run keeps enough replies, given the requests of the kept
/// replies in the order they were accepted (ADR 0044): for all, every request, in request order; for first, the listed
/// request accepted earliest, whatever its place in `requests`. `None` while too few replies are kept.
fn select(requests: &[MessageId], mode: WaitMode, accepted: &[&MessageId]) -> Option<Vec<MessageId>> {
    match mode {
        WaitMode::All => {
            let kept: HashSet<&MessageId> = accepted.iter().copied().collect();
            requests.iter().all(|request| kept.contains(request)).then(|| requests.to_vec())
        }
        WaitMode::First => accepted.iter().find(|kept| requests.contains(kept)).map(|winner| vec![(*winner).clone()]),
    }
}

/// What a wait of `mode` over `requests` that has too few replies selects when it times out: for all, the requests whose
/// replies are kept, in request order; for first, none.
fn timed_out(requests: &[MessageId], mode: WaitMode, accepted: &[&MessageId]) -> Vec<MessageId> {
    match mode {
        WaitMode::All => requests.iter().filter(|request| accepted.contains(request)).cloned().collect(),
        WaitMode::First => Vec::new(),
    }
}

/// The requests of a wait of `mode`, as diagnostics name them.
fn listed(requests: &[&MessageId], mode: WaitMode) -> String {
    let names: Vec<&str> = requests.iter().map(|request| request.as_str()).collect();
    match (mode, names.as_slice()) {
        (WaitMode::All, [one]) => format!("the request {one}"),
        (WaitMode::All, names) => format!("all of the requests {}", names.join(", ")),
        (WaitMode::First, names) => format!("the first of the requests {}", names.join(", ")),
    }
}

/// Buffers the create and the message of a new send or request entry with the turn's raw operations, so that they
/// commit with the entry and the next continuation, or not at all. The message keeps the entry's id and the executing
/// message's correlation id, and has no retry policy and the default retention. A failed read of the registry keeps its
/// typed error, so the failure's cause is `node` and the executing `Continue`'s policy retries it; a refusal is
/// permanent.
async fn apply<F: Flow>(turn: &mut Turn<FlowActor<F>>, entry: &JournalEntry, outbox: Outbox) -> HandlerResult {
    let JournalEntry::Send { id, to, message, create, .. } = entry else {
        unreachable!("only a send entry has an outbox");
    };
    let kind = entry.kind();
    let failed = |e: TurnError| match e {
        TurnError::Store(_) => HandlerError::new(SendFailed { kind, id: id.clone(), to: to.clone(), source: e }),
        e => HandlerError::permanent(format!("{kind} `{id}` to {to} is refused: {e}")),
    };
    match (create, outbox.state) {
        (Some(CreateKind::Independent), Some(state)) => turn.create_raw(to, state).await.map_err(failed)?,
        (Some(CreateKind::Child), Some(state)) => turn.create_child_raw(to, state).await.map_err(failed)?,
        (None, None) => {}
        _ => unreachable!("a send creates exactly when it has an initial state"),
    }
    let mut options = SendOptions::new();
    options.id = Some(message.clone());
    options.correlation_id = turn.message().correlation_id.clone();
    turn.send_raw(to, &outbox.message_type, outbox.payload, options).await.map_err(failed)?;
    Ok(())
}

/// A send or request whose check of the type registry failed: the store's error stays in its chain of causes.
#[derive(Debug)]
struct SendFailed {
    kind: &'static str,
    id: String,
    to: ActorAddress,
    source: TurnError,
}

impl fmt::Display for SendFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} `{}` to {} could not be checked", self.kind, self.id, self.to)
    }
}

impl std::error::Error for SendFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Why replay stopped at a saved entry.
fn unreplayable(_saved: &JournalEntry) -> String {
    "changed flow code cannot replay this run".to_owned()
}

/// The diagnostic of a message or a phase this release refuses for good.
fn later_release(what: &str) -> String {
    format!(
        "this release of cicada-actors refuses {what} for good; a later release can handle it, once every host and \
         worker of the flow type runs it"
    )
}

/// Self-sends a `Continue`, due at `wake` or now, with the flow's retry policy and the incoming correlation id, and
/// saves its id.
fn arm<F: Flow>(turn: &mut Turn<FlowActor<F>>, wake: Option<Timestamp>) -> HandlerResult {
    let mut options = SendOptions::new().retry(F::retry());
    if let Some(until) = wake {
        options = options.at(until);
    }
    options.correlation_id = turn.message().correlation_id.clone();
    let id = turn.send_self_with(FlowMsg::Continue, options).map_err(HandlerError::permanent_from)?;
    turn.state_mut().armed = Some(id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(json: Value) -> Result<FlowState<Value>, serde_json::Error> {
        serde_json::from_value(json)
    }

    fn step(id: &str) -> Value {
        serde_json::json!({"kind": "step", "id": id, "result": null})
    }

    #[test]
    fn the_minimal_state_has_defaults_and_ignores_unknown_fields() {
        let state = decode(serde_json::json!({"input": {"user": "u1"}, "other": 1})).unwrap();
        assert_eq!(state, FlowState::new(serde_json::json!({"user": "u1"})));
        assert!(decode(serde_json::json!({})).is_err(), "input is required");
        let done = decode(serde_json::json!({"input": 1, "status": "done", "armed": null})).unwrap();
        assert_eq!(done.status, FlowStatus::Done);
        let running = decode(serde_json::json!({"input": 1, "journal": [step("a")], "armed": null})).unwrap();
        assert_eq!((running.status, running.armed), (FlowStatus::Running, None), "running with no armed is valid");
    }

    #[test]
    fn ids_are_checked() {
        for id in ["a", "A-z_0.9~:", &"x".repeat(100)] {
            assert!(check_id(id).is_ok(), "{id}");
            assert!(decode(serde_json::json!({"input": 1, "journal": [step(id)]})).is_ok(), "{id}");
        }
        for id in ["", "a/b", "a b", "é", "a\0", &"x".repeat(101)] {
            assert!(check_id(id).is_err(), "{id:?}");
            let decoded = decode(serde_json::json!({"input": 1, "journal": [step(id)]}));
            assert!(decoded.is_err_and(|e| e.to_string().contains("a step id is")), "{id:?}");
        }
    }

    #[test]
    fn an_inconsistent_state_is_refused() {
        let late = time::format_instant(time::add(MAX_INSTANT, std::time::Duration::from_micros(1)).unwrap());
        let sleep = |until: &str| serde_json::json!({"kind": "sleep", "id": "a", "until": until});
        let many: Vec<Value> = (0..1001).map(|i| step(&format!("s{i}"))).collect();
        for (state, why) in [
            (serde_json::json!({"input": 1, "journal": [step("a"), step("a")]}), "twice"),
            (serde_json::json!({"input": 1, "journal": [step("a"), sleep("2026-01-01T00:00:00Z")]}), "twice"),
            (serde_json::json!({"input": 1, "journal": [sleep(&late)]}), "must not end after"),
            (serde_json::json!({"input": 1, "journal": many}), "at most 1000"),
            (serde_json::json!({"input": 1, "status": "done", "armed": "m1"}), "no armed"),
        ] {
            let decoded = decode(state.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {why}");
        }
        let full: Vec<Value> = (0..1000).map(|i| step(&format!("s{i}"))).collect();
        assert!(decode(serde_json::json!({"input": 1, "journal": full})).is_ok());
        let last = time::format_instant(MAX_INSTANT);
        assert!(decode(serde_json::json!({"input": 1, "journal": [sleep(&last)]})).is_ok());
        assert!(decode(serde_json::json!({"input": 1, "journal": [{"kind": "step", "id": "a"}]})).is_err());
    }

    #[test]
    fn results_are_read_by_step_id() {
        let state = decode(serde_json::json!({"input": 1, "journal": [
            {"kind": "step", "id": "n", "result": null},
            {"kind": "step", "id": "a", "result": "a1"},
            {"kind": "sleep", "id": "w", "until": "2026-11-14T00:00:00Z"},
        ]}))
        .unwrap();
        assert_eq!(state.result::<String>("a").unwrap().unwrap(), "a1");
        state.result::<()>("n").unwrap().unwrap();
        assert!(state.result::<i32>("a").unwrap().is_err());
        assert!(state.result::<Value>("w").is_none(), "a sleep is no step");
        assert!(state.result::<Value>("x").is_none());
    }

    fn nested(depth: usize) -> Value {
        (0..depth).fold(Value::Null, |inner, _| Value::Array(vec![inner]))
    }

    #[test]
    fn results_are_checked_before_they_are_saved() {
        assert_eq!(encode("a", &()), Ok(Value::Null));
        assert!(encode("a", &nested(MAX_RESULT_DEPTH)).is_ok());
        assert!(encode("a", &nested(MAX_RESULT_DEPTH + 1)).unwrap_err().contains("more than 61"));
        assert!(encode("a", &"a\0b").unwrap_err().contains("NUL"));
        assert!(encode("a", &serde_json::json!({"k\0": 1})).unwrap_err().contains("NUL"));
        let map: std::collections::HashMap<(u8, u8), u8> = [((1, 2), 3)].into();
        assert!(encode("a", &map).unwrap_err().contains("does not encode"));
        let deep_object = (0..MAX_RESULT_DEPTH + 1).fold(Value::Null, |inner, _| serde_json::json!({"k": inner}));
        assert!(encode("a", &deep_object).is_err());
    }

    #[test]
    fn a_run_ref_is_one_transparent_string() {
        struct NoTraits;
        impl Flow for NoTraits {
            const TYPE: &'static str = "test.flow";
            type Input = ();
            async fn run(&self, _step: &mut Step, _input: ()) -> HandlerResult {
                Ok(())
            }
        }
        let run = FlowRun::<NoTraits>::named("a/b");
        assert_eq!(serde_json::to_value(&run).unwrap(), serde_json::json!("a/b"));
        assert_eq!(run.address().unwrap().as_str(), "test.flow/a%2Fb");
        let back: FlowRun<NoTraits> = serde_json::from_value(serde_json::json!("k")).unwrap();
        assert_eq!(back.clone().key, "k");
        assert!(cicada::address::RefBinding::<FlowRun<NoTraits>>::new().is_ok());
    }
}

#[cfg(test)]
mod state_tests {
    //! The typed checks of ADR 0044 on the shared wire of sends, waits, replies and cleanup.

    use super::*;
    use serde_json::json;

    fn decode(json: Value) -> Result<FlowState<Value>, String> {
        serde_json::from_value(json).map_err(|e| e.to_string())
    }

    /// Asserts that `state` is refused with `why` in the error.
    #[track_caller]
    fn refused(state: Value, why: &str) {
        match decode(state.clone()) {
            Ok(_) => panic!("accepted {state}, expected {why}"),
            Err(e) => assert!(e.contains(why), "{e} lacks {why} for {state}"),
        }
    }

    #[track_caller]
    fn accepted(state: Value) -> FlowState<Value> {
        decode(state.clone()).unwrap_or_else(|e| panic!("refused {state}: {e}"))
    }

    fn step(id: &str) -> Value {
        json!({"kind": "step", "id": id, "result": null})
    }

    fn send(id: &str, message: &str) -> Value {
        json!({"kind": "send", "id": id, "to": "jobs/j1", "message": message})
    }

    fn request(id: &str, message: &str) -> Value {
        json!({"kind": "send", "id": id, "to": "jobs/j1", "message": message, "create": null, "reply": true})
    }

    fn open(id: &str, requests: &[&str], mode: &str) -> Value {
        json!({"kind": "wait", "id": id, "requests": requests, "mode": mode})
    }

    fn closed(id: &str, requests: &[&str], mode: &str, outcome: &str, received: &[&str]) -> Value {
        let until = if outcome == "timedOut" { json!("2026-11-14T00:00:00Z") } else { Value::Null };
        json!({"kind": "wait", "id": id, "requests": requests, "mode": mode, "until": until, "outcome": outcome,
            "received": received})
    }

    fn reply(request: &str) -> Value {
        json!({"request": request, "value": null})
    }

    fn running(journal: Vec<Value>, replies: Vec<Value>) -> Value {
        json!({"input": 1, "journal": journal, "replies": replies})
    }

    fn with(status: &str, journal: Vec<Value>, replies: Vec<Value>, cleanup: Vec<Value>) -> Value {
        json!({"input": 1, "status": status, "journal": journal, "replies": replies, "cleanup": cleanup})
    }

    /// Scenario: Literal progress state; and Journal wire forms.
    #[test]
    fn every_entry_kind_round_trips_with_its_defaults() {
        let state = accepted(running(
            vec![
                step("notify"),
                json!({"kind": "sleep", "id": "wait", "until": "2026-11-14T00:00:00Z"}),
                send("start", "m1"),
                json!({"kind": "send", "id": "child", "to": "jobs/j2", "message": "m2", "create": "child"}),
                request("ask", "m3"),
                open("answer", &["m3"], "all"),
            ],
            vec![],
        ));
        assert_eq!(
            state.journal[2],
            JournalEntry::Send {
                id: "start".into(),
                to: ActorAddress::parse("jobs/j1").unwrap(),
                message: MessageId::new("m1").unwrap(),
                create: None,
                reply: false,
            }
        );
        let written = serde_json::to_value(&state).unwrap();
        assert_eq!(written["journal"][0], step("notify"));
        assert_eq!(
            written["journal"][2],
            json!({"kind": "send", "id": "start", "to": "jobs/j1", "message": "m1", "create": null, "reply": false})
        );
        assert_eq!(written["journal"][3]["create"], json!("child"));
        assert_eq!(
            written["journal"][5],
            json!({"kind": "wait", "id": "answer", "requests": ["m3"], "mode": "all", "until": null, "outcome": null,
                "received": []})
        );
        assert_eq!((written["replies"].clone(), written["cleanup"].clone()), (json!([]), json!([])));
        assert_eq!(decode(written).unwrap(), state);
        refused(running(vec![json!({"kind": "wait", "id": "w", "requests": ["m1"]})], vec![]), "mode");
        refused(running(vec![json!({"kind": "send", "id": "s", "to": "jobs/j1"})], vec![]), "message");
        refused(running(vec![json!({"kind": "send", "id": "s", "to": "Jobs", "message": "m"})], vec![]), "");
    }

    /// Scenario: Old supported progress.
    #[test]
    fn an_old_state_decodes_without_a_new_depth_check_of_its_results() {
        let deep = (0..70).fold(Value::Null, |inner, _| Value::Array(vec![inner]));
        let old = json!({"input": 1, "journal": [{"kind": "step", "id": "a", "result": deep}], "status": "running",
            "armed": "m1"});
        let state = accepted(old);
        assert_eq!((state.replies.len(), state.cleanup.len()), (0, 0));
    }

    /// Scenario: Required nullable values.
    #[test]
    fn reply_values_are_required_and_may_be_null() {
        let message: FlowMsg =
            serde_json::from_value(json!({"type": "Reply", "request": "m1", "value": null})).unwrap();
        assert_eq!(message, FlowMsg::Reply { request: MessageId::new("m1").unwrap(), value: Value::Null });
        assert!(serde_json::from_value::<FlowMsg>(json!({"type": "Reply", "request": "m1"})).is_err());
        assert_eq!(serde_json::to_value(FlowMsg::Cancel).unwrap(), json!({"type": "Cancel"}));

        let state = accepted(running(vec![request("ask", "m1")], vec![reply("m1")]));
        assert_eq!(state.replies, [FlowReply { request: MessageId::new("m1").unwrap(), value: Value::Null }]);
        refused(running(vec![request("ask", "m1")], vec![json!({"request": "m1"})]), "missing field `value`");
    }

    /// Scenario: Invalid state.
    #[test]
    fn counts_ids_message_ids_and_deadlines_are_checked() {
        let late = time::format_instant(time::add(MAX_INSTANT, Duration::from_micros(1)).unwrap());
        refused(running(vec![send("a", "m1"), send("b", "m1")], vec![]), "repeats the message id m1");
        refused(with("cancelling", vec![send("a", "m1")], vec![], vec![send("b", "m1")]), "repeats the message id");
        refused(running(vec![send("a", "m1"), step("a")], vec![]), "holds the id `a` twice");
        refused(running(vec![send("a/b", "m1")], vec![]), "a step id is");
        let mut wait = open("w", &["m1"], "all");
        wait["until"] = json!(late);
        refused(running(vec![request("r", "m1"), wait], vec![]), "wait `w` must not end after");
        let full: Vec<Value> = (0..1000).map(|i| step(&format!("s{i}"))).collect();
        accepted(with("cancelling", full.clone(), vec![], full.clone()));
        let mut over = full.clone();
        over.push(step("x"));
        refused(with("cancelling", vec![], vec![], over), "the cleanup journal holds at most 1000");

        let sends: Vec<Value> = (0..1000).map(|i| request(&format!("r{i}"), &format!("m{i}"))).collect();
        let cleanup: Vec<Value> = (0..1000).map(|i| request(&format!("r{i}"), &format!("c{i}"))).collect();
        let mut replies: Vec<Value> = (0..1000).map(|i| reply(&format!("m{i}"))).collect();
        replies.extend((0..1000).map(|i| reply(&format!("c{i}"))));
        accepted(with("cancelling", sends.clone(), replies.clone(), cleanup.clone()));
        replies.push(reply("m0"));
        refused(with("cancelling", sends, replies, cleanup), "at most 2000 replies");
    }

    #[test]
    fn replies_name_requests_once() {
        refused(running(vec![], vec![reply("m1")]), "names no request");
        refused(running(vec![send("s", "m1")], vec![reply("m1")]), "names no request");
        refused(running(vec![request("r", "m1")], vec![reply("m1"), reply("m1")]), "two replies to m1");
        accepted(with("cancelling", vec![], vec![reply("c1")], vec![request("r", "c1")]));
    }

    /// Scenario: Separate phases.
    #[test]
    fn phases_have_separate_ids_and_the_status_rules() {
        accepted(with("cancelling", vec![step("a")], vec![], vec![step("a")]));
        accepted(with("cancelled", vec![step("a")], vec![], vec![step("a")]));
        refused(with("running", vec![], vec![], vec![step("a")]), "a running run has no cleanup");
        refused(with("done", vec![], vec![], vec![step("a")]), "a done run has no cleanup");
        refused(json!({"input": 1, "status": "done", "armed": "m9"}), "a done run has no armed");
        refused(json!({"input": 1, "status": "cancelled", "armed": "m9"}), "a cancelled run has no armed");
        accepted(json!({"input": 1, "status": "cancelling", "armed": "m9"}));
        let asked = vec![request("r", "m1"), open("w", &["m1"], "all")];
        accepted(running(asked.clone(), vec![]));
        refused(with("done", asked.clone(), vec![], vec![]), "a done run has no open wait");
        accepted(with("cancelled", asked.clone(), vec![], vec![]));
        let cleanup = vec![request("c", "m2"), open("cw", &["m2"], "all")];
        accepted(with("cancelling", vec![], vec![], cleanup.clone()));
        refused(with("cancelled", vec![], vec![], cleanup), "a cancelled run has no open cleanup wait");
    }

    /// Scenario: Unknown or reused request.
    #[test]
    fn a_wait_names_unique_earlier_requests_of_its_journal() {
        let r = |id: &str, m: &str| request(id, m);
        for (journal, why) in [
            (vec![open("w", &["m1"], "all")], "names m1, no earlier request"),
            (vec![send("s", "m1"), open("w", &["m1"], "all")], "no earlier request"),
            (vec![open("w", &["m1"], "all"), r("r", "m1")], "no earlier request"),
            (vec![r("r", "m1"), open("w", &["m1", "m1"], "all")], "names the request m1 twice"),
            (
                vec![r("r", "m1"), r("q", "m2"), open("w", &["m1"], "all"), open("v", &["m2"], "all")],
                "follows the open",
            ),
            (
                vec![r("r", "m1"), closed("w", &["m1"], "all", "received", &["m1"]), open("v", &["m1"], "all")],
                "names the request m1 of another wait",
            ),
            (vec![json!({"kind": "wait", "id": "w", "requests": [], "mode": "all"})], "1 through 100 requests"),
        ] {
            refused(running(journal, vec![reply("m1")]), why);
        }
        let many: Vec<Value> = (0..101).map(|i| r(&format!("r{i}"), &format!("m{i}"))).collect();
        let names: Vec<String> = (0..101).map(|i| format!("m{i}")).collect();
        let mut journal = many.clone();
        journal.push(json!({"kind": "wait", "id": "w", "requests": names, "mode": "all"}));
        refused(running(journal, vec![]), "1 through 100 requests, had 101");
        let mut journal = many[..100].to_vec();
        journal.push(json!({"kind": "wait", "id": "w", "requests": names[..100], "mode": "all"}));
        accepted(running(journal, vec![]));
        refused(with("cancelling", vec![], vec![], vec![open("w", &["m1"], "all")]), "no earlier request");
        let sleep = json!({"kind": "sleep", "id": "after", "until": "2026-11-14T00:00:00Z"});
        for (successor, kind) in [
            (step("after"), "step"),
            (sleep, "sleep"),
            (send("after", "m9"), "send"),
            (request("after", "m9"), "request"),
        ] {
            let why = format!("{kind} `after` of the journal follows the open wait `w`");
            refused(running(vec![r("r", "m1"), open("w", &["m1"], "all"), successor.clone()], vec![]), &why);
            let cleanup = vec![r("c", "c1"), open("w", &["c1"], "all"), successor];
            let why = format!("{kind} `after` of the cleanup journal follows the open wait `w`");
            refused(with("cancelling", vec![], vec![], cleanup), &why);
        }
        refused(with("cancelling", vec![open("w", &["c1"], "all")], vec![], vec![r("c", "c1")]), "no earlier request");
    }

    /// Scenario: Cleanup consumes an interrupted request.
    #[test]
    fn cleanup_may_wait_only_for_main_requests_no_closed_main_wait_names() {
        let main = vec![request("r", "m1"), request("q", "m2"), open("w", &["m1", "m2"], "all")];
        for status in ["cancelling", "cancelled"] {
            accepted(with(
                status,
                main.clone(),
                vec![reply("m1")],
                vec![closed("cw", &["m1"], "all", "received", &["m1"])],
            ));
            let partial = closed("cw", &["m1", "m2"], "all", "timedOut", &["m1"]);
            accepted(with(status, main.clone(), vec![reply("m1")], vec![partial]));
        }
        accepted(with("cancelling", main.clone(), vec![], vec![open("cw", &["m2"], "all")]));
        let unwaited = vec![request("r", "m1")];
        accepted(with("cancelling", unwaited, vec![], vec![open("cw", &["m1"], "all")]));
        let done = vec![request("r", "m1"), closed("w", &["m1"], "all", "received", &["m1"])];
        refused(
            with("cancelling", done, vec![reply("m1")], vec![open("cw", &["m1"], "all")]),
            "names m1, a request a closed main wait names",
        );
        let twice = vec![open("cw", &["m1"], "all"), open("cv", &["m1"], "all")];
        refused(with("cancelling", main, vec![], twice), "follows the open wait");
    }

    /// Scenario: Invalid outcome; and Partial timeout and interrupted main wait.
    #[test]
    fn a_closed_wait_selects_kept_replies_in_request_order() {
        let asked = || vec![request("a", "m1"), request("b", "m2"), request("c", "m3")];
        let all = ["m1", "m2", "m3"];
        let replies = || vec![reply("m1"), reply("m2"), reply("m3")];
        let case = |wait: Value| {
            let mut journal = asked();
            journal.push(wait);
            running(journal, replies())
        };
        for wait in [
            closed("w", &all, "all", "received", &all),
            closed("w", &all, "first", "received", &["m2"]),
            closed("w", &all, "all", "timedOut", &["m1", "m3"]),
            closed("w", &all, "all", "timedOut", &[]),
            closed("w", &all, "first", "timedOut", &[]),
        ] {
            accepted(case(wait));
        }
        let mut selecting = open("w", &all, "all");
        selecting["received"] = json!(["m1"]);
        for (wait, why) in [
            (selecting, "is open but selects replies"),
            (closed("w", &all, "all", "received", &["m1", "m2"]), "selects 2 of 3"),
            (closed("w", &all, "first", "received", &["m1", "m2"]), "selects 2 of 3"),
            (closed("w", &all, "first", "received", &[]), "selects 0 of 3"),
            (closed("w", &all, "all", "timedOut", &all), "selects 3 of 3"),
            (closed("w", &all, "first", "timedOut", &["m1"]), "selects 1 of 3"),
            (closed("w", &all, "all", "received", &["m2", "m1", "m3"]), "out of the order"),
            (closed("w", &all, "all", "timedOut", &["m1", "m1"]), "out of the order of its requests, or twice"),
            (closed("w", &["m1"], "all", "timedOut", &["m2"]), "selects m2, which it does not name"),
        ] {
            refused(case(wait), why);
        }
        let mut journal = asked();
        journal.push(closed("w", &all, "all", "received", &all));
        refused(running(journal, vec![reply("m1"), reply("m2")]), "selects m3, whose reply the run does not keep");
        let mut no_deadline = closed("w", &all, "all", "timedOut", &[]);
        no_deadline["until"] = Value::Null;
        refused(case(no_deadline), "timed out with no deadline");
    }
}

#[cfg(test)]
mod send_tests {
    //! Step::send without a runner: what it latches, what it converts and when, and its replay.

    use std::borrow::Cow;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex};

    use super::*;
    use futures::executor::block_on;
    use serde::{Deserializer, Serializer};
    use serde_json::json;

    /// The user-code boundaries of a new send, in the order a send reaches them.
    const POINTS: [&str; 5] = ["target", "address", "message", "encode", "state"];

    /// Counts each conversion of a send's arguments, and panics or fails at a planned one.
    #[derive(Default)]
    struct Counters {
        hits: Mutex<Vec<&'static str>>,
        panic_at: Mutex<Option<&'static str>>,
        fail_at: Mutex<Option<&'static str>>,
    }

    impl Counters {
        fn hit(&self, point: &'static str) -> Result<(), String> {
            self.hits.lock().unwrap().push(point);
            if *self.panic_at.lock().unwrap() == Some(point) {
                panic!("{point} panicked");
            }
            if *self.fail_at.lock().unwrap() == Some(point) {
                return Err(format!("{point} failed"));
            }
            Ok(())
        }

        fn hits(&self) -> Vec<&'static str> {
            self.hits.lock().unwrap().clone()
        }
    }

    /// A ref of `jobs` whose serialization, and so its address, is counted.
    #[derive(Clone)]
    struct Job {
        name: String,
        counters: Arc<Counters>,
    }

    impl Serialize for Job {
        fn serialize<Z: Serializer>(&self, s: Z) -> Result<Z::Ok, Z::Error> {
            self.counters.hit("address").map_err(serde::ser::Error::custom)?;
            s.serialize_str(&self.name)
        }
    }

    impl<'de> Deserialize<'de> for Job {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            Ok(Job { name: String::deserialize(d)?, counters: Arc::default() })
        }
    }

    impl ActorRef for Job {
        const TYPE: &'static str = "jobs";
        type Message = JobMsg;
    }

    /// A field whose serialization is counted.
    struct Counted(Arc<Counters>);

    impl Serialize for Counted {
        fn serialize<Z: Serializer>(&self, s: Z) -> Result<Z::Ok, Z::Error> {
            self.0.hit("encode").map_err(serde::ser::Error::custom)?;
            s.serialize_u32(3)
        }
    }

    impl<'de> Deserialize<'de> for Counted {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            u32::deserialize(d).map(|_| Counted(Arc::default()))
        }
    }

    impl JsonSchema for Counted {
        fn schema_name() -> Cow<'static, str> {
            "Counted".into()
        }

        fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
            schemars::json_schema!({"type": "integer"})
        }
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum JobMsg {
        Start { size: Counted },
        Ask { size: Counted, reply: ReplyTo<u32> },
    }

    /// A message argument whose conversion is counted.
    struct Start(Arc<Counters>);

    impl From<Start> for JobMsg {
        fn from(start: Start) -> Self {
            start.0.hit("message").unwrap();
            JobMsg::Start { size: Counted(start.0) }
        }
    }

    /// An initial state whose serialization is counted.
    struct Initial(Arc<Counters>);

    impl Serialize for Initial {
        fn serialize<Z: Serializer>(&self, s: Z) -> Result<Z::Ok, Z::Error> {
            self.0.hit("state").map_err(serde::ser::Error::custom)?;
            json!({"size": 3}).serialize(s)
        }
    }

    /// A target argument whose conversion is counted: a child create of the job with its initial state.
    struct To<'a>(&'a Job, &'a Initial);

    impl<'a> From<To<'a>> for Target<'a, Job, Initial> {
        fn from(to: To<'a>) -> Self {
            to.0.counters.hit("target").unwrap();
            Target::child(to.0, to.1)
        }
    }

    fn execution(journal: Vec<JournalEntry>) -> Step {
        let now = time::parse_instant("2026-11-14T00:00:00Z").unwrap();
        Step::new(journal, &ActorAddress::parse("test.flow/k").unwrap(), now)
    }

    fn job(counters: &Arc<Counters>) -> Job {
        Job { name: "j1".into(), counters: counters.clone() }
    }

    fn saved_send(id: &str, reply: bool) -> JournalEntry {
        JournalEntry::Send {
            id: id.into(),
            to: ActorAddress::parse("jobs/elsewhere").unwrap(),
            message: MessageId::new("m-saved").unwrap(),
            create: None,
            reply,
        }
    }

    /// Calls the counted send `id` in `step` with `counters` and returns whether it suspended.
    fn send(step: &mut Step, id: &str, counters: &Arc<Counters>) -> bool {
        let (job, initial) = (job(counters), Initial(counters.clone()));
        block_on(step.send(id, To(&job, &initial), Start(counters.clone()))).is_err()
    }

    fn latched(step: &Step) -> String {
        match &step.latch {
            None => "none".to_owned(),
            Some(Latch::InProgress(kind, id)) => format!("in progress {kind} {id}"),
            Some(Latch::New(entry, _)) => format!("new {}", entry.id()),
            Some(Latch::Waiting(until)) => format!("waiting {}", time::format_instant(*until)),
            Some(Latch::Close(index, outcome, selected)) => {
                let selected: Vec<&str> = selected.iter().map(MessageId::as_str).collect();
                format!("close {index} {outcome:?} [{}]", selected.join(","))
            }
            Some(Latch::Parked) => "parked".to_owned(),
            Some(Latch::Failed(e)) => format!("failed {e}"),
            Some(Latch::Permanent(reason)) => format!("permanent {reason}"),
        }
    }

    /// Scenario: Create and send, as far as the step goes.
    #[test]
    fn a_new_send_converts_in_order_and_latches_its_entry_and_outbox() {
        let counters = Arc::new(Counters::default());
        let mut step = execution(vec![]);
        assert!(send(&mut step, "start", &counters), "a new send suspends");
        assert_eq!(counters.hits(), POINTS, "each conversion once, in order");
        let Some(Latch::New(JournalEntry::Send { id, to, message, create, reply }, Some(outbox))) = &step.latch else {
            panic!("{}", latched(&step));
        };
        assert_eq!((id.as_str(), to.as_str(), *create, *reply), ("start", "jobs/j1", Some(CreateKind::Child), false));
        assert_eq!((outbox.message_type.as_str(), &outbox.payload), ("Start", &json!({"size": 3})));
        assert_eq!(outbox.state, Some(json!({"size": 3})));
        let first = message.clone();
        let mut again = execution(vec![]);
        send(&mut again, "start", &Arc::default());
        let Some(Latch::New(JournalEntry::Send { message, .. }, _)) = &again.latch else { panic!() };
        assert_ne!(message, &first, "a fresh message id each time");

        let mut plain = execution(vec![]);
        let counters = Arc::new(Counters::default());
        let target = job(&counters);
        assert!(block_on(plain.send("plain", &target, Start(counters.clone()))).is_err());
        let Some(Latch::New(JournalEntry::Send { create: None, .. }, Some(outbox))) = &plain.latch else {
            panic!("{}", latched(&plain));
        };
        assert!(outbox.state.is_none(), "no create, no initial state");
        assert_eq!(counters.hits(), ["address", "message", "encode"]);
    }

    /// Scenario: Proposed arguments change after commit.
    #[test]
    fn a_replayed_send_converts_nothing_and_keeps_its_saved_destination() {
        for point in POINTS {
            let counters = Arc::new(Counters::default());
            *counters.panic_at.lock().unwrap() = Some(point);
            let mut step = execution(vec![saved_send("start", false)]);
            assert!(!send(&mut step, "start", &counters), "{point}: replayed");
            assert!(counters.hits().is_empty(), "{point}: no conversion on replay");
            assert_eq!((latched(&step), step.cursor), ("none".to_owned(), 1), "{point}");
            assert_eq!(step.journal[0], saved_send("start", false), "the saved destination stays");
        }
    }

    /// Scenario: Latch and unpolled future.
    #[test]
    fn an_earlier_latch_or_an_unpolled_send_converts_nothing() {
        let counters = Arc::new(Counters::default());
        let mut step = execution(vec![]);
        step.latch = Some(Latch::InProgress("step", "a".to_owned()));
        assert!(send(&mut step, "start", &counters));
        assert_eq!(latched(&step), "in progress step a");
        assert!(counters.hits().is_empty() && step.seen.is_empty());

        let mut step = execution(vec![]);
        let (target, initial) = (job(&counters), Initial(counters.clone()));
        drop(step.send("start", To(&target, &initial), Start(counters.clone())));
        assert!(counters.hits().is_empty(), "an unpolled send does nothing");
        assert_eq!((latched(&step), step.seen.len()), ("none".to_owned(), 0));
    }

    /// Scenario: Swallowed suspension.
    #[test]
    fn a_new_send_latch_wins_over_later_helpers() {
        let mut step = execution(vec![]);
        let _ = send(&mut step, "start", &Arc::default());
        let mut invoked = false;
        let caught = block_on(step.run("next", |_key| {
            invoked = true;
            async { Ok(()) }
        }));
        assert!(caught.is_err() && !invoked);
        let later = Arc::new(Counters::default());
        assert!(send(&mut step, "other", &later));
        assert!(block_on(step.sleep("nap", Duration::ZERO)).is_err());
        assert!(later.hits().is_empty());
        assert_eq!(latched(&step), "new start", "one outbox, one new entry");
    }

    /// Scenario: Caught conversion panic.
    #[test]
    fn a_caught_panic_at_each_conversion_leaves_the_send_in_progress() {
        for (at, point) in POINTS.iter().enumerate() {
            let counters = Arc::new(Counters::default());
            *counters.panic_at.lock().unwrap() = Some(point);
            let mut step = execution(vec![]);
            let panicked = catch_unwind(AssertUnwindSafe(|| send(&mut step, "start", &counters)));
            assert!(panicked.is_err(), "{point}");
            assert_eq!(counters.hits(), POINTS[..=at], "{point}: nothing after the panic");
            assert_eq!(latched(&step), "in progress send start", "{point}");
            let later = Arc::new(Counters::default());
            assert!(send(&mut step, "other", &later), "{point}");
            let mut invoked = false;
            let _ = block_on(step.run("next", |_key| {
                invoked = true;
                async { Ok(()) }
            }));
            assert!(later.hits().is_empty() && !invoked, "{point}: no later helper runs");
            assert_eq!(latched(&step), "in progress send start", "{point}");
        }
    }

    #[test]
    fn a_failed_conversion_or_a_self_target_is_permanent() {
        for (point, why) in [
            ("address", "the target of send `start` has no address"),
            ("encode", "the message of send `start` does not encode"),
            ("state", "the initial state of send `start` does not encode"),
        ] {
            let counters = Arc::new(Counters::default());
            *counters.fail_at.lock().unwrap() = Some(point);
            let mut step = execution(vec![]);
            assert!(send(&mut step, "start", &counters));
            assert!(latched(&step).starts_with(&format!("permanent {why}")), "{point}: {}", latched(&step));
        }

        struct Myself;
        impl Flow for Myself {
            const TYPE: &'static str = "test.flow";
            type Input = ();
            async fn run(&self, _step: &mut Step, _input: ()) -> HandlerResult {
                Ok(())
            }
        }
        let mut step = execution(vec![]);
        let me = FlowRun::<Myself>::named("k");
        assert!(block_on(step.send("loop", Target::create(&me, &FlowState::new(())), FlowMsg::Continue)).is_err());
        assert!(latched(&step).contains("send `loop` names the run itself, test.flow/k"), "{}", latched(&step));
        let mut step = execution(vec![]);
        let other = FlowRun::<Myself>::named("other");
        assert!(block_on(step.send("sibling", &other, FlowMsg::Continue)).is_err());
        assert_eq!(latched(&step), "new sibling", "another run of the same type is no self target");
    }

    /// Scenario: Replay misuse.
    #[test]
    fn ids_kinds_requests_and_capacity_are_checked_before_any_conversion() {
        let journal_full: Vec<JournalEntry> =
            (0..1000).map(|i| JournalEntry::Step { id: format!("s{i}"), result: Value::Null }).collect();
        let cases: Vec<(Vec<JournalEntry>, &str, &str)> = vec![
            (vec![], "a b", "a step id is"),
            (vec![JournalEntry::Step { id: "start".into(), result: Value::Null }], "start", "journal holds step"),
            (vec![saved_send("other", false)], "start", "calls send `start` where its journal holds send `other`"),
            (vec![saved_send("start", true)], "start", "holds request `start` at entry 1: changed flow code"),
        ];
        for (journal, id, why) in cases {
            let counters = Arc::new(Counters::default());
            let mut step = execution(journal);
            assert!(send(&mut step, id, &counters));
            assert!(latched(&step).contains(why), "{id}: {}", latched(&step));
            assert!(counters.hits().is_empty(), "{id}");
        }
        let mut step = execution(vec![saved_send("start", false)]);
        assert!(!send(&mut step, "start", &Arc::default()));
        assert!(send(&mut step, "start", &Arc::default()));
        assert!(latched(&step).contains("calls the id `start` twice"), "{}", latched(&step));

        let mut step = execution(journal_full);
        for i in 0..1000 {
            block_on(step.run(&format!("s{i}"), |_key| async { Ok(()) })).unwrap();
        }
        let counters = Arc::new(Counters::default());
        assert!(send(&mut step, "start", &counters));
        assert!(latched(&step).contains("send `start` would be entry 1001"), "{}", latched(&step));
        assert!(counters.hits().is_empty());
    }

    // Requests (flow-reply-waits): the same boundaries as a send, with the builder in place of the message's conversion.

    /// The user-code boundaries of a new request, in the order a request reaches them.
    const REQUEST_POINTS: [&str; 5] = ["target", "address", "build", "encode", "state"];

    /// Calls the counted request `id` in `step`, whose builder is counted too: its handle on replay, `None` when it
    /// suspended.
    fn request(step: &mut Step, id: &str, counters: &Arc<Counters>) -> Option<Request<u32>> {
        let (job, initial) = (job(counters), Initial(counters.clone()));
        let build = |reply| {
            counters.hit("build").unwrap();
            JobMsg::Ask { size: Counted(counters.clone()), reply }
        };
        block_on(step.request(id, To(&job, &initial), build)).ok()
    }

    /// Scenario: Typed creation and send, as far as the step goes.
    #[test]
    fn a_new_request_builds_its_route_in_order_and_latches_a_request_entry() {
        let counters = Arc::new(Counters::default());
        let mut step = execution(vec![]);
        assert!(request(&mut step, "ask", &counters).is_none(), "a new request suspends");
        assert_eq!(counters.hits(), REQUEST_POINTS, "each conversion once, in order");
        let Some(Latch::New(JournalEntry::Send { id, to, message, create, reply }, Some(outbox))) = &step.latch else {
            panic!("{}", latched(&step));
        };
        assert_eq!((id.as_str(), to.as_str(), *create, *reply), ("ask", "jobs/j1", Some(CreateKind::Child), true));
        let route = json!({"run": "test.flow/k", "request": message.as_str()});
        assert_eq!((outbox.message_type.as_str(), &outbox.payload), ("Ask", &json!({"size": 3, "reply": route})));
        assert_eq!(outbox.state, Some(json!({"size": 3})));

        let mut plain = execution(vec![]);
        let target = job(&counters);
        let asked =
            block_on(plain.request("plain", &target, |reply| JobMsg::Ask { size: Counted(Arc::default()), reply }));
        assert!(asked.is_err());
        let Some(Latch::New(JournalEntry::Send { create: None, reply: true, .. }, Some(outbox))) = &plain.latch else {
            panic!("{}", latched(&plain));
        };
        assert!(outbox.state.is_none());
    }

    /// Scenario: Replay and first latch, of a saved request.
    #[test]
    fn a_replayed_request_returns_its_saved_id_and_converts_nothing() {
        for point in REQUEST_POINTS {
            let counters = Arc::new(Counters::default());
            *counters.panic_at.lock().unwrap() = Some(point);
            let mut step = execution(vec![saved_send("ask", true)]);
            let handle = request(&mut step, "ask", &counters).expect("replayed");
            assert_eq!(handle.id().as_str(), "m-saved", "{point}");
            assert_eq!(format!("{:?}", handle.clone()), r#"Request("m-saved")"#);
            assert!(counters.hits().is_empty(), "{point}: no conversion and no builder on replay");
            assert_eq!((latched(&step), step.cursor), ("none".to_owned(), 1), "{point}");
        }
        let counters = Arc::new(Counters::default());
        let mut step = execution(vec![saved_send("ask", false)]);
        assert!(request(&mut step, "ask", &counters).is_none());
        let why = "calls request `ask` where its journal holds send `ask` at entry 1: changed flow code";
        assert!(latched(&step).contains(why), "{}", latched(&step));
        assert!(counters.hits().is_empty(), "a plain send is no request");
    }

    /// Scenario: Replay and first latch, of an earlier latch and an unpolled request.
    #[test]
    fn an_earlier_latch_or_an_unpolled_request_builds_nothing() {
        let counters = Arc::new(Counters::default());
        let mut step = execution(vec![]);
        step.latch = Some(Latch::InProgress("step", "a".to_owned()));
        assert!(request(&mut step, "ask", &counters).is_none());
        assert_eq!(latched(&step), "in progress step a");
        assert!(counters.hits().is_empty() && step.seen.is_empty());

        let mut step = execution(vec![]);
        let (target, initial) = (job(&counters), Initial(counters.clone()));
        let build = |reply| {
            counters.hit("build").unwrap();
            JobMsg::Ask { size: Counted(counters.clone()), reply }
        };
        drop(step.request("ask", To(&target, &initial), build));
        assert!(counters.hits().is_empty(), "an unpolled request does nothing");
        assert_eq!((latched(&step), step.seen.len()), ("none".to_owned(), 0));

        let mut step = execution(vec![]);
        assert!(request(&mut step, "ask", &Arc::default()).is_none());
        let later = Arc::new(Counters::default());
        assert!(request(&mut step, "other", &later).is_none());
        assert!(send(&mut step, "more", &later));
        assert!(later.hits().is_empty());
        assert_eq!(latched(&step), "new ask", "the first latch wins");
    }

    /// Scenario: Request misuse and panic, of a caught panic.
    #[test]
    fn a_caught_panic_at_each_point_of_a_request_leaves_it_in_progress() {
        for (at, point) in REQUEST_POINTS.iter().enumerate() {
            let counters = Arc::new(Counters::default());
            *counters.panic_at.lock().unwrap() = Some(point);
            let mut step = execution(vec![]);
            let panicked = catch_unwind(AssertUnwindSafe(|| request(&mut step, "ask", &counters)));
            assert!(panicked.is_err(), "{point}");
            assert_eq!(counters.hits(), REQUEST_POINTS[..=at], "{point}: nothing after the panic");
            assert_eq!(latched(&step), "in progress request ask", "{point}");
            let later = Arc::new(Counters::default());
            assert!(request(&mut step, "other", &later).is_none(), "{point}");
            assert!(later.hits().is_empty(), "{point}: no later helper runs");
            assert_eq!(latched(&step), "in progress request ask", "{point}");
        }
    }

    /// Scenario: Request misuse and panic: failed conversions, self targets, ids and capacity.
    #[test]
    fn request_misuse_is_permanent_and_checked_before_any_builder() {
        for (point, why) in [
            ("address", "the target of request `ask` has no address"),
            ("encode", "the message of request `ask` does not encode"),
            ("state", "the initial state of request `ask` does not encode"),
        ] {
            let counters = Arc::new(Counters::default());
            *counters.fail_at.lock().unwrap() = Some(point);
            let mut step = execution(vec![]);
            assert!(request(&mut step, "ask", &counters).is_none());
            assert!(latched(&step).starts_with(&format!("permanent {why}")), "{point}: {}", latched(&step));
        }

        struct Myself;
        impl Flow for Myself {
            const TYPE: &'static str = "test.flow";
            type Input = ();
            async fn run(&self, _step: &mut Step, _input: ()) -> HandlerResult {
                Ok(())
            }
        }
        let mut step = execution(vec![]);
        let me = FlowRun::<Myself>::named("k");
        let mut built = false;
        let asked = block_on(step.request("loop", &me, |_: ReplyTo<()>| {
            built = true;
            FlowMsg::Continue
        }));
        assert!(asked.is_err() && !built, "a self target is refused before the builder");
        assert!(latched(&step).contains("request `loop` names the run itself, test.flow/k"), "{}", latched(&step));

        let cases: Vec<(Vec<JournalEntry>, &str, &str)> = vec![
            (vec![], "a b", "a step id is"),
            (vec![JournalEntry::Step { id: "ask".into(), result: Value::Null }], "ask", "journal holds step"),
            (vec![saved_send("other", true)], "ask", "calls request `ask` where its journal holds request `other`"),
        ];
        for (journal, id, why) in cases {
            let counters = Arc::new(Counters::default());
            let mut step = execution(journal);
            assert!(request(&mut step, id, &counters).is_none());
            assert!(latched(&step).contains(why), "{id}: {}", latched(&step));
            assert!(counters.hits().is_empty(), "{id}");
        }
        let mut step = execution(vec![saved_send("ask", true)]);
        assert!(request(&mut step, "ask", &Arc::default()).is_some());
        assert!(request(&mut step, "ask", &Arc::default()).is_none());
        assert!(latched(&step).contains("calls the id `ask` twice"), "{}", latched(&step));

        let mut journal: Vec<JournalEntry> =
            (0..999).map(|i| JournalEntry::Step { id: format!("s{i}"), result: Value::Null }).collect();
        journal.push(saved_send("last", true));
        let mut step = execution(journal);
        for i in 0..999 {
            block_on(step.run(&format!("s{i}"), |_key| async { Ok(()) })).unwrap();
        }
        assert!(request(&mut step, "last", &Arc::default()).is_some(), "entry 1000 replays");
        let counters = Arc::new(Counters::default());
        assert!(request(&mut step, "ask", &counters).is_none());
        assert!(latched(&step).contains("request `ask` would be entry 1001"), "{}", latched(&step));
        assert!(counters.hits().is_empty());
    }

    /// A reply value whose serialization fails.
    struct Unencodable;

    impl Serialize for Unencodable {
        fn serialize<Z: Serializer>(&self, _: Z) -> Result<Z::Ok, Z::Error> {
            Err(serde::ser::Error::custom("no reply"))
        }
    }

    fn route<T>() -> ReplyTo<T> {
        ReplyTo::new(ActorAddress::parse("test.flow/k").unwrap(), MessageId::new("m1").unwrap())
    }

    fn nested(depth: usize) -> Value {
        (0..depth).fold(Value::Null, |inner, _| Value::Array(vec![inner]))
    }

    /// Scenario: Value boundaries, of the helper's checks before anything is buffered.
    #[test]
    fn a_reply_is_encoded_and_checked_before_it_is_sent() {
        let (run, encoded) = route::<Value>().encode(&Value::Null).unwrap();
        assert_eq!((run.as_str(), encoded.message_type.as_str()), ("test.flow/k", "Reply"));
        assert_eq!(encoded.payload, json!({"request": "m1", "value": null}));
        assert!(route::<Value>().encode(&nested(MAX_RESULT_DEPTH)).is_ok());
        for (value, why) in [
            (nested(MAX_RESULT_DEPTH + 1), "more than 61"),
            (nested(MAX_RESULT_DEPTH + 2), "more than 61"),
            (json!("a\0b"), "NUL"),
            (json!({"k\0": 1}), "NUL"),
        ] {
            match route::<Value>().encode(&value) {
                Err(TurnError::Invalid(reason)) => assert!(reason.contains(why), "{reason}"),
                other => panic!("{other:?} for {why}"),
            }
        }
        match route::<Unencodable>().encode(&Unencodable) {
            Err(TurnError::Invalid(reason)) => assert!(reason.contains("does not encode: no reply"), "{reason}"),
            other => panic!("{other:?}"),
        }
    }

    /// Scenario: Stable schema, of the route's traits: none needs anything of `T`.
    #[test]
    fn a_route_is_the_same_whatever_its_reply_type() {
        struct Opaque;
        let (a, b) = (route::<Opaque>(), route::<Opaque>().clone());
        assert_eq!(a, b);
        assert_eq!(format!("{a:?}"), r#"ReplyTo { run: "test.flow/k", request: "m1" }"#);
        assert_eq!(serde_json::to_value(&a).unwrap(), json!({"run": "test.flow/k", "request": "m1"}));
        let back: ReplyTo<Opaque> =
            serde_json::from_value(json!({"run": "test.flow/k", "request": "m1", "other": true})).unwrap();
        assert_eq!(back, a, "unknown fields are ignored");
        assert!(serde_json::from_value::<ReplyTo<Opaque>>(json!({"run": "test.flow/k"})).is_err());
        assert!(serde_json::from_value::<ReplyTo<Opaque>>(json!({"request": "m1"})).is_err());
        let schema = |s: schemars::Schema| serde_json::to_value(s).unwrap();
        assert_eq!(
            schema(schemars::schema_for!(ReplyTo<String>)),
            schema(schemars::schema_for!(ReplyTo<u64>)),
            "one schema for every T"
        );
    }
}

#[cfg(test)]
mod sleep_tests {
    use super::*;
    use futures::executor::block_on;

    const SECOND: Duration = Duration::from_secs(1);

    fn at(text: &str) -> Timestamp {
        time::parse_instant(text).unwrap()
    }

    fn execution(journal: Vec<JournalEntry>, now: Timestamp) -> Step {
        Step::new(journal, &ActorAddress::parse("test.flow/k").unwrap(), now)
    }

    fn saved_sleep(id: &str, until: &str) -> JournalEntry {
        JournalEntry::Sleep { id: id.to_owned(), until: at(until) }
    }

    fn saved_step(id: &str) -> JournalEntry {
        JournalEntry::Step { id: id.to_owned(), result: Value::Null }
    }

    /// What the execution latched, as text.
    fn latched(step: &Step) -> String {
        match &step.latch {
            None => "none".to_owned(),
            Some(Latch::InProgress(kind, id)) => format!("in progress {kind} {id}"),
            Some(Latch::New(entry, _)) => format!("new {}", serde_json::to_string(entry).unwrap()),
            Some(Latch::Waiting(until)) => format!("waiting {}", time::format_instant(*until)),
            Some(Latch::Close(index, outcome, selected)) => {
                let selected: Vec<&str> = selected.iter().map(MessageId::as_str).collect();
                format!("close {index} {outcome:?} [{}]", selected.join(","))
            }
            Some(Latch::Parked) => "parked".to_owned(),
            Some(Latch::Failed(e)) => format!("failed {e}"),
            Some(Latch::Permanent(reason)) => format!("permanent {reason}"),
        }
    }

    /// Calls a sleep in `step` and returns whether it suspended.
    fn sleep(step: &mut Step, id: &str, duration: Duration) -> bool {
        block_on(step.sleep(id, duration)).is_err()
    }

    #[test]
    fn a_new_sleep_latches_its_wake_time_rounded_up_to_microseconds() {
        let now = at("2026-11-14T00:00:00Z");
        for (duration, until) in [
            (Duration::ZERO, "2026-11-14T00:00:00Z"),
            (Duration::from_nanos(1), "2026-11-14T00:00:00.000001Z"),
            (Duration::from_nanos(1_001), "2026-11-14T00:00:00.000002Z"),
            (Duration::from_micros(1), "2026-11-14T00:00:00.000001Z"),
            (Duration::from_secs(60), "2026-11-14T00:01:00Z"),
        ] {
            let mut step = execution(vec![], now);
            assert!(sleep(&mut step, "w", duration), "{duration:?}: a new sleep suspends, zero included");
            let expected = format!(r#"new {{"kind":"sleep","id":"w","until":"{until}"}}"#);
            assert_eq!(latched(&step), expected, "{duration:?}");
        }
    }

    #[test]
    fn a_new_sleep_is_bounded_by_max_delay_and_max_instant() {
        let now = at("2026-11-14T00:00:00Z");
        let mut step = execution(vec![], now);
        assert!(sleep(&mut step, "w", MAX_DELAY));
        assert!(latched(&step).starts_with("new "), "MAX_DELAY itself is allowed: {}", latched(&step));
        let mut step = execution(vec![], now);
        sleep(&mut step, "w", MAX_DELAY + Duration::from_nanos(1));
        assert!(latched(&step).contains("longer than the most a sleep may last"), "{}", latched(&step));

        let late = time::sub(MAX_INSTANT, SECOND);
        let mut step = execution(vec![], late);
        sleep(&mut step, "w", SECOND);
        assert_eq!(latched(&step), r#"new {"kind":"sleep","id":"w","until":"9999-12-31T23:59:59.999999Z"}"#);
        for duration in [SECOND + Duration::from_nanos(1), Duration::from_secs(2), MAX_DELAY] {
            let mut step = execution(vec![], late);
            sleep(&mut step, "w", duration);
            assert!(latched(&step).contains("after 9999-12-31T23:59:59.999999Z"), "{duration:?}: {}", latched(&step));
        }
    }

    #[test]
    fn sleep_ids_share_the_step_namespace_and_the_entry_limit() {
        let now = at("2026-11-14T00:00:00Z");
        let mut step = execution(vec![], now);
        sleep(&mut step, "a b", SECOND);
        assert!(latched(&step).contains("a step id is"), "{}", latched(&step));

        let mut step = execution(vec![saved_step("a")], now);
        block_on(step.run("a", |_key| async { Ok(()) })).unwrap();
        sleep(&mut step, "a", SECOND);
        assert!(latched(&step).contains("calls the id `a` twice"), "{}", latched(&step));

        let full: Vec<JournalEntry> = (0..1000).map(|i| saved_step(&format!("s{i}"))).collect();
        let mut step = execution(full, now);
        for i in 0..1000 {
            block_on(step.run(&format!("s{i}"), |_key| async { Ok(()) })).unwrap();
        }
        sleep(&mut step, "w", SECOND);
        assert!(latched(&step).contains("sleep `w` would be entry 1001"), "{}", latched(&step));
    }

    #[test]
    fn a_replayed_sleep_waits_for_its_saved_wake_time_whatever_the_duration() {
        let until = "2026-11-14T00:01:00Z";
        for duration in [Duration::ZERO, SECOND, MAX_DELAY + SECOND] {
            let mut step = execution(vec![saved_sleep("w", until)], at("2026-11-14T00:00:59.999999Z"));
            assert!(sleep(&mut step, "w", duration), "{duration:?}: not yet due");
            assert_eq!(latched(&step), format!("waiting {until}"), "{duration:?}");

            for now in [until, "2026-11-15T00:00:00Z"] {
                let mut step = execution(vec![saved_sleep("w", until)], at(now));
                assert!(!sleep(&mut step, "w", duration), "{duration:?} at {now}: elapsed");
                assert_eq!((latched(&step), step.cursor), ("none".to_owned(), 1), "{duration:?} at {now}");
            }
        }
    }

    #[test]
    fn a_sleep_and_a_step_do_not_replay_each_other() {
        let now = at("2026-11-14T00:00:00Z");
        let mut step = execution(vec![saved_step("w")], now);
        sleep(&mut step, "w", SECOND);
        assert!(latched(&step).contains("calls sleep `w` where its journal holds step `w`"), "{}", latched(&step));

        let mut step = execution(vec![saved_sleep("v", "2026-11-13T00:00:00Z")], now);
        sleep(&mut step, "w", SECOND);
        assert!(latched(&step).contains("calls sleep `w` where its journal holds sleep `v`"), "{}", latched(&step));

        let mut step = execution(vec![saved_sleep("w", "2026-11-13T00:00:00Z")], now);
        let mut invoked = false;
        let _ = block_on(step.run("w", |_key| {
            invoked = true;
            async { Ok(()) }
        }));
        assert!(!invoked);
        assert!(latched(&step).contains("calls step `w` where its journal holds sleep `w`"), "{}", latched(&step));
    }

    #[test]
    fn an_earlier_latch_wins_over_a_sleep_and_a_sleep_latch_over_a_later_step() {
        let now = at("2026-11-14T00:00:00Z");
        let mut step = execution(vec![], now);
        step.latch = Some(Latch::InProgress("step", "a".to_owned()));
        assert!(sleep(&mut step, "w", SECOND));
        assert_eq!(latched(&step), "in progress step a", "an abandoned step stays the outcome");
        assert!(step.seen.is_empty() && step.cursor == 0, "the sleep did nothing");

        for journal in [vec![], vec![saved_sleep("w", "2026-11-15T00:00:00Z")]] {
            let mut step = execution(journal, now);
            assert!(sleep(&mut step, "w", SECOND));
            let before = latched(&step);
            let mut invoked = false;
            let caught = block_on(step.run("next", |_key| {
                invoked = true;
                async { Ok(()) }
            }));
            assert!(caught.is_err() && !invoked, "{before}: a later step invokes nothing");
            assert!(sleep(&mut step, "again", SECOND));
            assert_eq!(latched(&step), before, "the sleep's latch stays");
        }

        let mut step = execution(vec![], now);
        drop(step.sleep("w", SECOND));
        assert_eq!((latched(&step), step.seen.len()), ("none".to_owned(), 0), "an unpolled sleep does nothing");
    }
}

#[cfg(test)]
mod wait_tests {
    //! Step::wait without a runner: what it latches for new, open and closed waits, its checks, and its decoding.

    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;
    use futures::executor::block_on;
    use serde::Deserializer;
    use serde_json::json;

    fn id(text: &str) -> MessageId {
        MessageId::new(text).unwrap()
    }

    fn execution(journal: Vec<JournalEntry>, replies: &[(&str, Value)]) -> Step {
        let now = time::parse_instant("2026-11-14T00:00:00Z").unwrap();
        let mut step = Step::new(journal, &ActorAddress::parse("test.flow/k").unwrap(), now);
        step.replies = replies.iter().map(|(request, value)| (id(request), value.clone())).collect();
        step
    }

    fn request(name: &str, message: &str) -> JournalEntry {
        JournalEntry::Send {
            id: name.into(),
            to: ActorAddress::parse("jobs/j1").unwrap(),
            message: id(message),
            create: None,
            reply: true,
        }
    }

    fn wait(name: &str, requests: &[&str], outcome: Option<WaitOutcome>) -> JournalEntry {
        let requests: Vec<MessageId> = requests.iter().map(|r| id(r)).collect();
        let received = if outcome.is_some() { requests.clone() } else { Vec::new() };
        JournalEntry::Wait { id: name.into(), requests, mode: WaitMode::All, until: None, outcome, received }
    }

    fn latched(step: &Step) -> String {
        match &step.latch {
            None => "none".to_owned(),
            Some(Latch::InProgress(kind, id)) => format!("in progress {kind} {id}"),
            Some(Latch::New(entry, outbox)) => {
                format!("new {} {}", serde_json::to_string(entry).unwrap(), outbox.is_some())
            }
            Some(Latch::Waiting(until)) => format!("waiting {}", time::format_instant(*until)),
            Some(Latch::Close(index, outcome, selected)) => {
                let selected: Vec<&str> = selected.iter().map(MessageId::as_str).collect();
                format!("close {index} {outcome:?} [{}]", selected.join(","))
            }
            Some(Latch::Parked) => "parked".to_owned(),
            Some(Latch::Failed(e)) => format!("failed {e}"),
            Some(Latch::Permanent(reason)) => format!("permanent {reason}"),
        }
    }

    /// Replays the request `name` and then waits `wait` for it: the wait's result.
    fn replay_and_wait<T: DeserializeOwned + Send>(step: &mut Step, name: &str, wait: &str) -> Result<T, HandlerError> {
        let handle = block_on(step.request(name, &job(), |_: ReplyTo<T>| Never::Go)).expect("the request replays");
        block_on(step.wait(wait, &handle))
    }

    /// `jobs/j1`.
    #[derive(Clone, Serialize, Deserialize)]
    #[serde(transparent)]
    struct Job(String);

    impl ActorRef for Job {
        const TYPE: &'static str = "jobs";
        type Message = Never;
    }

    fn job() -> Job {
        Job("j1".into())
    }

    /// A message that replay never builds.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Never {
        Go,
    }

    /// Scenario: Wait before reply; and Reply before wait, as far as the step goes.
    #[test]
    fn a_new_wait_latches_an_open_entry_or_a_closed_one_when_the_reply_is_kept() {
        let mut step = execution(vec![request("ask", "m1")], &[]);
        assert!(replay_and_wait::<Value>(&mut step, "ask", "done").is_err(), "a new wait suspends");
        assert_eq!(
            latched(&step),
            r#"new {"kind":"wait","id":"done","requests":["m1"],"mode":"all","until":null,"outcome":null,"received":[]} false"#
        );

        let mut step = execution(vec![request("ask", "m1")], &[("m1", json!(7))]);
        assert!(replay_and_wait::<Value>(&mut step, "ask", "done").is_err(), "a closed new wait suspends too");
        let closed = r#"{"kind":"wait","id":"done","requests":["m1"],"mode":"all","until":null,"outcome":"received","received":["m1"]}"#;
        assert_eq!(latched(&step), format!("new {closed} false"));
    }

    /// Scenario: Supplied open wait with a reply; Public Continue while parked; and Reply before wait, on replay.
    #[test]
    fn a_saved_wait_closes_parks_or_returns_its_committed_value() {
        let journal = vec![request("ask", "m1"), wait("done", &["m1"], None)];
        let mut step = execution(journal.clone(), &[]);
        assert!(replay_and_wait::<Value>(&mut step, "ask", "done").is_err());
        assert_eq!((latched(&step), step.cursor), ("parked".to_owned(), 2), "open, no reply: parked");

        let mut step = execution(journal, &[("m1", json!("v"))]);
        assert!(replay_and_wait::<Value>(&mut step, "ask", "done").is_err(), "the value waits for the closure");
        assert_eq!(latched(&step), "close 1 Received [m1]");

        let journal = vec![request("ask", "m1"), wait("done", &["m1"], Some(WaitOutcome::Received))];
        let mut step = execution(journal.clone(), &[("m1", json!("v"))]);
        assert_eq!(replay_and_wait::<String>(&mut step, "ask", "done").unwrap(), "v");
        assert_eq!(latched(&step), "none", "the decoding marker is cleared");
        assert!(block_on(step.run("next", |_key| async { Ok(()) })).is_err(), "later helpers go on");
        assert!(latched(&step).starts_with("new "), "{}", latched(&step));

        let mut step = execution(journal, &[("m1", Value::Null)]);
        assert_eq!(replay_and_wait::<Option<u32>>(&mut step, "ask", "done").unwrap(), None, "null is data");
    }

    /// Scenario: Typed decode failure.
    #[test]
    fn a_saved_value_that_does_not_decode_or_whose_decoder_panics_fails_for_good() {
        let journal = vec![request("ask", "m1"), wait("done", &["m1"], Some(WaitOutcome::Received))];
        let mut step = execution(journal.clone(), &[("m1", json!("seven"))]);
        assert!(replay_and_wait::<u32>(&mut step, "ask", "done").is_err());
        let why = "the saved reply of wait `done` does not decode as the type asked for";
        assert!(latched(&step).contains(why), "{}", latched(&step));
        assert!(latched(&step).contains("fix the code and send a public Continue"), "{}", latched(&step));

        /// A reply whose decoder panics.
        struct Exploding;
        impl<'de> Deserialize<'de> for Exploding {
            fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                panic!("the decoder panicked")
            }
        }
        let mut step = execution(journal, &[("m1", json!(1))]);
        let handle = block_on(step.request("ask", &job(), |_: ReplyTo<Exploding>| Never::Go)).unwrap();
        let panicked = catch_unwind(AssertUnwindSafe(|| block_on(step.wait("done", &handle))));
        assert!(panicked.is_err());
        assert_eq!(latched(&step), "in progress wait done");
        let mut invoked = false;
        let _ = block_on(step.run("next", |_key| {
            invoked = true;
            async { Ok(()) }
        }));
        assert!(!invoked, "the marker blocks later helpers");
        assert_eq!(latched(&step), "in progress wait done");
    }

    /// Scenario: Wait misuse.
    #[test]
    fn a_wait_names_its_own_earlier_unused_request_and_its_saved_shape() {
        let saved = |entry: JournalEntry| vec![request("ask", "m1"), request("other", "m2"), entry];
        let deadline = JournalEntry::Wait {
            id: "done".into(),
            requests: vec![id("m1")],
            mode: WaitMode::All,
            until: Some(time::parse_instant("2026-11-15T00:00:00Z").unwrap()),
            outcome: None,
            received: vec![],
        };
        let mut first = wait("done", &["m1"], None);
        if let JournalEntry::Wait { mode, .. } = &mut first {
            *mode = WaitMode::First;
        }
        for (journal, why) in [
            (saved(wait("done", &["m1", "m2"], None)), "saved for all of the requests m1, m2, not for the request m1"),
            (saved(first), "wait `done` is saved for the first of the requests m1, not for the request m1"),
            (saved(deadline), "wait `done` is saved with a deadline, which Step::wait cannot replay"),
            (saved(wait("done", &["m2"], None)), "wait `done` is saved for the request m2, not for the request m1"),
            (saved(JournalEntry::Step { id: "done".into(), result: Value::Null }), "where its journal holds step"),
            (saved(wait("later", &["m1"], None)), "calls wait `done` where its journal holds wait `later`"),
        ] {
            let mut step = execution(journal, &[]);
            let handle = block_on(step.request("ask", &job(), |_: ReplyTo<Value>| Never::Go)).unwrap();
            block_on(step.request("other", &job(), |_: ReplyTo<Value>| Never::Go)).unwrap();
            assert!(block_on(step.wait("done", &handle)).is_err());
            assert!(latched(&step).contains(why), "{why}: {}", latched(&step));
        }

        // A new wait for an unknown request, or for one another wait names.
        let mut step = execution(vec![request("ask", "m1")], &[]);
        block_on(step.request("ask", &job(), |_: ReplyTo<Value>| Never::Go)).unwrap();
        assert!(block_on(step.wait("done", &Request::<Value>::new(id("m9")))).is_err());
        assert!(
            latched(&step).contains("wait `done` names m9, no earlier request of the journal"),
            "{}",
            latched(&step)
        );
        let journal = vec![request("ask", "m1"), wait("first", &["m1"], Some(WaitOutcome::Received))];
        let mut step = execution(journal, &[("m1", json!(1))]);
        let handle: Request<Value> = block_on(step.request("ask", &job(), |_| Never::Go)).unwrap();
        block_on(step.wait("first", &handle)).unwrap();
        assert!(block_on(step.wait("again", &handle)).is_err());
        assert!(latched(&step).contains("names the request m1 of the wait `first`"), "{}", latched(&step));

        // A plain send is no request; a repeated id, and entry 1001.
        let mut plain = request("ask", "m1");
        if let JournalEntry::Send { reply, .. } = &mut plain {
            *reply = false;
        }
        let mut step = execution(vec![plain], &[]);
        assert!(block_on(step.send("ask", &job(), Never::Go)).is_ok());
        assert!(block_on(step.wait("done", &Request::<Value>::new(id("m1")))).is_err());
        assert!(latched(&step).contains("no earlier request"), "{}", latched(&step));
        let mut step = execution(vec![request("ask", "m1")], &[]);
        let handle: Request<Value> = block_on(step.request("ask", &job(), |_| Never::Go)).unwrap();
        assert!(block_on(step.wait("ask", &handle)).is_err());
        assert!(latched(&step).contains("calls the id `ask` twice"), "{}", latched(&step));

        let mut journal: Vec<JournalEntry> =
            (0..999).map(|i| JournalEntry::Step { id: format!("s{i}"), result: Value::Null }).collect();
        journal.push(request("ask", "m1"));
        let mut step = execution(journal, &[]);
        for i in 0..999 {
            block_on(step.run(&format!("s{i}"), |_key| async { Ok(()) })).unwrap();
        }
        let handle: Request<Value> = block_on(step.request("ask", &job(), |_| Never::Go)).unwrap();
        assert!(block_on(step.wait("done", &handle)).is_err());
        assert!(latched(&step).contains("wait `done` would be entry 1001"), "{}", latched(&step));

        let mut journal: Vec<JournalEntry> =
            (0..998).map(|i| JournalEntry::Step { id: format!("s{i}"), result: Value::Null }).collect();
        journal.push(request("ask", "m1"));
        let mut step = execution(journal, &[]);
        for i in 0..998 {
            block_on(step.run(&format!("s{i}"), |_key| async { Ok(()) })).unwrap();
        }
        let handle: Request<Value> = block_on(step.request("ask", &job(), |_| Never::Go)).unwrap();
        assert!(block_on(step.wait("done", &handle)).is_err());
        assert!(latched(&step).starts_with(r#"new {"kind":"wait""#), "entry 1000: {}", latched(&step));
    }

    /// Scenario: Swallowed suspension; first latch, unpolled waits and earlier latches.
    #[test]
    fn the_first_latch_wins_over_and_against_a_wait() {
        let mut step = execution(vec![request("ask", "m1")], &[]);
        let handle: Request<Value> = block_on(step.request("ask", &job(), |_| Never::Go)).unwrap();
        drop(step.wait("done", &handle));
        assert_eq!((latched(&step), step.cursor), ("none".to_owned(), 1), "an unpolled wait does nothing");
        let _ = block_on(step.wait("done", &handle));
        let open = latched(&step);
        assert!(open.starts_with("new "), "{open}");
        let mut invoked = false;
        let _ = block_on(step.run("next", |_key| {
            invoked = true;
            async { Ok(()) }
        }));
        assert!(block_on(step.wait("other", &handle)).is_err());
        assert!(!invoked && latched(&step) == open, "the swallowed wait's latch stays");

        let mut step = execution(vec![request("ask", "m1"), wait("done", &["m1"], None)], &[]);
        step.latch = Some(Latch::InProgress("step", "a".to_owned()));
        assert!(block_on(step.wait("done", &Request::<Value>::new(id("m1")))).is_err());
        assert_eq!((latched(&step), step.seen.len()), ("in progress step a".to_owned(), 0), "an earlier latch wins");
    }
}

#[cfg(test)]
mod wait_timeout_tests {
    //! Step::wait_timeout without a runner: the deadline of a new wait and its limits, checked before an early reply;
    //! replay by the saved deadline; the paths of an open, received and timed-out wait; misuse; latches and decoding.

    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;
    use futures::executor::block_on;
    use serde::Deserializer;
    use serde_json::json;

    const SECOND: Duration = Duration::from_secs(1);
    const NOW: &str = "2026-11-14T00:00:00Z";
    const DEADLINE: &str = "2026-11-14T00:01:00Z";

    fn at(text: &str) -> Timestamp {
        time::parse_instant(text).unwrap()
    }

    fn id(text: &str) -> MessageId {
        MessageId::new(text).unwrap()
    }

    fn execution(journal: Vec<JournalEntry>, replies: &[(&str, Value)], now: Timestamp) -> Step {
        let mut step = Step::new(journal, &ActorAddress::parse("test.flow/k").unwrap(), now);
        step.replies = replies.iter().map(|(request, value)| (id(request), value.clone())).collect();
        step
    }

    fn request(name: &str, message: &str) -> JournalEntry {
        JournalEntry::Send {
            id: name.into(),
            to: ActorAddress::parse("jobs/j1").unwrap(),
            message: id(message),
            create: None,
            reply: true,
        }
    }

    /// A single wait `name` for `m1` with the deadline `until`, if any, and `outcome`.
    fn wait(name: &str, until: Option<&str>, outcome: Option<WaitOutcome>) -> JournalEntry {
        let received = if outcome == Some(WaitOutcome::Received) { vec![id("m1")] } else { Vec::new() };
        JournalEntry::Wait {
            id: name.into(),
            requests: vec![id("m1")],
            mode: WaitMode::All,
            until: until.map(at),
            outcome,
            received,
        }
    }

    fn latched(step: &Step) -> String {
        match &step.latch {
            None => "none".to_owned(),
            Some(Latch::InProgress(kind, id)) => format!("in progress {kind} {id}"),
            Some(Latch::New(entry, outbox)) => {
                format!("new {} {}", serde_json::to_string(entry).unwrap(), outbox.is_some())
            }
            Some(Latch::Waiting(until)) => format!("waiting {}", time::format_instant(*until)),
            Some(Latch::Close(index, outcome, selected)) => {
                let selected: Vec<&str> = selected.iter().map(MessageId::as_str).collect();
                format!("close {index} {outcome:?} [{}]", selected.join(","))
            }
            Some(Latch::Parked) => "parked".to_owned(),
            Some(Latch::Failed(e)) => format!("failed {e}"),
            Some(Latch::Permanent(reason)) => format!("permanent {reason}"),
        }
    }

    /// `jobs/j1`.
    #[derive(Clone, Serialize, Deserialize)]
    #[serde(transparent)]
    struct Job(String);

    impl ActorRef for Job {
        const TYPE: &'static str = "jobs";
        type Message = Never;
    }

    fn job() -> Job {
        Job("j1".into())
    }

    /// A message that replay never builds.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Never {
        Go,
    }

    /// Replays the request `ask` and returns its handle.
    fn ask<T>(step: &mut Step) -> Request<T> {
        block_on(step.request("ask", &job(), |_: ReplyTo<T>| Never::Go)).expect("the request replays")
    }

    /// Replays the request `ask` and waits `done` for it with `timeout`: the wait's result.
    fn timed<T: DeserializeOwned + Send>(step: &mut Step, timeout: Duration) -> Result<Waited<T>, HandlerError> {
        let handle = ask::<T>(step);
        block_on(step.wait_timeout("done", &handle, timeout))
    }

    /// The new wait entry `done` for `m1` with the deadline `until`, `outcome` and `received`, as latched.
    fn new_entry(until: &str, outcome: &str, received: &str) -> String {
        format!(
            r#"new {{"kind":"wait","id":"done","requests":["m1"],"mode":"all","until":"{until}","outcome":{outcome},"received":[{received}]}} false"#
        )
    }

    /// Scenarios: New timed wait; Early reply and zero duration; Time limits, of 1ns.
    #[test]
    fn a_new_timed_wait_saves_its_deadline_from_the_claim_open_or_received() {
        for (timeout, until) in [
            (Duration::ZERO, NOW),
            (Duration::from_nanos(1), "2026-11-14T00:00:00.000001Z"),
            (Duration::from_nanos(1_001), "2026-11-14T00:00:00.000002Z"),
            (SECOND * 60, DEADLINE),
        ] {
            let mut step = execution(vec![request("ask", "m1")], &[], at(NOW));
            assert!(timed::<Value>(&mut step, timeout).is_err(), "{timeout:?}: a new wait suspends");
            assert_eq!(latched(&step), new_entry(until, "null", ""), "{timeout:?}: open, never timed out at once");

            let mut step = execution(vec![request("ask", "m1")], &[("m1", json!(7))], at(NOW));
            assert!(timed::<Value>(&mut step, timeout).is_err(), "{timeout:?}: a closed new wait suspends too");
            assert_eq!(latched(&step), new_entry(until, r#""received""#, r#""m1""#), "{timeout:?}");
        }
    }

    /// Scenario: Time limits. MAX_DELAY fits, one nanosecond more does not; at MAX_INSTANT - 1s, 1s fits and 1s + 1ns
    /// does not; an invalid timeout fails even when the reply is already kept.
    #[test]
    fn a_new_deadline_is_bounded_by_max_delay_and_max_instant_before_an_early_reply_is_used() {
        for replies in [vec![], vec![("m1", json!(1))]] {
            let mut step = execution(vec![request("ask", "m1")], &replies, at(NOW));
            assert!(timed::<Value>(&mut step, MAX_DELAY).is_err());
            assert!(latched(&step).starts_with(r#"new {"kind":"wait""#), "MAX_DELAY fits: {}", latched(&step));

            let mut step = execution(vec![request("ask", "m1")], &replies, at(NOW));
            assert!(timed::<Value>(&mut step, MAX_DELAY + Duration::from_nanos(1)).is_err());
            assert!(latched(&step).starts_with("permanent wait `done` lasts"), "{replies:?}: {}", latched(&step));
            assert!(latched(&step).contains("longer than the most a wait may last"), "{}", latched(&step));

            let late = time::sub(MAX_INSTANT, SECOND);
            let mut step = execution(vec![request("ask", "m1")], &replies, late);
            assert!(timed::<Value>(&mut step, SECOND).is_err());
            assert!(latched(&step).contains(r#""until":"9999-12-31T23:59:59.999999Z""#), "{}", latched(&step));
            for timeout in [SECOND + Duration::from_nanos(1), MAX_DELAY] {
                let mut step = execution(vec![request("ask", "m1")], &replies, late);
                assert!(timed::<Value>(&mut step, timeout).is_err());
                let why = "permanent wait `done` would end at";
                assert!(latched(&step).starts_with(why), "{timeout:?}: {}", latched(&step));
                assert!(latched(&step).contains("after 9999-12-31T23:59:59.999999Z"), "{}", latched(&step));
            }
        }
    }

    /// Scenarios: Replay matching, with another and an invalid duration; Early execution; Timer execution; and
    /// Supplied reply past the deadline.
    #[test]
    fn a_saved_open_wait_closes_times_out_or_waits_by_its_saved_deadline_whatever_the_timeout() {
        let open = vec![request("ask", "m1"), wait("done", Some(DEADLINE), None)];
        for timeout in [Duration::ZERO, SECOND * 3600, MAX_DELAY + SECOND] {
            let mut step = execution(open.clone(), &[], at("2026-11-14T00:00:59.999999Z"));
            assert!(timed::<Value>(&mut step, timeout).is_err(), "{timeout:?}");
            assert_eq!((latched(&step), step.cursor), (format!("waiting {DEADLINE}"), 2), "{timeout:?}: re-armed");

            for now in [DEADLINE, "2026-11-15T00:00:00Z"] {
                let mut step = execution(open.clone(), &[], at(now));
                assert!(timed::<Value>(&mut step, timeout).is_err(), "{timeout:?} at {now}");
                assert_eq!(latched(&step), "close 1 TimedOut []", "{timeout:?} at {now}: decided, not returned");

                let mut step = execution(open.clone(), &[("m1", json!("v"))], at(now));
                assert!(timed::<Value>(&mut step, timeout).is_err(), "{timeout:?} at {now}");
                assert_eq!(latched(&step), "close 1 Received [m1]", "{timeout:?} at {now}: a kept reply wins past it");
            }
        }
    }

    /// Scenario: Timer execution and later replay. A committed outcome is returned; a timeout is not decoded and does
    /// not look at a reply kept later, nor at the clock.
    #[test]
    fn a_saved_decision_returns_its_outcome_and_later_helpers_go_on() {
        let received = vec![request("ask", "m1"), wait("done", Some(DEADLINE), Some(WaitOutcome::Received))];
        let mut step = execution(received.clone(), &[("m1", json!(7))], at("2027-01-01T00:00:00Z"));
        assert_eq!(timed::<u32>(&mut step, SECOND).unwrap(), Waited::Received(7));
        assert_eq!(latched(&step), "none", "the decoding marker is cleared");
        let mut step = execution(received, &[("m1", Value::Null)], at(NOW));
        assert_eq!(timed::<Option<u32>>(&mut step, SECOND).unwrap(), Waited::Received(None), "null is data");

        let timed_out = vec![request("ask", "m1"), wait("done", Some(DEADLINE), Some(WaitOutcome::TimedOut))];
        for replies in [vec![], vec![("m1", json!("not a number"))]] {
            let mut step = execution(timed_out.clone(), &replies, at(NOW));
            assert_eq!(timed::<u32>(&mut step, SECOND).unwrap(), Waited::TimedOut, "{replies:?}");
            assert_eq!(latched(&step), "none");
            assert!(block_on(step.run("next", |_key| async { Ok(()) })).is_err(), "later helpers go on");
            assert!(latched(&step).starts_with(r#"new {"kind":"step""#), "{}", latched(&step));
        }
    }

    /// Scenario: Replay matching, of misuse: swapped APIs, another handle, a join or a first wait, a reused request, an invalid or
    /// repeated id, and entry 1001.
    #[test]
    fn a_timed_wait_matches_only_a_saved_single_wait_with_a_deadline() {
        let saved = |entry: JournalEntry| vec![request("ask", "m1"), request("other", "m2"), entry];
        let mut join = wait("done", Some(DEADLINE), None);
        if let JournalEntry::Wait { requests, .. } = &mut join {
            requests.push(id("m2"));
        }
        let mut first = wait("done", Some(DEADLINE), None);
        if let JournalEntry::Wait { mode, .. } = &mut first {
            *mode = WaitMode::First;
        }
        let mut other = wait("done", Some(DEADLINE), None);
        if let JournalEntry::Wait { requests, .. } = &mut other {
            requests[0] = id("m2");
        }
        for (journal, why) in [
            (saved(wait("done", None, None)), "wait `done` is saved without a deadline, which Step::wait_timeout"),
            (saved(join), "wait `done` is saved for all of the requests m1, m2, not for the request m1"),
            (saved(first), "wait `done` is saved for the first of the requests m1, not for the request m1"),
            (saved(other), "wait `done` is saved for the request m2, not for the request m1"),
            (saved(JournalEntry::Step { id: "done".into(), result: Value::Null }), "where its journal holds step"),
        ] {
            let mut step = execution(journal, &[], at(NOW));
            let handle = ask::<Value>(&mut step);
            block_on(step.request("other", &job(), |_: ReplyTo<Value>| Never::Go)).unwrap();
            assert!(block_on(step.wait_timeout("done", &handle, SECOND)).is_err());
            assert!(latched(&step).contains(why), "{why}: {}", latched(&step));
        }

        // Step::wait on a saved wait with a deadline.
        let mut step = execution(saved(wait("done", Some(DEADLINE), None)), &[], at(NOW));
        let handle = ask::<Value>(&mut step);
        block_on(step.request("other", &job(), |_: ReplyTo<Value>| Never::Go)).unwrap();
        assert!(block_on(step.wait("done", &handle)).is_err());
        let why = "is saved with a deadline, which Step::wait cannot replay";
        assert!(latched(&step).contains(why), "{}", latched(&step));

        // A new timed wait for a request another wait names, or for none of the journal.
        let journal = vec![request("ask", "m1"), wait("first", Some(DEADLINE), Some(WaitOutcome::TimedOut))];
        let mut step = execution(journal, &[], at(NOW));
        let handle = ask::<Value>(&mut step);
        assert_eq!(block_on(step.wait_timeout("first", &handle, SECOND)).unwrap(), Waited::TimedOut);
        assert!(block_on(step.wait_timeout("again", &handle, SECOND)).is_err());
        assert!(latched(&step).contains("names the request m1 of the wait `first`"), "{}", latched(&step));
        let mut step = execution(vec![request("ask", "m1")], &[], at(NOW));
        ask::<Value>(&mut step);
        assert!(block_on(step.wait_timeout("done", &Request::<Value>::new(id("m9")), SECOND)).is_err());
        assert!(latched(&step).contains("names m9, no earlier request"), "{}", latched(&step));

        // Entry 1000 fits, 1001 does not; an invalid or a repeated id.
        for (count, fits) in [(998, true), (999, false)] {
            let mut journal: Vec<JournalEntry> =
                (0..count).map(|i| JournalEntry::Step { id: format!("s{i}"), result: Value::Null }).collect();
            journal.push(request("ask", "m1"));
            let mut step = execution(journal, &[], at(NOW));
            for i in 0..count {
                block_on(step.run(&format!("s{i}"), |_key| async { Ok(()) })).unwrap();
            }
            assert!(timed::<Value>(&mut step, SECOND).is_err());
            let expected = if fits { r#"new {"kind":"wait""# } else { "permanent wait `done` would be entry 1001" };
            assert!(latched(&step).starts_with(expected), "{count}: {}", latched(&step));
        }
        for (wait, why) in [("ask", "calls the id `ask` twice"), ("a b", "a step id is")] {
            let mut step = execution(vec![request("ask", "m1")], &[], at(NOW));
            let handle = ask::<Value>(&mut step);
            assert!(block_on(step.wait_timeout(wait, &handle, SECOND)).is_err());
            assert!(latched(&step).contains(why), "{why}: {}", latched(&step));
        }
    }

    /// Scenario: Latch and decoding: an unpolled call changes nothing, the first latch wins over a swallowed decision,
    /// an earlier latch wins over the wait, a received reply that does not decode or whose decoder panics fails for
    /// good and blocks later helpers.
    #[test]
    fn the_first_latch_wins_and_decoding_failures_keep_the_outcome() {
        let mut step = execution(vec![request("ask", "m1")], &[], at(NOW));
        let handle = ask::<Value>(&mut step);
        drop(step.wait_timeout("done", &handle, SECOND));
        assert_eq!((latched(&step), step.cursor, step.seen.len()), ("none".to_owned(), 1, 1), "unpolled: nothing");

        for (journal, now) in [
            (vec![request("ask", "m1")], NOW),
            (vec![request("ask", "m1"), wait("done", Some(DEADLINE), None)], NOW),
            (vec![request("ask", "m1"), wait("done", Some(DEADLINE), None)], DEADLINE),
        ] {
            let mut step = execution(journal, &[], at(now));
            let handle = ask::<Value>(&mut step);
            let _ = block_on(step.wait_timeout("done", &handle, SECOND));
            let first = latched(&step);
            let mut invoked = false;
            let caught = block_on(step.run("next", |_key| {
                invoked = true;
                async { Ok(()) }
            }));
            assert!(caught.is_err() && !invoked, "{first}: a later step invokes nothing");
            assert!(block_on(step.wait_timeout("other", &handle, SECOND)).is_err());
            assert_eq!(latched(&step), first, "the swallowed latch stays");
        }

        let mut step = execution(vec![request("ask", "m1"), wait("done", Some(DEADLINE), None)], &[], at(DEADLINE));
        step.latch = Some(Latch::InProgress("step", "a".to_owned()));
        assert!(block_on(step.wait_timeout("done", &Request::<Value>::new(id("m1")), SECOND)).is_err());
        assert_eq!((latched(&step), step.seen.len()), ("in progress step a".to_owned(), 0), "an earlier latch wins");

        let received = vec![request("ask", "m1"), wait("done", Some(DEADLINE), Some(WaitOutcome::Received))];
        let mut step = execution(received.clone(), &[("m1", json!("seven"))], at(NOW));
        assert!(timed::<u32>(&mut step, SECOND).is_err());
        assert!(latched(&step).contains("the saved reply of wait `done` does not decode"), "{}", latched(&step));
        assert!(latched(&step).contains("fix the code and send a public Continue"), "{}", latched(&step));

        /// A reply whose decoder panics.
        struct Exploding;
        impl<'de> Deserialize<'de> for Exploding {
            fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                panic!("the decoder panicked")
            }
        }
        let mut step = execution(received, &[("m1", json!(1))], at(NOW));
        let handle = ask::<Exploding>(&mut step);
        let panicked = catch_unwind(AssertUnwindSafe(|| block_on(step.wait_timeout("done", &handle, SECOND))));
        assert!(panicked.is_err());
        assert_eq!(latched(&step), "in progress wait done");
        let mut invoked = false;
        let _ = block_on(step.run("next", |_key| {
            invoked = true;
            async { Ok(()) }
        }));
        assert!(!invoked && latched(&step) == "in progress wait done", "the marker blocks later helpers");
    }

    /// The reply type is inferred from the request, and a flow that waits with a timeout has a `Send` future.
    #[test]
    fn the_reply_type_is_inferred_and_a_timed_flow_is_send() {
        struct Timed;
        impl Flow for Timed {
            const TYPE: &'static str = "timed";
            type Input = String;

            async fn run(&self, step: &mut Step, name: String) -> HandlerResult {
                let request = step.request("ask", &Job(name), |_reply: ReplyTo<u32>| Never::Go).await?;
                match step.wait_timeout("done", &request, SECOND).await? {
                    Waited::Received(n) => step.run("got", |_key| async move { Ok(n + 1) }).await?,
                    Waited::TimedOut => step.run("late", |_key| async { Ok(0) }).await?,
                };
                Ok(())
            }
        }
        fn send<F: Future + Send>(_: F) {}
        let mut step = execution(vec![], &[], at(NOW));
        send(Timed.run(&mut step, "j1".into()));
    }
}

#[cfg(test)]
mod join_tests {
    //! The joins of Step without a runner: the pure selection, the checks of a new join's list, its requests, its room
    //! and its deadline before early replies; replay by the exact ordered list, mode and deadline presence, with the
    //! structural equivalence of a one-request all wait and a single wait; the paths of new, open and closed joins;
    //! decoding only the selection, in request order; latches; and `Send` futures over borrowed handles.

    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;
    use futures::executor::block_on;
    use serde::Deserializer;
    use serde_json::json;

    const SECOND: Duration = Duration::from_secs(1);
    const NOW: &str = "2026-11-14T00:00:00Z";
    const DEADLINE: &str = "2026-11-14T00:01:00Z";

    fn at(text: &str) -> Timestamp {
        time::parse_instant(text).unwrap()
    }

    fn id(text: &str) -> MessageId {
        MessageId::new(text).unwrap()
    }

    fn ids(texts: &[&str]) -> Vec<MessageId> {
        texts.iter().map(|text| id(text)).collect()
    }

    fn execution(journal: Vec<JournalEntry>, replies: &[(&str, Value)], now: Timestamp) -> Step {
        let mut step = Step::new(journal, &ActorAddress::parse("test.flow/k").unwrap(), now);
        step.replies = replies.iter().map(|(request, value)| (id(request), value.clone())).collect();
        step
    }

    /// The request entry `r<n>` with the message id `m<n>`.
    fn request(n: usize) -> JournalEntry {
        JournalEntry::Send {
            id: format!("r{n}"),
            to: ActorAddress::parse("jobs/j1").unwrap(),
            message: id(&format!("m{n}")),
            create: None,
            reply: true,
        }
    }

    /// The request entries `r1` through `r<count>`.
    fn requests(count: usize) -> Vec<JournalEntry> {
        (1..=count).map(request).collect()
    }

    /// The request entries `r1` through `r<count>` followed by `entry`.
    fn after_requests(count: usize, entry: JournalEntry) -> Vec<JournalEntry> {
        let mut journal = requests(count);
        journal.push(entry);
        journal
    }

    /// The wait `name` of `mode` for `requests`, with the deadline `until`, `outcome` and `received`.
    fn wait(
        name: &str,
        requests: &[&str],
        mode: WaitMode,
        until: Option<&str>,
        outcome: Option<WaitOutcome>,
        received: &[&str],
    ) -> JournalEntry {
        JournalEntry::Wait {
            id: name.into(),
            requests: ids(requests),
            mode,
            until: until.map(at),
            outcome,
            received: ids(received),
        }
    }

    fn latched(step: &Step) -> String {
        match &step.latch {
            None => "none".to_owned(),
            Some(Latch::InProgress(kind, id)) => format!("in progress {kind} {id}"),
            Some(Latch::New(entry, outbox)) => {
                format!("new {} {}", serde_json::to_string(entry).unwrap(), outbox.is_some())
            }
            Some(Latch::Waiting(until)) => format!("waiting {}", time::format_instant(*until)),
            Some(Latch::Close(index, outcome, selected)) => {
                let selected: Vec<&str> = selected.iter().map(MessageId::as_str).collect();
                format!("close {index} {outcome:?} [{}]", selected.join(","))
            }
            Some(Latch::Parked) => "parked".to_owned(),
            Some(Latch::Failed(e)) => format!("failed {e}"),
            Some(Latch::Permanent(reason)) => format!("permanent {reason}"),
        }
    }

    /// `jobs/j1`.
    #[derive(Clone, Serialize, Deserialize)]
    #[serde(transparent)]
    struct Job(String);

    impl ActorRef for Job {
        const TYPE: &'static str = "jobs";
        type Message = Never;
    }

    fn job() -> Job {
        Job("j1".into())
    }

    /// A message that replay never builds.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Never {
        Go,
    }

    /// Replays the requests `r1` through `r<count>` and returns their handles.
    fn ask<T>(step: &mut Step, count: usize) -> Vec<Request<T>> {
        (1..=count)
            .map(|n| block_on(step.request(&format!("r{n}"), &job(), |_: ReplyTo<T>| Never::Go)).expect("replays"))
            .collect()
    }

    /// The handles `m<n>` of `ns`, in that order.
    fn handles<T>(ns: &[usize]) -> Vec<Request<T>> {
        ns.iter().map(|n| Request::new(id(&format!("m{n}")))).collect()
    }

    /// The JSON of a latched new wait `done` for `requests` of `mode`, with `until`, `outcome` and `received`.
    fn new_entry(requests: &str, mode: &str, until: &str, outcome: &str, received: &str) -> String {
        format!(
            r#"new {{"kind":"wait","id":"done","requests":[{requests}],"mode":"{mode}","until":{until},"outcome":{outcome},"received":[{received}]}} false"#
        )
    }

    #[test]
    fn all_selects_every_request_in_request_order_and_first_the_earliest_accepted() {
        let (m1, m2, m3) = (id("m1"), id("m2"), id("m3"));
        let requests = ids(&["m1", "m2", "m3"]);
        assert_eq!(select(&requests, WaitMode::All, &[&m3, &m1]), None, "too few");
        assert_eq!(select(&requests, WaitMode::All, &[&m3, &m1, &m2]), Some(requests.clone()), "request order");
        assert_eq!(timed_out(&requests, WaitMode::All, &[&m3, &m1]), ids(&["m1", "m3"]), "the kept subset, in order");
        assert_eq!(timed_out(&requests, WaitMode::First, &[&m3]), Vec::<MessageId>::new(), "first selects none");
        let other = id("m9");
        assert_eq!(select(&requests, WaitMode::First, &[&other]), None, "an unlisted reply");
        assert_eq!(
            select(&requests, WaitMode::First, &[&other, &m3, &m1]),
            Some(vec![m3.clone()]),
            "earliest accepted"
        );
        assert_eq!(select(&requests, WaitMode::First, &[&m1, &m3]), Some(vec![m1]), "not request priority");
    }

    /// Asserts that a new all join `done` for `asked` after the requests `r1` through `r<count>`, with `timeout`, is
    /// refused for good with `why`, with no reply kept and with the replies to `m1` and `m2` kept.
    fn refused(count: usize, asked: &[usize], timeout: Option<Duration>, why: &str) {
        let kept = [("m1", json!(1)), ("m2", json!(2))];
        for replies in [&[][..], &kept[..]] {
            let mut step = execution(requests(count), replies, at(NOW));
            ask::<Value>(&mut step, count);
            let asked = handles::<Value>(asked);
            let _ = match timeout {
                None => block_on(step.wait_all("done", &asked)).map(|_| ()),
                Some(timeout) => block_on(step.wait_all_timeout("done", &asked, timeout)).map(|_| ()),
            };
            assert!(latched(&step).starts_with("permanent "), "{why}: {}", latched(&step));
            assert!(latched(&step).contains(why), "{why}: {}", latched(&step));
        }
    }

    /// Scenarios: Invalid list or reuse; and Capacity and deadline, of the checks of a new join, made before an early
    /// reply is used: its list, its requests, then its deadline.
    #[test]
    fn a_new_join_checks_its_list_requests_and_deadline_before_its_replies() {
        refused(2, &[], None, "wait `done` names 1 through 100 requests, had 0");
        let all: Vec<usize> = (1..=101).collect();
        refused(101, &all, None, "names 1 through 100 requests, had 101");
        refused(2, &[1, 2, 1], None, "wait `done` names the request m1 twice");
        refused(2, &[1, 9], None, "wait `done` names m9, no earlier request of the journal");
        refused(2, &[1, 2], Some(MAX_DELAY + Duration::from_nanos(1)), "permanent wait `done` lasts");
        let mut plain = request(2);
        if let JournalEntry::Send { reply, .. } = &mut plain {
            *reply = false;
        }
        let mut step = execution(vec![request(1), plain], &[("m1", json!(1))], at(NOW));
        ask::<Value>(&mut step, 1);
        block_on(step.send("r2", &job(), Never::Go)).unwrap();
        let _ = block_on(step.wait_all("done", &handles::<Value>(&[1, 2])));
        assert!(latched(&step).contains("names m2, no earlier request"), "a plain send: {}", latched(&step));

        // A losing request of a closed first wait cannot be reused.
        let closed = wait("w", &["m1", "m2"], WaitMode::First, None, Some(WaitOutcome::Received), &["m2"]);
        let mut step = execution(after_requests(2, closed), &[("m1", json!(1)), ("m2", json!(2))], at(NOW));
        let asked = ask::<Value>(&mut step, 2);
        assert_eq!(block_on(step.wait_first("w", &asked)).unwrap(), (1, json!(2)));
        let _ = block_on(step.wait_all("done", &asked[..1]));
        assert!(latched(&step).contains("names the request m1 of the wait `w`"), "{}", latched(&step));

        // A new deadline after MAX_INSTANT fails, also for a first join whose replies are all kept.
        let late = time::sub(MAX_INSTANT, SECOND);
        let mut step = execution(requests(2), &[("m1", json!(1)), ("m2", json!(2))], late);
        let asked = ask::<Value>(&mut step, 2);
        let _ = block_on(step.wait_first_timeout("done", &asked, SECOND + Duration::from_nanos(1)));
        assert!(latched(&step).starts_with("permanent wait `done` would end at"), "{}", latched(&step));
    }

    /// Scenario: Capacity and deadline: 100 requests joined as entry 1000 take one entry; entry 1001 does not fit.
    #[test]
    fn a_join_of_100_requests_is_one_entry_and_entry_1001_does_not_fit() {
        for (steps, fits) in [(899, true), (900, false)] {
            let mut journal: Vec<JournalEntry> =
                (0..steps).map(|i| JournalEntry::Step { id: format!("s{i}"), result: Value::Null }).collect();
            journal.extend(requests(100));
            let mut step = execution(journal, &[], at(NOW));
            for i in 0..steps {
                block_on(step.run(&format!("s{i}"), |_key| async { Ok(()) })).unwrap();
            }
            let asked = ask::<Value>(&mut step, 100);
            assert!(block_on(step.wait_all("done", &asked)).is_err());
            if fits {
                let Some(Latch::New(JournalEntry::Wait { requests, .. }, None)) = &step.latch else {
                    panic!("{}", latched(&step))
                };
                assert_eq!(requests.len(), 100, "entry 1000 holds the whole join");
            } else {
                assert!(latched(&step).contains("wait `done` would be entry 1001"), "{}", latched(&step));
            }
        }
    }

    /// Scenarios: Zero timeout and supplied progress, of a new join; and Multiple early replies. A new join commits open,
    /// with its deadline when it has one, also with a partial set of replies and a zero timeout; with enough replies it
    /// commits received, the first wait selecting the reply accepted earliest.
    #[test]
    fn a_new_join_latches_open_or_received_with_its_selection() {
        let list = r#""m1","m2","m3""#;
        let mut step = execution(requests(3), &[("m2", json!(2))], at(NOW));
        let asked = ask::<Value>(&mut step, 3);
        assert!(block_on(step.wait_all("done", &asked)).is_err());
        assert_eq!(latched(&step), new_entry(list, "all", "null", "null", ""), "partial: open, parked");

        for timeout in [Duration::ZERO, SECOND * 60] {
            let until = if timeout.is_zero() { NOW } else { DEADLINE };
            let mut step = execution(requests(3), &[("m3", json!(3)), ("m1", json!(1))], at(NOW));
            let asked = ask::<Value>(&mut step, 3);
            assert!(block_on(step.wait_all_timeout("done", &asked, timeout)).is_err());
            assert_eq!(latched(&step), new_entry(list, "all", &format!(r#""{until}""#), "null", ""), "{timeout:?}");
        }

        let all = [("m3", json!(3)), ("m1", json!(1)), ("m2", json!(2))];
        let mut step = execution(requests(3), &all, at(NOW));
        let asked = ask::<Value>(&mut step, 3);
        assert!(block_on(step.wait_all_timeout("done", &asked, Duration::ZERO)).is_err());
        let closed = new_entry(list, "all", &format!(r#""{NOW}""#), r#""received""#, list);
        assert_eq!(latched(&step), closed, "request order, whatever the order of the replies");

        for (kept, winner) in [(["m2", "m1"], r#""m2""#), (["m1", "m2"], r#""m1""#)] {
            let replies: Vec<(&str, Value)> = kept.iter().map(|r| (*r, json!(r))).collect();
            let mut step = execution(requests(3), &replies, at(NOW));
            let asked = ask::<Value>(&mut step, 3);
            assert!(block_on(step.wait_first("done", &asked)).is_err());
            assert_eq!(latched(&step), new_entry(list, "first", "null", r#""received""#, winner), "{kept:?}");
        }
        let mut step = execution(requests(3), &[], at(NOW));
        let asked = ask::<Value>(&mut step, 3);
        assert!(block_on(step.wait_first_timeout("done", &asked, SECOND * 60)).is_err());
        assert_eq!(latched(&step), new_entry(list, "first", &format!(r#""{DEADLINE}""#), "null", ""));
    }

    /// Scenarios: Zero timeout and supplied progress; Supplied open first wait; and the table of decisions: enough
    /// replies close received before the deadline is looked at; a due incomplete join times out with the kept subset
    /// (all) or none (first); an early one re-arms; an untimed one parks.
    #[test]
    fn a_saved_open_join_closes_times_out_waits_or_parks() {
        let open = |mode, until| after_requests(3, wait("done", &["m1", "m2", "m3"], mode, until, None, &[]));
        let all_timed = |step: &mut Step| {
            let asked = ask::<Value>(step, 3);
            block_on(step.wait_all_timeout("done", &asked, SECOND)).map(|_| ())
        };
        let first_timed = |step: &mut Step| {
            let asked = ask::<Value>(step, 3);
            block_on(step.wait_first_timeout("done", &asked, SECOND)).map(|_| ())
        };
        let partial = [("m3", json!(3)), ("m1", json!(1))];
        let full = [("m3", json!(3)), ("m1", json!(1)), ("m2", json!(2))];
        for now in [NOW, DEADLINE, "2027-01-01T00:00:00Z"] {
            let mut step = execution(open(WaitMode::All, Some(DEADLINE)), &full, at(now));
            assert!(all_timed(&mut step).is_err());
            assert_eq!(latched(&step), "close 3 Received [m1,m2,m3]", "{now}: enough replies, deadline or not");
            let mut step = execution(open(WaitMode::First, Some(DEADLINE)), &partial, at(now));
            assert!(first_timed(&mut step).is_err());
            assert_eq!(latched(&step), "close 3 Received [m3]", "{now}: the earliest accepted listed reply");
        }
        for now in [DEADLINE, "2027-01-01T00:00:00Z"] {
            let mut step = execution(open(WaitMode::All, Some(DEADLINE)), &partial, at(now));
            assert!(all_timed(&mut step).is_err());
            assert_eq!(latched(&step), "close 3 TimedOut [m1,m3]", "{now}: the kept subset, in request order");
            let mut step = execution(open(WaitMode::First, Some(DEADLINE)), &[], at(now));
            assert!(first_timed(&mut step).is_err());
            assert_eq!(latched(&step), "close 3 TimedOut []", "{now}: first selects none");
        }
        let mut step = execution(open(WaitMode::All, Some(DEADLINE)), &partial, at("2026-11-14T00:00:59.999999Z"));
        assert!(all_timed(&mut step).is_err());
        assert_eq!((latched(&step), step.cursor), (format!("waiting {DEADLINE}"), 4), "early: re-armed, no entry");
        let mut step = execution(open(WaitMode::All, None), &partial, at("2027-01-01T00:00:00Z"));
        let asked = ask::<Value>(&mut step, 3);
        assert!(block_on(step.wait_all("done", &asked)).is_err());
        assert_eq!(latched(&step), "parked", "untimed and incomplete");
        let mut step = execution(open(WaitMode::First, None), &[], at(NOW));
        let asked = ask::<Value>(&mut step, 3);
        assert!(block_on(step.wait_first("done", &asked)).is_err());
        assert_eq!(latched(&step), "parked");
    }

    /// Scenarios: Partial typed results; First result; and Decode selection and failure, of a committed decision: values
    /// in request order, one slot per request, the winner's position, and only the selection is decoded.
    #[test]
    fn a_closed_join_returns_its_selection_in_request_order_and_decodes_nothing_else() {
        let three = ["m1", "m2", "m3"];
        let received = wait("done", &three, WaitMode::All, None, Some(WaitOutcome::Received), &three);
        let replies = [("m3", json!(3)), ("m1", json!(1)), ("m2", json!(2))];
        let mut step = execution(after_requests(3, received), &replies, at(NOW));
        let asked = ask::<u32>(&mut step, 3);
        assert_eq!(block_on(step.wait_all("done", &asked)).unwrap(), [1, 2, 3]);
        assert_eq!(latched(&step), "none", "the decoding marker is cleared");

        // A timed-out all join: the missing position is not decoded, even when a reply to it is kept later.
        let timed = wait("done", &three, WaitMode::All, Some(DEADLINE), Some(WaitOutcome::TimedOut), &["m1", "m3"]);
        let replies = [("m3", json!(3)), ("m1", json!(1)), ("m2", json!("not a number"))];
        let mut step = execution(after_requests(3, timed), &replies, at(NOW));
        let asked = ask::<u32>(&mut step, 3);
        let slots = block_on(step.wait_all_timeout("done", &asked, SECOND)).unwrap();
        assert_eq!(slots, [Waited::Received(1), Waited::TimedOut, Waited::Received(3)]);
        assert!(block_on(step.run("next", |_key| async { Ok(()) })).is_err(), "later helpers go on");
        assert!(latched(&step).starts_with(r#"new {"kind":"step""#), "{}", latched(&step));

        // A received timed all join has only Received slots.
        let timed =
            wait("done", &["m1", "m2"], WaitMode::All, Some(DEADLINE), Some(WaitOutcome::Received), &["m1", "m2"]);
        let mut step = execution(after_requests(2, timed), &replies, at(NOW));
        let asked = ask::<Value>(&mut step, 2);
        let slots = block_on(step.wait_all_timeout("done", &asked, SECOND)).unwrap();
        assert_eq!(slots, [Waited::Received(json!(1)), Waited::Received(json!("not a number"))]);

        // A first join: the winner's position and value; the losers' replies are never decoded.
        let first = wait("done", &three, WaitMode::First, None, Some(WaitOutcome::Received), &["m2"]);
        let replies = [("m1", json!("loser")), ("m2", json!(2)), ("m3", json!("loser"))];
        let mut step = execution(after_requests(3, first), &replies, at(NOW));
        let asked = ask::<u32>(&mut step, 3);
        assert_eq!(block_on(step.wait_first("done", &asked)).unwrap(), (1, 2));
        for (outcome, received, expected) in [
            (WaitOutcome::Received, &["m3"][..], Waited::Received((2, 3))),
            (WaitOutcome::TimedOut, &[][..], Waited::TimedOut),
        ] {
            let first = wait("done", &three, WaitMode::First, Some(DEADLINE), Some(outcome), received);
            let replies = [("m3", json!(3)), ("m1", json!("loser"))];
            let mut step = execution(after_requests(3, first), &replies, at("2027-01-01T00:00:00Z"));
            let asked = ask::<u32>(&mut step, 3);
            assert_eq!(block_on(step.wait_first_timeout("done", &asked, SECOND)).unwrap(), expected, "{outcome:?}");
            assert_eq!(latched(&step), "none");
        }
    }

    /// Scenario: Decode selection and failure: the second selected value fails to decode for good and keeps the
    /// outcome, and a value whose decoder panics leaves the marker, which blocks later helpers.
    #[test]
    fn a_selected_value_that_does_not_decode_or_whose_decoder_panics_fails_for_good() {
        let three = ["m1", "m2", "m3"];
        let journal = after_requests(3, wait("done", &three, WaitMode::All, None, Some(WaitOutcome::Received), &three));
        let replies = [("m1", json!(1)), ("m2", json!("two")), ("m3", json!(3))];
        let mut step = execution(journal.clone(), &replies, at(NOW));
        let asked = ask::<u32>(&mut step, 3);
        assert!(block_on(step.wait_all("done", &asked)).is_err());
        let why = "the saved reply of wait `done` does not decode as the type asked for, at position 1 (request m2)";
        assert!(latched(&step).contains(why), "{}", latched(&step));
        assert!(latched(&step).contains("fix the code and send a public Continue"), "{}", latched(&step));

        /// A reply whose decoder panics.
        struct Exploding;
        impl<'de> Deserialize<'de> for Exploding {
            fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                panic!("the decoder panicked")
            }
        }
        let mut step = execution(journal, &replies, at(NOW));
        let asked = ask::<Exploding>(&mut step, 3);
        assert!(catch_unwind(AssertUnwindSafe(|| block_on(step.wait_all("done", &asked)))).is_err());
        assert_eq!(latched(&step), "in progress wait done");
        let mut invoked = false;
        let _ = block_on(step.run("next", |_key| {
            invoked = true;
            async { Ok(()) }
        }));
        assert!(!invoked && latched(&step) == "in progress wait done", "the marker blocks later helpers");
    }

    type Call = fn(&mut Step, &[usize]) -> Result<(), HandlerError>;

    fn all_of(step: &mut Step, ns: &[usize]) -> Result<(), HandlerError> {
        block_on(step.wait_all::<Value>("done", &handles(ns))).map(|_| ())
    }

    fn first_of(step: &mut Step, ns: &[usize]) -> Result<(), HandlerError> {
        block_on(step.wait_first::<Value>("done", &handles(ns))).map(|_| ())
    }

    /// Scenarios: Order changed on replay; and Capacity and deadline, of a saved timed join replayed with an invalid
    /// duration.
    #[test]
    fn replay_matches_the_exact_ordered_list_mode_and_deadline_presence() {
        let saved = |mode, until| after_requests(3, wait("done", &["m1", "m2"], mode, until, None, &[]));
        let cases: [(Call, &[usize], &str); 5] = [
            (all_of, &[2, 1], "is saved for all of the requests m1, m2, not for all of the requests m2, m1"),
            (all_of, &[1, 2, 3], "not for all of the requests m1, m2, m3"),
            (all_of, &[1, 3], "not for all of the requests m1, m3"),
            (all_of, &[1], "not for the request m1"),
            (first_of, &[1, 2], "not for the first of the requests m1, m2"),
        ];
        for (call, ns, why) in cases {
            let mut step = execution(saved(WaitMode::All, None), &[("m1", json!(1))], at(NOW));
            ask::<Value>(&mut step, 3);
            assert!(call(&mut step, ns).is_err());
            assert!(latched(&step).contains(why), "{why}: {}", latched(&step));
            assert_eq!(step.journal, saved(WaitMode::All, None), "the saved wait stays");
        }
        let mut step = execution(saved(WaitMode::First, None), &[], at(NOW));
        ask::<Value>(&mut step, 3);
        assert!(all_of(&mut step, &[1, 2]).is_err());
        let why = "saved for the first of the requests m1, m2, not for all of";
        assert!(latched(&step).contains(why), "{}", latched(&step));

        // Deadline presence, both ways.
        let mut step = execution(saved(WaitMode::All, Some(DEADLINE)), &[], at(NOW));
        ask::<Value>(&mut step, 3);
        assert!(all_of(&mut step, &[1, 2]).is_err());
        let why = "wait `done` is saved with a deadline, which Step::wait_all cannot replay";
        assert!(latched(&step).contains(why), "{}", latched(&step));
        let mut step = execution(saved(WaitMode::First, None), &[], at(NOW));
        ask::<Value>(&mut step, 3);
        assert!(block_on(step.wait_first_timeout::<Value>("done", &handles(&[1, 2]), SECOND)).is_err());
        let why = "wait `done` is saved without a deadline, which Step::wait_first_timeout cannot replay";
        assert!(latched(&step).contains(why), "{}", latched(&step));

        // A saved timed join replayed with another, even an invalid, duration keeps its saved deadline.
        for timeout in [Duration::ZERO, MAX_DELAY + SECOND] {
            let mut step = execution(saved(WaitMode::All, Some(DEADLINE)), &[], at(NOW));
            ask::<Value>(&mut step, 3);
            assert!(block_on(step.wait_all_timeout::<Value>("done", &handles(&[1, 2]), timeout)).is_err());
            assert_eq!(latched(&step), format!("waiting {DEADLINE}"), "{timeout:?}");
            let mut step = execution(saved(WaitMode::First, Some(DEADLINE)), &[], at(DEADLINE));
            ask::<Value>(&mut step, 3);
            assert!(block_on(step.wait_first_timeout::<Value>("done", &handles(&[1, 2]), timeout)).is_err());
            assert_eq!(latched(&step), "close 3 TimedOut []", "{timeout:?}");
        }
    }

    /// Scenario: One-request all structural equivalence, in both directions and for the timed pair; a first wait of one
    /// request matches neither.
    #[test]
    fn a_one_request_all_join_and_a_single_wait_replay_each_other() {
        let saved = |until: Option<&str>, outcome| {
            let received: &[&str] = if outcome == Some(WaitOutcome::Received) { &["m1"] } else { &[] };
            after_requests(1, wait("done", &["m1"], WaitMode::All, until, outcome, received))
        };
        // New: the same entry either way.
        for timeout in [None, Some(SECOND * 60)] {
            let mut by_single = execution(requests(1), &[], at(NOW));
            let asked = ask::<Value>(&mut by_single, 1);
            let mut by_join = execution(requests(1), &[], at(NOW));
            let joined = ask::<Value>(&mut by_join, 1);
            match timeout {
                None => {
                    let _ = block_on(by_single.wait("done", &asked[0]));
                    let _ = block_on(by_join.wait_all("done", &joined));
                }
                Some(timeout) => {
                    let _ = block_on(by_single.wait_timeout("done", &asked[0], timeout));
                    let _ = block_on(by_join.wait_all_timeout("done", &joined, timeout));
                }
            }
            assert!(latched(&by_single).starts_with("new "), "{}", latched(&by_single));
            assert_eq!(latched(&by_single), latched(&by_join), "{timeout:?}");
        }
        // Saved by either, replayed by the other.
        let reply = [("m1", json!(7))];
        let mut step = execution(saved(None, Some(WaitOutcome::Received)), &reply, at(NOW));
        let asked = ask::<u32>(&mut step, 1);
        assert_eq!(block_on(step.wait_all("done", &asked)).unwrap(), [7]);
        let mut step = execution(saved(None, Some(WaitOutcome::Received)), &reply, at(NOW));
        let asked = ask::<u32>(&mut step, 1);
        assert_eq!(block_on(step.wait("done", &asked[0])).unwrap(), 7);
        let mut step = execution(saved(Some(DEADLINE), Some(WaitOutcome::TimedOut)), &[], at(NOW));
        let asked = ask::<u32>(&mut step, 1);
        assert_eq!(block_on(step.wait_all_timeout("done", &asked, SECOND)).unwrap(), [Waited::TimedOut]);
        let mut step = execution(saved(Some(DEADLINE), Some(WaitOutcome::Received)), &reply, at(NOW));
        let asked = ask::<u32>(&mut step, 1);
        assert_eq!(block_on(step.wait_timeout("done", &asked[0], SECOND)).unwrap(), Waited::Received(7));
        let mut step = execution(saved(Some(DEADLINE), None), &[], at(DEADLINE));
        let asked = ask::<u32>(&mut step, 1);
        assert!(block_on(step.wait_all_timeout("done", &asked, SECOND)).is_err());
        assert_eq!(latched(&step), "close 1 TimedOut []", "an open single wait decided by the join helper");

        // Mode first never matches all, with one request too.
        let first =
            after_requests(1, wait("done", &["m1"], WaitMode::First, None, Some(WaitOutcome::Received), &["m1"]));
        let mut step = execution(first.clone(), &reply, at(NOW));
        let asked = ask::<u32>(&mut step, 1);
        assert_eq!(block_on(step.wait_first("done", &asked)).unwrap(), (0, 7));
        for single in [true, false] {
            let mut step = execution(first.clone(), &reply, at(NOW));
            let asked = ask::<u32>(&mut step, 1);
            let _ = if single {
                block_on(step.wait("done", &asked[0])).map(|_| ())
            } else {
                block_on(step.wait_all("done", &asked)).map(|_| ())
            };
            let why = "is saved for the first of the requests m1, not for the request m1";
            assert!(latched(&step).contains(why), "{}", latched(&step));
        }
        let mut step = execution(saved(None, Some(WaitOutcome::Received)), &reply, at(NOW));
        let asked = ask::<u32>(&mut step, 1);
        assert!(block_on(step.wait_first("done", &asked)).is_err());
        let why = "saved for the request m1, not for the first of the requests m1";
        assert!(latched(&step).contains(why), "{}", latched(&step));
    }

    /// Scenario: Latch and decoding, of joins: an unpolled join changes nothing, a swallowed join's latch wins over
    /// later helpers, and an earlier latch wins over a join.
    #[test]
    fn the_first_latch_wins_over_and_against_a_join() {
        let mut step = execution(requests(2), &[], at(NOW));
        let asked = ask::<Value>(&mut step, 2);
        drop(step.wait_all("done", &asked));
        drop(step.wait_first_timeout("done", &asked, SECOND));
        assert_eq!((latched(&step), step.cursor, step.seen.len()), ("none".to_owned(), 2, 2), "unpolled: nothing");
        let _ = block_on(step.wait_first("done", &asked));
        let first = latched(&step);
        assert!(first.starts_with("new "), "{first}");
        let mut invoked = false;
        let _ = block_on(step.run("next", |_key| {
            invoked = true;
            async { Ok(()) }
        }));
        assert!(block_on(step.wait_all("other", &asked)).is_err());
        assert!(!invoked && latched(&step) == first, "the swallowed join's latch stays");

        let mut step = execution(requests(2), &[], at(NOW));
        step.latch = Some(Latch::InProgress("step", "a".to_owned()));
        assert!(block_on(step.wait_all_timeout::<Value>("done", &handles(&[1, 2]), SECOND)).is_err());
        assert_eq!((latched(&step), step.seen.len()), ("in progress step a".to_owned(), 0), "an earlier latch wins");
    }

    /// Scenario: Send future and borrowed handles: a flow that joins a `Vec` or an array of requests whose reply type is
    /// `Send` but not `Sync` has a `Send` future.
    #[test]
    fn a_flow_that_joins_borrowed_handles_of_a_reply_that_is_not_sync_is_send() {
        struct Joins;
        impl Flow for Joins {
            const TYPE: &'static str = "joins";
            type Input = String;

            async fn run(&self, step: &mut Step, name: String) -> HandlerResult {
                let a = step.request("a", &Job(name.clone()), |_reply: ReplyTo<Cell<u32>>| Never::Go).await?;
                let b = step.request("b", &Job(name), |_reply: ReplyTo<Cell<u32>>| Never::Go).await?;
                let pair = [a.clone(), b.clone()];
                let all: Vec<Cell<u32>> = step.wait_all("all", &pair).await?;
                let list = vec![a, b];
                let (_, first) = step.wait_first("first", &list).await?;
                let slots = step.wait_all_timeout("slots", &list, SECOND).await?;
                let won = step.wait_first_timeout("won", &pair, SECOND).await?;
                let total = all.len() + slots.len() + first.get() as usize + usize::from(won == Waited::TimedOut);
                step.run("sum", |_key| async move { Ok(total) }).await?;
                Ok(())
            }
        }
        fn send<F: Future + Send>(_: F) {}
        let mut step = execution(vec![], &[], at(NOW));
        send(Joins.run(&mut step, "j1".into()));
    }
}
