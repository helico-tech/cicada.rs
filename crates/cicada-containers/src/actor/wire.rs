//! What a container actor takes, keeps and sends: its messages, its state and its end.

use cicada::prelude::*;
use cicada::runtime::limits::MAX_TYPE_LENGTH;
use cicada::time::Timestamp;
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer, Serialize};

use crate::SkippedFile;

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
    /// Runs the container until it ends, and sends the end to `to`, with its progress first when `progressType` is
    /// given. The first `Start` sets the target; a later one changes nothing, unless the container has ended: then it
    /// sends the saved end to its own target at once, again for each such `Start`. While the outputs are collected
    /// the end is not saved yet: it goes to the first target when the collection finishes. A message type out of
    /// bounds is refused for good, and nothing is saved.
    Start {
        /// The target of the end.
        to: ActorAddress,
        /// The message type of the end, whose payload is `{"key": …, "end": …}`, a [`ContainerEnded`]; not empty, at
        /// most 200 characters, without NUL. The target's registry is checked only when the end is sent.
        #[schemars(length(min = 1, max = 200))]
        message_type: String,
        /// The message type of the container's progress, whose payload is a [`ContainerProgress`]; bounded as
        /// `messageType`. With it, the program finds its progress file, the job's `PROGRESS_FILE`, in
        /// `CICADA_PROGRESS_FILE`; absent or null: no progress is read.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(length(min = 1, max = 200))]
        progress_type: Option<String>,
    },
    /// Stops the container and ends it `stopped`, or `timedOut` past its deadline; one never started is never created.
    Stop,
    /// Stops the container if needed, records its end, then removes it. Send it before deleting the actor, or the
    /// container stays. With a store and an output retention, the actor deletes the stored outputs that long after the
    /// removal; deleting the actor before then leaves them in the store.
    Remove,
    /// Arms a fresh poll when one was lost: due when the armed one was, or at once when that passed. Does nothing with no
    /// work left; a removed container whose stored outputs wait for their deletion has work left.
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
        check_type("messageType", &message_type)?;
        Ok(Self::Start { to, message_type, progress_type: None })
    }

    /// A `Start` whose end goes to `target` as `message_type`, and its progress as `progress_type`. Fails as
    /// [`start`](Self::start) does, and for a progress type out of the same bounds.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Build { id: String }
    /// # impl ActorRef for Build { const TYPE: &'static str = "build"; type Message = BuildMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum BuildMsg { Ended(cicada_containers::ContainerEnded), Progress(cicada_containers::ContainerProgress) }
    /// use cicada_containers::ContainerMsg;
    ///
    /// let start = ContainerMsg::start_with_progress(&Build { id: "b1".into() }, "Ended", "Progress")?;
    /// assert_eq!(serde_json::to_value(&start)?, serde_json::json!({"type": "Start", "to": "build/b1",
    ///     "messageType": "Ended", "progressType": "Progress"}));
    /// assert!(ContainerMsg::start_with_progress(&Build { id: "b1".into() }, "Ended", "").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn start_with_progress<R: ActorRef>(
        target: &R,
        message_type: impl Into<String>,
        progress_type: impl Into<String>,
    ) -> Result<Self, InvalidContainer> {
        let progress_type = progress_type.into();
        check_type("progressType", &progress_type)?;
        let Self::Start { to, message_type, .. } = Self::start(target, message_type)? else {
            unreachable!("start builds a Start")
        };
        Ok(Self::Start { to, message_type, progress_type: Some(progress_type) })
    }
}

