//! The supervisor (ADR 0028): an actor that starts its children, watches them, and restarts one that is at fault.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use cicada::prelude::*;
use cicada::store::MessageId;
use cicada::time::{self, Timestamp};
use cicada::turn::TurnError;
use cicada::{Failed, FailureCause, StopCause, Stopped};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{EncodeError, StoredMessage};

/// A supervisor, `cicada.supervisor/<name>`; any name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Supervisor {
    /// The name: the key of its address.
    pub name: String,
}

impl Supervisor {
    /// The supervisor named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for Supervisor {
    const TYPE: &'static str = "cicada.supervisor";
    type Message = SupervisorMsg;
}

/// The messages of a [`Supervisor`]. `StartChild`, `Restarted` and `Escalate` are its own: it acts on them only when it
/// sent them itself, and from anybody else they end `done` and change nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum SupervisorMsg {
    /// Starts every child of the state that is not running, each in a turn of its own; ends a restart that is under
    /// way of every child that is running, and takes back one that was given up. A second `Start` starts what the
    /// first could not.
    Start,
    /// Starts one child: in one commit, creates it as a child of the supervisor with the state of its entry, watches
    /// it, sends it its start message and marks it as running. Sent by the supervisor to itself, with a retry policy.
    StartChild {
        /// The child.
        actor: ActorAddress,
    },
    /// Ends the restart of a child whose id the supervisor kept. Sent by the supervisor to itself in a restart.
    Restarted {
        /// The child.
        actor: ActorAddress,
    },
    /// Fails, so that the watchers of the supervisor are told: as a handler fails when `restart` is set (a supervisor
    /// above restarts this one), else refused for good. Sent by the supervisor to itself when it gives up on a child.
    Escalate {
        /// The child it gave up on.
        actor: ActorAddress,
        /// The error the message fails with.
        error: String,
        /// True after too many restarts; false for a child that cannot be started or read.
        restart: bool,
    },
    /// A message of a child ended `failed`.
    Failed(Failed),
    /// A child went away.
    Stopped(Stopped),
}

/// The default of [`SupervisorState::max_restarts`].
pub const DEFAULT_MAX_RESTARTS: u32 = 3;
/// The default of [`SupervisorState::within_seconds`].
pub const DEFAULT_WITHIN_SECONDS: u64 = 60;

fn default_max_restarts() -> u32 {
    DEFAULT_MAX_RESTARTS
}

fn default_within_seconds() -> u64 {
    DEFAULT_WITHIN_SECONDS
}

/// The state of a [`Supervisor`]: its children, fixed for its life, and its restart limit. Build it with
/// `SupervisorState::new`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorState {
    /// The children by address.
    #[serde(default)]
    pub children: BTreeMap<ActorAddress, ChildEntry>,
    /// The most restarts of one child within [`Self::within_seconds`]; one failure more gives the child up.
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
    /// The window of the restart limit, in seconds.
    #[serde(default = "default_within_seconds")]
    pub within_seconds: u64,
}

impl SupervisorState {
    /// A state without children and with the default limit: 3 restarts within 60 seconds.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Worker { id: String }
    /// # impl ActorRef for Worker { const TYPE: &'static str = "worker"; type Message = WorkerMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum WorkerMsg { Begin { batch: u32 } }
    /// # #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
    /// # struct WorkerState { done: u32 }
    /// use cicada_actors::{Child, SupervisorState};
    ///
    /// let worker = Worker { id: "w1".into() };
    /// let state = SupervisorState::new()
    ///     .child(Child::new(&worker, &WorkerState::default())?.start(WorkerMsg::Begin { batch: 7 })?)
    ///     .restart_limit(5, std::time::Duration::from_secs(120));
    /// assert_eq!(state.children.len(), 1);
    /// # Ok::<(), cicada_actors::EncodeError>(())
    /// ```
    pub fn new() -> Self {
        Self { children: BTreeMap::new(), max_restarts: DEFAULT_MAX_RESTARTS, within_seconds: DEFAULT_WITHIN_SECONDS }
    }

    /// This state with the child `child`.
    pub fn child<R: ActorRef>(mut self, child: Child<R>) -> Self {
        self.children.insert(child.address, child.entry);
        self
    }

    /// This state with the restart limit of `max_restarts` restarts of one child within `within` (whole seconds).
    pub fn restart_limit(mut self, max_restarts: u32, within: Duration) -> Self {
        self.max_restarts = max_restarts;
        self.within_seconds = within.as_secs();
        self
    }
}

