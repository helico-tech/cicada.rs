//! Durable flows (ADR 0031): a multi-step job written as ordinary async code. Each run is one actor that replays the
//! results its journal holds and executes one new step or sleep per turn, saving it and arming the next `Continue` in
//! the same commit; a sleep's `Continue` is due at its saved wake time.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, MAX_DELAY, MAX_INSTANT, Timestamp};
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

    /// The retry policy of every `Continue` the runner sends itself, and so of every step; by default the crate's
    /// shared policy (7 attempts, exponential from 1 s to 32 s).
    fn retry() -> RetryPolicy {
        retry_policy()
    }

    /// The flow's code. It calls [`Step::run`] for each external step and [`Step::sleep`] for each durable wait, and
    /// passes their errors on with `?`; it returns `Ok(())` once every step is done.
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

/// The messages of a [`FlowRun`]. The wire holds `Reply` and `Cancel` (ADR 0044) ahead of the releases that implement
/// them: this release refuses both for good, so upgrade every host and worker of a flow type before sending them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum FlowMsg {
    /// Starts or recovers a running run; a done or cancelled run ignores it. Sent by a host, it cancels the armed
    /// continuation and arms a fresh one with the flow's retry policy, without running flow code; the armed one replays
    /// the flow and executes its next step.
    Continue,
    /// The reply of a target to a request of the run; refused for good by this release, which sends no requests.
    Reply {
        /// The id of the request's message, saved with its send entry.
        request: MessageId,
        /// The reply's JSON, `null` included; required, so a missing value is not `null`.
        value: Value,
    },
    /// Asks the run to stop; refused for good by this release.
    Cancel,
}

/// The phase of a run. This release executes only running runs; `cancelling` and `cancelled` are on the wire for later
/// releases (ADR 0044).
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
    /// A cancelled run; it keeps its state until it is deleted.
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
        /// Whether the message is a request that a reply answers; absent: false. This release sends no requests.
        #[serde(default)]
        reply: bool,
    },
    /// A wait for the replies of earlier requests; this release does not execute waits.
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
        let JournalEntry::Wait { id, requests, mode, until, outcome, received } = entry else {
            continue;
        };
        let refuse = |problem: String| Err(format!("wait `{id}` of {phase} {problem}"));
        if let Some(open) = waits.open {
            return refuse(format!("follows the open wait `{open}`"));
        }
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

/// What the first new step or sleep, the first sleep not yet due, or the first misuse, of an execution decided. It
/// wins over what the flow returns.
enum Latch {
    /// A closure was invoked and has not finished: its future was dropped, or it panicked, if this is left.
    InProgress(String),
    /// A new entry to save with the next continuation, which a sleep's wake time delays.
    New(JournalEntry),
    /// A saved sleep whose wake time has not come: the next continuation is due then, and nothing is saved.
    Waiting(Timestamp),
    /// The closure's own error, with its retryability.
    Failed(HandlerError),
    /// A replay or result that cannot be saved; refused for good.
    Permanent(String),
}

/// The private error by which [`Step::run`] and [`Step::sleep`] suspend a flow: passed on with `?`, it ends the
/// execution.
#[derive(Debug)]
struct Suspend;

impl fmt::Display for Suspend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the flow is suspended after a new step or at a sleep; pass this error on with `?`")
    }
}

impl std::error::Error for Suspend {}

/// The steps of one execution of a flow: it replays saved results and elapsed sleeps in journal order and executes at
/// most one new step or sleep. A flow sees it only through [`Flow::run`]; it gives no access to the turn.
pub struct Step {
    journal: Vec<JournalEntry>,
    cursor: usize,
    seen: HashSet<String>,
    address: String,
    /// Store time at the claim: what a sleep counts from and is compared with.
    now: Timestamp,
    latch: Option<Latch>,
}

impl fmt::Debug for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Step").field("run", &self.address).field("cursor", &self.cursor).finish_non_exhaustive()
    }
}