/// Refuses a message type, named `field` in the error, that the state decoder would refuse.
pub(crate) fn check_type(field: &str, message_type: &str) -> Result<(), InvalidContainer> {
    let length = message_type.chars().count();
    if length == 0 || length > MAX_TYPE_LENGTH || message_type.contains('\0') {
        return Err(InvalidContainer(format!(
            "{field} must be 1 through {MAX_TYPE_LENGTH} characters without NUL, had {length}"
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
    /// Ended, its end held as pending while its log and outputs go into the actor's
    /// [`OutputStore`](crate::OutputStore); its end is recorded and sent when that is done. Only with a store.
    Collecting,
    /// Ended, its end recorded.
    Ended,
    /// Removed, after its end.
    Removed,
}

/// Where the end and the progress go: the target and message types of the first `Start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndTarget {
    /// The target.
    pub to: ActorAddress,
    /// The message type, whose payload is a [`ContainerEnded`].
    #[schemars(length(min = 1, max = 200))]
    pub message_type: String,
    /// The message type of the progress, whose payload is a [`ContainerProgress`]; absent or null: no progress is read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 200))]
    pub progress_type: Option<String>,
}

/// How a container ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// The program exited by itself, with its code.
    Exited,
    /// The runtime ended it, or it could not be run: it did not start, ran out of memory, went missing, the runtime
    /// failed too often in a row, or its key, request, spec, start or inputs were refused. The end's
    /// [`kind`](ContainerEnd::kind) says which.
    Failed,
    /// It ran past its deadline and was stopped.
    TimedOut,
    /// It was stopped on request, or never made.
    Stopped,
}

/// Why a container ended `failed`, for a host that routes failures: set on every `failed` end since 0.21.0 and on no
/// other end. An end saved before has none, also when it failed, as no kind is guessed from its reason.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    /// It never ran: its key, spec, image, start or inputs were refused, or a runtime call it needs is unsupported.
    NotStarted,
    /// The runtime killed it for lack of memory: it reported `Failed` with the reason
    /// [`OUT_OF_MEMORY`](crate::OUT_OF_MEMORY).
    OutOfMemory,
    /// The runtime reported another failure of a started container, such as a dead one or an error of the daemon.
    RuntimeEnded,
    /// A started container was gone.
    Missing,
    /// The runtime failed the settings' maximum errors in a row.
    RuntimeErrors,
}

/// The end of a container: saved in the state and sent to the target in the commit that records it, before anything
/// removes the container. Decoding refuses a `kind` on an end that is not `failed`, in a state and in a
/// [`ContainerEnded`] alike.
///
/// Built as a literal, it names every field: `outputs: None` for an end without a store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ContainerEnd {
    /// How it ended.
    pub outcome: Outcome,
    /// The exit code, when the runtime gave one.
    #[serde(default)]
    pub code: Option<i64>,
    /// Why it failed, on a `failed` end only; none on an end saved before 0.21.0.
    #[serde(default)]
    pub kind: Option<FailureKind>,
    /// Why, in words: at most 1 KiB, without NUL.
    pub reason: String,
    /// The tail of its output, stdout and stderr together: at most the settings' last lines and bytes, without NUL;
    /// empty when nothing was made, or `(no logs: …)` when they could not be read.
    #[serde(default)]
    pub logs: String,
    /// What went into the actor's [`OutputStore`](crate::OutputStore) at the end: on every end of a container that ran,
    /// when the actor has a store; absent without a store, on an end the container did not run for (`notStarted`,
    /// `missing`, `runtimeErrors`) and on an end saved before 0.23.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<OutputsReport>,
}

/// What the container actor stored at a container's end, in [`ContainerEnd::outputs`]: the rest of its log, then the
/// files of its job's [`outputs`](crate::ContainerJob::outputs) folder within its limits. The counts are those of the
/// last attempt; all 0 without an outputs folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutputsReport {
    /// How many files were stored.
    pub stored: u32,
    /// Their bytes together.
    pub bytes: u64,
    /// How many entries were skipped: by [`copy_out`](crate::ContainerRuntime::copy_out)'s limits and link rule, or
    /// refused by the store.
    pub skipped: u32,
    /// The first 10 skipped entries, each with its path and reason (a size appears only in a reason); both without NUL
    /// and at most 1 KiB.
    #[schemars(length(max = 10))]
    pub first_skipped: Vec<SkippedFile>,
    /// Why the outputs could not be stored, as `(no outputs: …)`: the collection failed too often in a row, or a retry
    /// cannot help (a bad outputs path, a runtime or store without the call, or the container gone); none when it went
    /// through. What was stored before the error stays stored.
    #[serde(default)]
    pub error: Option<String>,
}