impl Default for SupervisorState {
    fn default() -> Self {
        Self::new()
    }
}

/// What a supervisor holds for one child: how to start it, and what it knows of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChildEntry {
    /// The state the child is created with, at its start and at each restart.
    pub state: Value,
    /// The message sent to the child after each start and restart; none when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<StoredMessage>,
    /// True while a life of the child that the supervisor started is running.
    #[serde(default)]
    pub running: bool,
    /// The id of the `Restarted` of a restart that is under way; while it is set, failures of the child are ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_id: Option<MessageId>,
    /// True when the supervisor gave up on the child.
    #[serde(default)]
    pub given_up: bool,
    /// Why it gave up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// The times of its restarts (store time) within the window of the limit.
    #[serde(default)]
    pub restarts: Vec<Timestamp>,
}

impl ChildEntry {
    /// An entry that creates the child with `state`, not started yet.
    pub fn new(state: Value) -> Self {
        Self {
            state,
            start: None,
            running: false,
            restart_id: None,
            given_up: false,
            last_error: None,
            restarts: Vec::new(),
        }
    }
}

/// A child of a [`SupervisorState`], built from a typed ref: the compiler checks its start message against the family
/// of the ref.
///
/// ```compile_fail
/// # use cicada::prelude::*;
/// # use serde::{Deserialize, Serialize};
/// # #[derive(Debug, Clone, Serialize, Deserialize)]
/// # struct Worker { id: String }
/// # impl ActorRef for Worker { const TYPE: &'static str = "worker"; type Message = WorkerMsg; }
/// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// # #[serde(tag = "type")]
/// # enum WorkerMsg { Begin { batch: u32 } }
/// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// # #[serde(tag = "type")]
/// # enum MailerMsg { Send { to: String } }
/// use cicada_actors::Child;
///
/// let worker = Worker { id: "w1".into() };
/// let child = Child::new(&worker, &cicada::NoState {}).unwrap().start(MailerMsg::Send { to: "a@b.c".into() });
/// ```
pub struct Child<R: ActorRef> {
    address: ActorAddress,
    entry: ChildEntry,
    of: PhantomData<fn() -> R>,
}

impl<R: ActorRef> Child<R> {
    /// The child `child`, created with `state`. Fails for a ref without an address and for a state that does not
    /// encode or is `null`.
    pub fn new<S: Serialize + ?Sized>(child: &R, state: &S) -> Result<Self, EncodeError> {
        let address = child.address().map_err(|e| EncodeError(e.to_string()))?;
        let state = serde_json::to_value(state)
            .map_err(|e| EncodeError(format!("The state of {address} cannot be encoded: {e}")))?;
        if state.is_null() {
            return Err(EncodeError(format!("The state of {address} is null; a state is never null")));
        }
        Ok(Self { address, entry: ChildEntry::new(state), of: PhantomData })
    }

    /// This child with the start message `message`, sent to it after each start and restart.
    pub fn start(mut self, message: impl Into<R::Message>) -> Result<Self, EncodeError> {
        self.entry.start = Some(StoredMessage::of(&message.into())?);
        Ok(self)
    }
}

impl<R: ActorRef> fmt::Debug for Child<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Child").field("address", &self.address).field("entry", &self.entry).finish()
    }
}

/// The supervisor type. It has no settings: the children and the limit are in the state of each supervisor.
///
/// **A restart empties the mailbox of the child**: its pending messages end `purged`, no dead letter is written for
/// them, and the child gets the state of its entry again. So supervise actors whose state can be built again from the
/// state of their entry; dead letters and a topic are not such actors, a supervisor is.
///
/// A supervisor restarts a child whose message failed through a fault of the child: the cause `handler` or `timeout`.
/// A restart is one commit: it deletes the child, creates it again with the state of its entry, sends it its start
/// message and sends itself `Restarted`. One incident is one restart: until its `Restarted` comes, failures of the
/// child are ignored. A child whose state or address its type cannot read (the cause `state`) is given up and shown;
/// every other cause restarts nothing. After too many restarts it gives up on the child and fails a message of its
/// own, so that a supervisor above it restarts it. Host it on nodes that host the types of its children: they check an
/// entry when the child starts.
#[derive(Debug, Clone, Copy, Default)]
pub struct SupervisorActor;