impl Step {
    fn new(journal: Vec<JournalEntry>, address: &ActorAddress, now: Timestamp) -> Self {
        Self { journal, cursor: 0, seen: HashSet::new(), address: address.as_str().to_owned(), now, latch: None }
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
        self.latch = Some(Latch::InProgress(id.to_owned()));
        let outcome = call(format!("{}/{id}", self.address)).await;
        let latch = match outcome {
            Err(e) => Latch::Failed(e),
            Ok(value) => match encode(id, &value) {
                Ok(result) => Latch::New(JournalEntry::Step { id: id.to_owned(), result }),
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
            Ok(None) => match self.room(id, "sleep").and_then(|()| wake_time(id, self.now, duration)) {
                Ok(until) => Latch::New(JournalEntry::Sleep { id: id.to_owned(), until }),
                Err(reason) => Latch::Permanent(reason),
            },
        };
        self.latch = Some(latch);
        Err(HandlerError::new(Suspend))
    }

    /// Checks that the journal has room for the new step or sleep `id`.
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

/// The wake time of a new sleep: `now + duration`, rounded up to microseconds; the duration at most [`MAX_DELAY`] and
/// the time at most [`MAX_INSTANT`].
fn wake_time(id: &str, now: Timestamp, duration: Duration) -> Result<Timestamp, String> {
    if duration > MAX_DELAY {
        return Err(format!(
            "sleep `{id}` lasts {}, longer than the most a sleep may last, {}",
            time::format_duration(duration),
            time::format_duration(MAX_DELAY)
        ));
    }
    let until = time::add(now, duration).map(time::ceil).map_err(|e| format!("sleep `{id}` cannot end: {e}"))?;
    if until > MAX_INSTANT {
        return Err(format!(
            "sleep `{id}` would end at {}, after {}",
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
/// a replayed sleep not yet due arms one due at that same time and saves nothing. A flow that returns `Ok` after replaying every entry marks the run done
/// and clears `armed`; a done run ignores `Continue` and keeps its state until it is deleted. An execution that fails
/// commits nothing; its message follows its retry policy, and its watchers are told as for any actor. The runner
/// installs no watch.
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
        match message {
            FlowMsg::Continue => {}
            FlowMsg::Reply { .. } | FlowMsg::Cancel => return Err(HandlerError::permanent(later_release(&message))),
        }
        match turn.state().status {
            FlowStatus::Running => {}
            FlowStatus::Done | FlowStatus::Cancelled => return Ok(()),
            FlowStatus::Cancelling => {
                return Err(HandlerError::permanent(
                    "this release of cicada-actors cannot run the cleanup of a cancelling run; a later release can, \
                     once every host and worker of the flow type runs it",
                ));
            }
        }
        if turn.state().armed.as_ref() != Some(&turn.message().id) {
            if let Some(old) = turn.state().armed.clone() {
                turn.cancel(&old)?;
            }
            return arm(turn, None);
        }
        let state = turn.state();
        let mut step = Step::new(state.journal.clone(), turn.self_address(), turn.now());
        let returned = self.flow.run(&mut step, state.input.clone()).await;
        match step.latch.take() {
            Some(Latch::InProgress(id)) => Err(HandlerError::permanent(format!(
                "step `{id}` was abandoned while its closure ran (its future was dropped, or it panicked); a timeout \
                 belongs inside the closure"
            ))),
            Some(Latch::Failed(e)) => Err(e),
            Some(Latch::Permanent(reason)) => Err(HandlerError::permanent(reason)),
            Some(Latch::New(entry)) => {
                let wake = match &entry {
                    JournalEntry::Sleep { until, .. } => Some(*until),
                    _ => None,
                };
                turn.state_mut().journal.push(entry);
                arm(turn, wake)
            }
            Some(Latch::Waiting(until)) => arm(turn, Some(until)),
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

/// Why replay stopped at `saved`: a wait or a request needs a later release; anything else, changed code.
fn unreplayable(saved: &JournalEntry) -> &'static str {
    match saved {
        JournalEntry::Wait { .. } | JournalEntry::Send { reply: true, .. } => {
            "this release of cicada-actors cannot replay waits or requests; a later release can, once every host and \
             worker of the flow type runs it"
        }
        _ => "changed flow code cannot replay this run",
    }
}

/// The diagnostic of a message this release refuses for good.
fn later_release(message: &FlowMsg) -> String {
    let name = match message {
        FlowMsg::Reply { .. } => "Reply",
        _ => "Cancel",
    };
    format!(
        "FlowMsg::{name} needs a later release of cicada-actors; this one refuses it for good. Upgrade every host and \
         worker of the flow type before sending it"
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
            Some(Latch::InProgress(id)) => format!("in progress {id}"),
            Some(Latch::New(entry)) => format!("new {}", serde_json::to_string(entry).unwrap()),
            Some(Latch::Waiting(until)) => format!("waiting {}", time::format_instant(*until)),
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
        step.latch = Some(Latch::InProgress("a".to_owned()));
        assert!(sleep(&mut step, "w", SECOND));
        assert_eq!(latched(&step), "in progress a", "an abandoned step stays the outcome");
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