impl<'de> Deserialize<'de> for ContainerEnd {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// The wire form of a [`ContainerEnd`], unchecked.
        #[derive(Deserialize)]
        struct RawEnd {
            outcome: Outcome,
            #[serde(default)]
            code: Option<i64>,
            #[serde(default)]
            kind: Option<FailureKind>,
            reason: String,
            #[serde(default)]
            logs: String,
            #[serde(default)]
            outputs: Option<OutputsReport>,
        }

        let RawEnd { outcome, code, kind, reason, logs, outputs } = RawEnd::deserialize(deserializer)?;
        if kind.is_some() && outcome != Outcome::Failed {
            return Err(D::Error::custom("an end has a kind only when it is failed"));
        }
        Ok(Self { outcome, code, kind, reason, logs, outputs })
    }
}

impl OutputsReport {
    /// Nothing stored, and `error` as `(no outputs: …)`, bounded as a reason.
    pub(crate) fn failed(error: &str) -> Self {
        Self { error: Some(super::text::reason(&format!("(no outputs: {error})"))), ..Self::default() }
    }
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

/// The payload of a progress message a container actor sends when its `Start` named a `progressType`: its key, the
/// number of this progress, counting from 1, and the progress file's text as the program wrote it. A host's message
/// family takes it as a variant such as `Progress(ContainerProgress)`.
///
/// The actor reads the file at each poll of the running container and once in the turn that records an end for which
/// it ran, and sends its text only when it changed: a new `seq` per change. The last progress goes in the end's commit,
/// before the end; but a progress the owner fails to handle is retried after a backoff, so it can arrive after a later
/// progress or after the end. Drop a progress whose `seq` is not above the last one handled, and any after the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContainerProgress {
    /// The key of the container actor.
    pub key: String,
    /// The number of this progress: 1 for the first, one more for each change.
    pub seq: u64,
    /// The file's text: at most 64 KiB read, as UTF-8 with each invalid sequence and NUL replaced by U+FFFD; empty only
    /// after a progress that was not.
    pub text: String,
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
    /// The store time after which a running container is stopped and ends `timedOut`; set at its start when its job's
    /// `timeout` gives one, capped at the largest instant, and never moved.
    pub deadline: Option<Timestamp>,
    /// The end, once recorded.
    pub end: Option<ContainerEnd>,
    /// The end held while the container is collecting, recorded with its outputs when that is done.
    pub pending: Option<ContainerEnd>,
    /// Why the end could not be sent to the target: its type is not registered or does not take the message.
    pub undelivered: Option<String>,
    /// The armed poll; only it acts.
    pub armed: Option<MessageId>,
    /// When the armed poll is due.
    pub due: Option<Timestamp>,
    /// Runtime errors in a row.
    pub errors: u32,
    /// The `seq` of the last progress sent; 0 before the first.
    pub progress_seq: u64,
    /// The SHA-256 of the last progress text sent, in lower-case hex; none before the first.
    pub progress_hash: Option<String>,
    /// The store time after which the container's stored outputs are deleted from the actor's
    /// [`OutputStore`](crate::OutputStore): set by the removal when the actor has a store and an
    /// [`output_retention`](crate::ContainerSettings::output_retention), capped at the largest instant, never moved,
    /// and cleared once they are deleted. Only in the phase `Removed`.
    pub outputs_until: Option<Timestamp>,
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
    /// New, started, collecting, ended or removed; absent: new.
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
    /// The end held while the outputs are collected; present exactly while collecting.
    #[serde(default)]
    pending: Option<ContainerEnd>,
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
    /// The `seq` of the last progress sent; absent: 0, before the first.
    #[serde(default)]
    progress_seq: u64,
    /// The lower-case hex SHA-256 of the last progress text sent; absent or null exactly while `progressSeq` is 0.
    #[serde(default)]
    progress_hash: Option<String>,
    /// When the stored outputs of a removed container are deleted; absent or null when none wait, and set only when
    /// removed.
    #[serde(default)]
    outputs_until: Option<Timestamp>,
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
            pending: raw.pending,
            undelivered: raw.undelivered,
            armed: raw.armed,
            due: raw.due,
            errors: raw.errors,
            progress_seq: raw.progress_seq,
            progress_hash: raw.progress_hash,
            outputs_until: raw.outputs_until,
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
    ///     "phase": "new", "target": null, "deadline": null, "end": null, "pending": null, "undelivered": null,
    ///     "armed": null, "due": null, "errors": 0, "progressSeq": 0, "progressHash": null, "outputsUntil": null}));
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
            pending: None,
            undelivered: None,
            armed: None,
            due: None,
            errors: 0,
            progress_seq: 0,
            progress_hash: None,
            outputs_until: None,
        }
    }

    /// The checks across fields, beyond what the schema says.
    fn check(&self) -> Result<(), InvalidContainer> {
        let ended = matches!(self.phase, Phase::Ended | Phase::Removed);
        if ended != self.end.is_some() {
            return Err(InvalidContainer("a container has an end exactly once it is ended or removed".to_owned()));
        }
        if (self.phase == Phase::Collecting) != self.pending.is_some() {
            return Err(InvalidContainer("a container has a pending end exactly while it is collecting".to_owned()));
        }
        if self.phase != Phase::New && self.goal.is_none() {
            return Err(InvalidContainer("a container past new has a goal".to_owned()));
        }
        if self.armed.is_some() != self.due.is_some() {
            return Err(InvalidContainer("an armed poll has a due time, and only an armed poll".to_owned()));
        }
        if let Some(target) = &self.target {
            check_type("messageType", &target.message_type)?;
            if let Some(progress_type) = &target.progress_type {
                check_type("progressType", progress_type)?;
            }
        }
        if self.outputs_until.is_some() && self.phase != Phase::Removed {
            return Err(InvalidContainer("a container has outputs to delete only once it is removed".to_owned()));
        }
        if self.progress_hash.is_some() != (self.progress_seq > 0) {
            return Err(InvalidContainer("a progress hash is kept exactly once a progress was sent".to_owned()));
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
            (
                json!({"request": 1, "target": {"to": "build/b1", "messageType": "E", "progressType": ""}}),
                "progressType",
            ),
            (json!({"request": 1, "goal": "run", "phase": "collecting"}), "pending end exactly"),
            (json!({"request": 1, "goal": "run", "phase": "started", "pending": end}), "pending end exactly"),
            (json!({"request": 1, "goal": "run", "phase": "collecting", "pending": end, "end": end}), "an end exactly"),
            (json!({"request": 1, "progressSeq": 1}), "progress hash"),
            (json!({"request": 1, "progressHash": "ab"}), "progress hash"),
            (
                json!({"request": 1, "goal": "remove", "phase": "ended", "end": end, "outputsUntil": "2026-01-08T00:00:00Z"}),
                "only once it is removed",
            ),
        ] {
            let decoded = decode(state.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {state}");
        }
        let removed = json!({"request": 1, "goal": "remove", "phase": "removed", "end": end});
        assert_eq!(decode(removed.clone()).unwrap().outputs_until, None, "a state from 0.22 has none");
        let mut waiting = removed;
        waiting["outputsUntil"] = json!("2026-01-08T00:00:00Z");
        let decoded = decode(waiting.clone()).unwrap();
        assert!(decoded.outputs_until.is_some());
        assert_eq!(serde_json::to_value(&decoded).unwrap()["outputsUntil"], waiting["outputsUntil"]);
        let collecting = json!({"request": 1, "goal": "remove", "phase": "collecting", "pending": end});
        assert!(decode(collecting).is_ok());
    }

    #[test]
    fn a_failed_end_saved_before_kinds_decodes_without_one() {
        let end = json!({"outcome": "failed", "code": null, "reason": "the container went missing", "logs": ""});
        let state = json!({"request": 1, "goal": "run", "phase": "ended", "end": end});
        assert_eq!(decode(state).unwrap().end.unwrap().kind, None);
        let ended: ContainerEnded = serde_json::from_value(json!({"key": "j1", "end": end})).unwrap();
        assert_eq!((ended.end.outcome, ended.end.kind), (Outcome::Failed, None));
    }

    #[test]
    fn a_kind_on_an_end_that_is_not_failed_is_refused() {
        for outcome in ["exited", "timedOut", "stopped"] {
            let end = json!({"outcome": outcome, "kind": "missing", "reason": "x"});
            let state = json!({"request": 1, "goal": "stop", "phase": "ended", "end": end});
            let decoded = decode(state);
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains("only when it is failed")), "{decoded:?}");
            let ended = serde_json::from_value::<ContainerEnded>(json!({"key": "j1", "end": end}));
            assert!(ended.as_ref().is_err_and(|e| e.to_string().contains("only when it is failed")), "{ended:?}");
        }
        let failed = json!({"outcome": "failed", "kind": "outOfMemory", "reason": "out of memory"});
        let ended: ContainerEnded = serde_json::from_value(json!({"key": "j1", "end": failed})).unwrap();
        assert_eq!(ended.end.kind, Some(FailureKind::OutOfMemory));
    }

    #[test]
    fn an_end_without_outputs_decodes_and_keeps_none_on_the_wire() {
        let old = json!({"outcome": "exited", "code": 0, "kind": null, "reason": "exited with code 0", "logs": ""});
        let ended: ContainerEnded = serde_json::from_value(json!({"key": "j1", "end": old})).unwrap();
        assert_eq!(ended.end.outputs, None);
        assert_eq!(serde_json::to_value(&ended.end).unwrap(), old, "no `outputs` written without one");
        let report = json!({"stored": 1, "bytes": 3, "skipped": 1,
            "firstSkipped": [{"path": "/out/big", "reason": "70 bytes, more than the 3 left"}], "error": null});
        let mut with = old.clone();
        with["outputs"] = report.clone();
        let end: ContainerEnd = serde_json::from_value(with.clone()).unwrap();
        let outputs = end.outputs.clone().unwrap();
        assert_eq!((outputs.stored, outputs.bytes, outputs.skipped), (1, 3, 1));
        assert_eq!(outputs.first_skipped, [SkippedFile::new("/out/big", "70 bytes, more than the 3 left")]);
        assert_eq!(serde_json::to_value(&end).unwrap(), with);
    }

    #[test]
    fn the_failure_kinds_are_camel_case() {
        let kinds = [
            FailureKind::NotStarted,
            FailureKind::OutOfMemory,
            FailureKind::RuntimeEnded,
            FailureKind::Missing,
            FailureKind::RuntimeErrors,
        ];
        let names: Vec<Value> = kinds.iter().map(|kind| serde_json::to_value(kind).unwrap()).collect();
        assert_eq!(
            names,
            [
                json!("notStarted"),
                json!("outOfMemory"),
                json!("runtimeEnded"),
                json!("missing"),
                json!("runtimeErrors")
            ]
        );
    }

    #[test]
    fn the_goal_moves_forward_in_declaration_order() {
        assert!(Goal::Run < Goal::Stop && Goal::Stop < Goal::Remove);
    }
}
