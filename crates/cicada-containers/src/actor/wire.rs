//! What a container actor takes, keeps and sends: its messages, its state and its end.

use cicada::prelude::*;
use cicada::runtime::limits::MAX_TYPE_LENGTH;
use cicada::time::Timestamp;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Settings, a `Start` or a state of a container actor out of their bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidContainer(pub(crate) String);

/// The messages of a container actor. `Start`, `Stop` and `Remove` only set the goal and arm a poll; the goal moves only
/// forward, from run to stop to remove. Only the armed `Poll` acts.
///
/// ```
/// use cicada_containers::ContainerMsg;
///
/// assert_eq!(serde_json::to_value(ContainerMsg::Remove)?, serde_json::json!({"type": "Remove"}));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum ContainerMsg {
    /// Runs the container until it ends, and sends the end to `to`. The first `Start` sets the target; a later one
    /// changes nothing, unless the container has ended: then it sends the saved end to its own target at once, again
    /// for each such `Start`. A message type out of bounds is refused for good, and nothing is saved.
    Start {
        /// The target of the end.
        to: ActorAddress,
        /// The message type of the end, whose payload is `{"key": …, "end": …}`, a [`ContainerEnded`]; not empty, at
        /// most 200 characters, without NUL. The target's registry is checked only when the end is sent.
        #[schemars(length(min = 1, max = 200))]
        message_type: String,
    },
    /// Stops the container and ends it `stopped`, or `timedOut` past its deadline; one never started is never created.
    Stop,
    /// Stops the container if needed, records its end, then removes it. Send it before deleting the actor, or the
    /// container stays.
    Remove,
    /// Arms a fresh poll when one was lost: due when the armed one was, or at once when that passed. Does nothing with no
    /// work left.
    Check,
    /// The actor's own timer: only the armed one acts, by taking one step toward the goal. Any other is ignored.
    Poll,
}

impl ContainerMsg {
    /// A `Start` whose end goes to `target` as `message_type`. Fails for a ref without an address, and for a message
    /// type that is empty, longer than `MAX_TYPE_LENGTH` characters or holds a NUL.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Build { id: String }
    /// # impl ActorRef for Build { const TYPE: &'static str = "build"; type Message = BuildMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum BuildMsg { Ended(cicada_containers::ContainerEnded) }
    /// use cicada_containers::ContainerMsg;
    ///
    /// let start = ContainerMsg::start(&Build { id: "b1".into() }, "Ended")?;
    /// assert_eq!(serde_json::to_value(&start)?, serde_json::json!({"type": "Start", "to": "build/b1",
    ///     "messageType": "Ended"}));
    /// assert!(ContainerMsg::start(&Build { id: "b1".into() }, "").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn start<R: ActorRef>(target: &R, message_type: impl Into<String>) -> Result<Self, InvalidContainer> {
        let to = target.address().map_err(|e| InvalidContainer(e.to_string()))?;
        let message_type = message_type.into();
        check_message_type(&message_type)?;
        Ok(Self::Start { to, message_type })
    }
}

/// Refuses a message type the state decoder would refuse.
pub(crate) fn check_message_type(message_type: &str) -> Result<(), InvalidContainer> {
    let length = message_type.chars().count();
    if length == 0 || length > MAX_TYPE_LENGTH || message_type.contains('\0') {
        return Err(InvalidContainer(format!(
            "messageType must be 1 through {MAX_TYPE_LENGTH} characters without NUL, had {length}"
        )));
    }
    Ok(())
}

/// What is wanted of a container; it moves only forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Goal {
    /// Running until it ends.
    Run,
    /// Stopped, with its end recorded.
    Stop,
    /// Removed, after its end is recorded.
    Remove,
}

/// Where a container is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Nothing made yet, or made by a step whose commit was lost.
    #[default]
    New,
    /// Created and started.
    Started,
    /// Ended, its end recorded.
    Ended,
    /// Removed, after its end.
    Removed,
}

/// Where the end goes: the target and message type of the first `Start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndTarget {
    /// The target.
    pub to: ActorAddress,
    /// The message type, whose payload is a [`ContainerEnded`].
    #[schemars(length(min = 1, max = 200))]
    pub message_type: String,
}

/// How a container ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// The program exited by itself, with its code.
    Exited,
    /// The runtime ended it, or it could not be run: it did not start, ran out of memory, went missing, the runtime
    /// failed too often in a row, or its key, request or spec was refused.
    Failed,
    /// It ran past its deadline and was stopped.
    TimedOut,
    /// It was stopped on request, or never made.
    Stopped,
}

/// The end of a container: saved in the state and sent to the target in the commit that records it, before anything
/// removes the container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContainerEnd {
    /// How it ended.
    pub outcome: Outcome,
    /// The exit code, when the runtime gave one.
    #[serde(default)]
    pub code: Option<i64>,
    /// Why, in words: at most 1 KiB, without NUL.
    pub reason: String,
    /// The tail of its output, stdout and stderr together: at most the settings' last lines and bytes, without NUL;
    /// empty when nothing was made, or `(no logs: …)` when they could not be read.
    #[serde(default)]
    pub logs: String,
}

/// The payload of the end message a container actor sends: its key, as given, and its end. A host's message family
/// takes it as a variant such as `Ended(ContainerEnded)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContainerEnded {
    /// The key of the container actor.
    pub key: String,
    /// How it ended.
    pub end: ContainerEnd,
}