impl Actor for SupervisorActor {
    type Ref = Supervisor;
    type State = SupervisorState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SupervisorMsg) -> HandlerResult {
        let own = turn.message().from.as_ref() == Some(turn.self_address());
        match message {
            SupervisorMsg::Start => start(turn),
            SupervisorMsg::StartChild { actor } if own => start_child(turn, &actor).await,
            SupervisorMsg::Restarted { actor } if own => {
                restarted(turn, &actor);
                Ok(())
            }
            SupervisorMsg::Escalate { error, restart, .. } if own => {
                Err(if restart { HandlerError::msg(error) } else { HandlerError::permanent(error) })
            }
            // Not sent by the supervisor itself: a client, or the redrive of a dead letter.
            SupervisorMsg::StartChild { .. } | SupervisorMsg::Restarted { .. } | SupervisorMsg::Escalate { .. } => {
                Ok(())
            }
            SupervisorMsg::Failed(failed) => child_failed(turn, &failed).await,
            SupervisorMsg::Stopped(stopped) => child_stopped(turn, &stopped),
        }
    }
}

/// The retry policy of a `StartChild` and of a delivery of a topic: up to 7 attempts, the waits doubling from one
/// second, 63 seconds in all.
pub(crate) fn retry_policy() -> RetryPolicy {
    let backoff = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(32)).expect("a valid backoff");
    RetryPolicy::new(7, backoff).expect("a valid retry policy")
}

/// True when the message of this turn has no attempt after this one.
pub(crate) fn last_attempt<A: Actor>(turn: &Turn<A>) -> bool {
    let message = turn.message();
    message.retry.as_ref().is_none_or(|policy| !policy.retries_after(message.failures + 1))
}

fn start(turn: &mut Turn<SupervisorActor>) -> HandlerResult {
    let waiting: Vec<ActorAddress> =
        turn.state().children.iter().filter(|(_, entry)| !entry.running).map(|(actor, _)| actor.clone()).collect();
    for actor in waiting {
        turn.send_self_with(SupervisorMsg::StartChild { actor }, SendOptions::new().retry(retry_policy()))?;
    }
    let takes_back = |entry: &ChildEntry| entry.running && (entry.restart_id.is_some() || entry.given_up);
    if turn.state().children.values().any(takes_back) {
        for entry in turn.state_mut().children.values_mut().filter(|entry| entry.running) {
            entry.restart_id = None;
            if entry.given_up {
                entry.given_up = false;
                entry.last_error = None;
                entry.restarts.clear();
            }
        }
    }
    Ok(())
}

/// A call of a start or a restart that was refused, and whether the child was created before it.
struct Refused {
    created: bool,
    error: TurnError,
}

/// The calls of a start, in their fixed order: the create of the child as a child of the supervisor, the watch (only
/// at a start: a restart keeps the watch), and the start message.
async fn create_and_send_start(
    turn: &mut Turn<SupervisorActor>,
    actor: &ActorAddress,
    entry: &ChildEntry,
    watch: bool,
) -> Result<(), Refused> {
    let refused = |created| move |error| Refused { created, error };
    turn.create_child_raw(actor, entry.state.clone()).await.map_err(refused(false))?;
    if watch {
        turn.watch(actor).map_err(refused(true))?;
    }
    if let Some(start) = &entry.start {
        turn.send_raw(actor, &start.message_type, start.payload.clone(), SendOptions::new())
            .await
            .map_err(refused(true))?;
    }
    Ok(())
}

async fn start_child(turn: &mut Turn<SupervisorActor>, actor: &ActorAddress) -> HandlerResult {
    let Some(entry) = turn.state().children.get(actor).cloned() else {
        return Ok(());
    };
    if entry.running {
        return Ok(());
    }
    match create_and_send_start(turn, actor, &entry, true).await {
        Ok(()) => {
            let entry = child_mut(turn, actor);
            entry.running = true;
            entry.restart_id = None;
            entry.given_up = false;
            entry.last_error = None;
            entry.restarts.clear();
            Ok(())
        }
        // A failed read of the store, or a refused call while attempts are left: the attempt fails, nothing changes,
        // and the next attempt or a later `Start` tries again.
        Err(Refused { error: error @ TurnError::Store(_), .. }) => Err(error.into()),
        Err(Refused { error, .. }) if !last_attempt(turn) => Err(error.into()),
        Err(Refused { created, error }) => {
            let error = error.to_string();
            give_up(turn, actor, &error, created);
            escalate(turn, actor, format!("cannot start {actor}: {error}"), false)
        }
    }
}

fn restarted(turn: &mut Turn<SupervisorActor>, actor: &ActorAddress) {
    let id = turn.message().id.clone();
    if turn.state().children.get(actor).is_some_and(|entry| entry.restart_id.as_ref() == Some(&id)) {
        child_mut(turn, actor).restart_id = None;
    }
}

async fn child_failed(turn: &mut Turn<SupervisorActor>, failed: &Failed) -> HandlerResult {
    let actor = &failed.actor;
    let Some(entry) = turn.state().children.get(actor) else {
        return Ok(());
    };
    if !entry.running || entry.given_up || entry.restart_id.is_some() {
        return Ok(());
    }
    match failed.cause {
        FailureCause::Handler | FailureCause::Timeout => restart(turn, actor, &failed.error).await,
        FailureCause::State => {
            give_up(turn, actor, &failed.error, true);
            escalate(turn, actor, format!("cannot read {actor}: {}", failed.error), false)
        }
        FailureCause::Refused
        | FailureCause::Message
        | FailureCause::Commit
        | FailureCause::Node
        | FailureCause::ClaimLimit => Ok(()),
    }
}

async fn restart(turn: &mut Turn<SupervisorActor>, actor: &ActorAddress, error: &str) -> HandlerResult {
    let now = turn.now();
    let within = Duration::from_secs(turn.state().within_seconds);
    let max = turn.state().max_restarts as usize;
    let entry = turn.state().children[actor].clone();
    let recent: Vec<Timestamp> =
        entry.restarts.iter().copied().filter(|at| time::add(*at, within).is_ok_and(|end| end > now)).collect();
    if recent.len() >= max {
        let failures = recent.len() + 1;
        child_mut(turn, actor).restarts = recent;
        give_up(turn, actor, error, true);
        let within = time::format_duration(within);
        return escalate(
            turn,
            actor,
            format!("restart limit: {actor} failed {failures} times in {within}: {error}"),
            true,
        );
    }
    // One commit, in this order: the delete, the create, the start message, and `Restarted` to itself.
    turn.delete_at(actor)?;
    match create_and_send_start(turn, actor, &entry, false).await {
        Ok(()) => {
            let id = turn.send_self(SupervisorMsg::Restarted { actor: actor.clone() })?;
            let entry = child_mut(turn, actor);
            entry.restart_id = Some(id);
            entry.restarts = recent;
            entry.restarts.push(now);
            Ok(())
        }
        // Nothing is committed, and the child keeps running.
        Err(Refused { error: error @ TurnError::Store(_), .. }) => Err(error.into()),
        // The calls before the refused one stay: without the create, the child is deleted and not created again.
        Err(Refused { created, error }) => {
            let error = error.to_string();
            give_up(turn, actor, &error, created);
            escalate(turn, actor, format!("cannot start {actor}: {error}"), false)
        }
    }
}

fn child_stopped(turn: &mut Turn<SupervisorActor>, stopped: &Stopped) -> HandlerResult {
    let actor = &stopped.actor;
    let Some(entry) = turn.state().children.get(actor) else {
        return Ok(());
    };
    // The life that a restart of the supervisor itself ended.
    if stopped.cause == StopCause::Deleted && stopped.by.as_ref() == Some(turn.self_address()) {
        return Ok(());
    }
    // The child stopped itself, or somebody else deleted it: it is meant to be gone. During a restart, the life the
    // restart created goes too.
    let restarting = entry.restart_id.is_some();
    turn.state_mut().children.remove(actor);
    turn.unwatch(actor)?;
    if restarting {
        turn.delete_at(actor)?;
    }
    Ok(())
}

/// Marks the child as given up with `error`; `running` says whether a life of it that the supervisor started runs.
fn give_up(turn: &mut Turn<SupervisorActor>, actor: &ActorAddress, error: &str, running: bool) {
    let entry = child_mut(turn, actor);
    entry.running = running;
    entry.restart_id = None;
    entry.given_up = true;
    entry.last_error = Some(error.to_owned());
}

fn escalate(turn: &mut Turn<SupervisorActor>, actor: &ActorAddress, error: String, restart: bool) -> HandlerResult {
    turn.send_self(SupervisorMsg::Escalate { actor: actor.clone(), error, restart })?;
    Ok(())
}

fn child_mut<'a>(turn: &'a mut Turn<SupervisorActor>, actor: &ActorAddress) -> &'a mut ChildEntry {
    turn.state_mut().children.get_mut(actor).expect("a child of the state")
}