/// The state of a container actor: the host's typed request, never a spec, and the reconciliation's progress. Create it
/// with [`ContainerState::new`]; a decoded state is checked the same way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    rename_all = "camelCase",
    try_from = "RawState<R>",
    bound(serialize = "R: Serialize", deserialize = "R: DeserializeOwned")
)]
pub struct ContainerState<R> {
    /// The host's request, from which its job builds the spec at each create.
    pub request: R,
    /// What is wanted; none before the first `Start`, `Stop` or `Remove`.
    pub goal: Option<Goal>,
    /// Where the container is.
    pub phase: Phase,
    /// Where the end goes; none before the first `Start`.
    pub target: Option<EndTarget>,
    /// The store time after which a running container is stopped and ends `timedOut`; set at its start when the
    /// settings have a timeout.
    pub deadline: Option<Timestamp>,
    /// The end, once recorded.
    pub end: Option<ContainerEnd>,
    /// Why the end could not be sent to the target: its type is not registered or does not take the message.
    pub undelivered: Option<String>,
    /// The armed poll; only it acts.
    pub armed: Option<MessageId>,
    /// When the armed poll is due.
    pub due: Option<Timestamp>,
    /// Runtime errors in a row.
    pub errors: u32,
}

/// The wire form of a [`ContainerState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", bound(deserialize = "R: DeserializeOwned"))]
struct RawState<R> {
    /// The host's request, from which its job builds the spec at each create.
    request: R,
    /// Run, stop or remove; absent or null before the first `Start`, `Stop` or `Remove`, and set in any other phase
    /// than new.
    #[serde(default)]
    goal: Option<Goal>,
    /// New, started, ended or removed; absent: new.
    #[serde(default)]
    phase: Phase,
    /// Where the end goes; absent or null before the first `Start`.
    #[serde(default)]
    target: Option<EndTarget>,
    /// The deadline of a running container; absent or null without a timeout.
    #[serde(default)]
    deadline: Option<Timestamp>,
    /// The end; present exactly once ended or removed.
    #[serde(default)]
    end: Option<ContainerEnd>,
    /// Why the end could not be sent; absent or null when it was, or was not yet.
    #[serde(default)]
    undelivered: Option<String>,
    /// The id of the armed poll; absent or null when none is armed.
    #[serde(default)]
    armed: Option<MessageId>,
    /// When the armed poll is due; present exactly with `armed`.
    #[serde(default)]
    due: Option<Timestamp>,
    /// Runtime errors in a row; absent: 0.
    #[serde(default)]
    errors: u32,
}

impl<R> TryFrom<RawState<R>> for ContainerState<R> {
    type Error = InvalidContainer;

    fn try_from(raw: RawState<R>) -> Result<Self, InvalidContainer> {
        let state = Self {
            request: raw.request,
            goal: raw.goal,
            phase: raw.phase,
            target: raw.target,
            deadline: raw.deadline,
            end: raw.end,
            undelivered: raw.undelivered,
            armed: raw.armed,
            due: raw.due,
            errors: raw.errors,
        };
        state.check()?;
        Ok(state)
    }
}

impl<R> ContainerState<R> {
    /// A container of `request` with no goal, nothing made and no target: create it, then send it a `Start`.
    ///
    /// ```
    /// use cicada_containers::ContainerState;
    ///
    /// let state = ContainerState::new(serde_json::json!({"commit": "abc"}));
    /// assert_eq!(serde_json::to_value(&state)?, serde_json::json!({"request": {"commit": "abc"}, "goal": null,
    ///     "phase": "new", "target": null, "deadline": null, "end": null, "undelivered": null, "armed": null,
    ///     "due": null, "errors": 0}));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(request: R) -> Self {
        Self {
            request,
            goal: None,
            phase: Phase::New,
            target: None,
            deadline: None,
            end: None,
            undelivered: None,
            armed: None,
            due: None,
            errors: 0,
        }
    }

    /// The checks across fields, beyond what the schema says.
    fn check(&self) -> Result<(), InvalidContainer> {
        let ended = matches!(self.phase, Phase::Ended | Phase::Removed);
        if ended != self.end.is_some() {
            return Err(InvalidContainer("a container has an end exactly once it is ended or removed".to_owned()));
        }
        if self.phase != Phase::New && self.goal.is_none() {
            return Err(InvalidContainer("a container past new has a goal".to_owned()));
        }
        if self.armed.is_some() != self.due.is_some() {
            return Err(InvalidContainer("an armed poll has a due time, and only an armed poll".to_owned()));
        }
        if let Some(target) = &self.target {
            check_message_type(&target.message_type)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn decode(json: Value) -> Result<ContainerState<Value>, serde_json::Error> {
        serde_json::from_value(json)
    }

    #[test]
    fn a_minimal_state_decodes_as_a_new_one() {
        let state = decode(json!({"request": {"commit": "abc"}})).unwrap();
        assert_eq!(state, ContainerState::new(json!({"commit": "abc"})));
        assert!(decode(json!({})).is_err(), "the request is required");
    }

    #[test]
    fn an_inconsistent_state_is_refused() {
        let end = json!({"outcome": "stopped", "reason": "stopped"});
        for (state, why) in [
            (json!({"request": 1, "goal": "run", "phase": "ended"}), "has an end exactly"),
            (json!({"request": 1, "goal": "run", "end": end}), "has an end exactly"),
            (json!({"request": 1, "phase": "started"}), "has a goal"),
            (json!({"request": 1, "armed": "m1"}), "due time"),
            (json!({"request": 1, "due": "2026-01-01T00:00:00Z"}), "due time"),
            (json!({"request": 1, "target": {"to": "build/b1", "messageType": "E\u{0}"}}), "messageType"),
        ] {
            let decoded = decode(state.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {state}");
        }
        let removed = json!({"request": 1, "goal": "remove", "phase": "removed", "end": end});
        assert!(decode(removed).is_ok());
    }

    #[test]
    fn the_goal_moves_forward_in_declaration_order() {
        assert!(Goal::Run < Goal::Stop && Goal::Stop < Goal::Remove);
    }
}
