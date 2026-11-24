//! The container actor (ADR 0057): a reconciled external resource. A message only records what should be true and
//! arms a poll; one armed poll at a time asks the runtime what is true, takes one step toward the goal, saves it and
//! arms the next. The container's name is the settings' prefix plus the actor's key, so a step that runs again finds
//! the container instead of making a second one. Runtime errors count in the state and back off; they never fail a
//! turn, as a failed poll would be lost.

mod text;
mod wire;

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, MAX_INSTANT, Timestamp};
use cicada::turn::TurnError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use wire::{
    ContainerEnd, ContainerEnded, ContainerMsg, ContainerState, EndTarget, Goal, InvalidContainer, Outcome, Phase,
};

use crate::{ContainerError, ContainerRuntime, ContainerSpec, ContainerStatus, ErrorKind};

/// The longest container name, prefix and key together.
const MAX_NAME_LENGTH: usize = 200;

/// A container job: a host-named actor type whose actors each run one container, built from a small typed request.
///
/// ```
/// use cicada::prelude::*;
/// use cicada_containers::{Container, ContainerJob, ContainerSpec};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// struct Test {
///     /// The commit to test.
///     commit: String,
/// }
///
/// /// No Clone and no Debug needed: a job holds what builds its specs, such as a secret store.
/// struct Tests;
///
/// impl ContainerJob for Tests {
///     const TYPE: &'static str = "ci.tests";
///     type Request = Test;
///
///     fn spec(&self, name: &str, test: &Test) -> Result<ContainerSpec, String> {
///         let mut spec = ContainerSpec::new(name, "rust:1");
///         spec.command = Some(vec!["cargo".into(), "test".into()]);
///         spec.env.insert("COMMIT".into(), test.commit.clone());
///         Ok(spec)
///     }
/// }
///
/// let container = Container::<Tests>::named("t1");
/// assert_eq!(container.address()?.as_str(), "ci.tests/t1");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait ContainerJob: Send + Sync + 'static {
    /// The type name of the actors, chosen by the host.
    const TYPE: &'static str;
    /// The request of one container, fixed at its create and kept in its state; its schema is part of the contract.
    type Request: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static;

    /// The spec of the container `name` for `request`, built at each create, so secrets never reach the store. The actor
    /// sets the spec's name to `name` whatever it returns. An `Err` ends the container `failed` with its text, and
    /// nothing is made. It is sync: load what it needs before the actor runs.
    fn spec(&self, name: &str, request: &Self::Request) -> Result<ContainerSpec, String>;
}

/// The container actor of the job `J` keyed `key`, `<J::TYPE>/<key>`: a typed ref whose one string is the key. It needs
/// no `Clone` or `Debug` of `J`.
#[derive(Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct Container<J> {
    /// The key: the end of the container's name. Never reuse it for another job, also after a delete, as a new actor
    /// adopts the old container and its end.
    pub key: String,
    #[serde(skip)]
    job: PhantomData<fn() -> J>,
}

impl<J> Container<J> {
    /// The container actor keyed `key`.
    pub fn named(key: impl Into<String>) -> Self {
        Self { key: key.into(), job: PhantomData }
    }
}

impl<J> Clone for Container<J> {
    fn clone(&self) -> Self {
        Self::named(self.key.clone())
    }
}

impl<J: ContainerJob> fmt::Debug for Container<J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.address() {
            Ok(address) => f.debug_tuple("Container").field(&address.as_str()).finish(),
            Err(_) => f.debug_tuple("Container").field(&self.key).finish(),
        }
    }
}

impl<J: ContainerJob> ActorRef for Container<J> {
    const TYPE: &'static str = J::TYPE;
    type Message = ContainerMsg;
}

/// The settings of a container actor type, the host's: changed without touching actors, though a saved deadline does
/// not move. Make them with [`ContainerSettings::new`] and set the other fields by name.
///
/// ```
/// use std::time::Duration;
/// use cicada_containers::ContainerSettings;
///
/// let mut settings = ContainerSettings::new("ci-")?;
/// settings.timeout = Some(Duration::from_secs(600));
/// assert_eq!(settings.poll_interval, Duration::from_secs(2));
/// assert!(ContainerSettings::new("-ci").is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSettings {
    /// The start of every container's name, before the key: a letter or digit first, then only `A-Za-z0-9_.-`.
    pub prefix: String,
    /// The time between polls of a running container, and the base of the backoff after an error; 2 s. Keep it
    /// positive.
    pub poll_interval: Duration,
    /// The longest wait between polls after errors; 60 s.
    pub max_backoff: Duration,
    /// The runtime errors in a row that end a container without an end `failed`; 10. A removal keeps trying.
    pub max_errors: u32,
    /// How long a container may run before it is stopped and ends `timedOut`; none.
    pub timeout: Option<Duration>,
    /// How long a stop waits before it kills the container; 10 s. It runs inside one turn.
    pub stop_grace: Duration,
    /// The most lines of output an end keeps; 200.
    pub log_lines: u32,
    /// The most bytes of output an end keeps, after the lines; 16 KiB.
    pub log_bytes: usize,
}

impl ContainerSettings {
    /// The defaults, with names that start with `prefix`. Fails unless it starts with a letter or a digit and holds only
    /// `A-Za-z0-9_.-`.
    pub fn new(prefix: impl Into<String>) -> Result<Self, InvalidContainer> {
        let prefix = prefix.into();
        if !valid_name(&prefix) {
            return Err(InvalidContainer(format!(
                "the prefix {prefix:?} must start with a letter or a digit and hold only A-Za-z0-9_.-"
            )));
        }
        Ok(Self {
            prefix,
            poll_interval: Duration::from_secs(2),
            max_backoff: Duration::from_secs(60),
            max_errors: 10,
            timeout: None,
            stop_grace: Duration::from_secs(10),
            log_lines: 200,
            log_bytes: 16 * 1024,
        })
    }

    /// The name of the container keyed `key`, or why there is none.
    fn name(&self, key: &str) -> Result<String, String> {
        let name = format!("{}{key}", self.prefix);
        if !valid_name(&name) || name.len() > MAX_NAME_LENGTH {
            return Err(format!(
                "the key {key:?} cannot name a container: {name:?} must start with a letter or a digit, hold only \
                 A-Za-z0-9_.- and have at most {MAX_NAME_LENGTH} characters"
            ));
        }
        Ok(name)
    }

    /// The wait before the next poll after `errors` errors in a row: the interval doubled per error, at most
    /// `max_backoff`.
    fn backoff(&self, errors: u32) -> Duration {
        1u32.checked_shl(errors)
            .and_then(|factor| self.poll_interval.checked_mul(factor))
            .map_or(self.max_backoff, |wait| wait.min(self.max_backoff))
    }
}

/// True for a name Docker takes: a letter or digit first, then only `A-Za-z0-9_.-`.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The retry policy of every poll: the ready-made actors' shared one, 7 attempts, exponential from 1 s to 32 s. The end
/// message has it too.
fn retry_policy() -> RetryPolicy {
    let backoff = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(32)).expect("a valid backoff");
    RetryPolicy::new(7, backoff).expect("a valid retry policy")
}

/// The container actor type of the job `J`, under `J::TYPE`, with its runtime and settings.
///
/// Each poll takes one step for its goal and phase: for run, a new container is built by the job's spec, created and
/// started, and a started one is inspected until it ends (started again while created, stopped at its deadline); for
/// stop or remove before the end, it is stopped and inspected, and ends `stopped`, or `timedOut` past its deadline; for
/// remove after the end, it is removed. `ImageNotFound` and `InvalidSpec` from `create` end it `failed` at once. Any
/// other runtime error counts in a row and delays the next poll by the interval doubled per error, up to the maximum;
/// at the maximum errors in a row a container without an end ends `failed`.
///
/// Every node that can claim these actors must reach the same daemon: pin them with a claim owner when only some do
/// (ADR 0052). A create's image pull and a stop's grace run inside one turn and its maximum duration.
pub struct ContainerActor<J> {
    job: J,
    runtime: Arc<dyn ContainerRuntime>,
    settings: ContainerSettings,
}

impl<J: ContainerJob> ContainerActor<J> {
    /// The actor type of `job`, on `runtime`, with `settings`.
    pub fn new(job: J, runtime: Arc<dyn ContainerRuntime>, settings: ContainerSettings) -> Self {
        Self { job, runtime, settings }
    }
}

type ContainerTurn<J> = Turn<ContainerActor<J>>;

impl<J: ContainerJob> Actor for ContainerActor<J> {
    type Ref = Container<J>;
    type State = ContainerState<J::Request>;

    async fn handle(&self, turn: &mut ContainerTurn<J>, message: ContainerMsg) -> HandlerResult {
        match message {
            ContainerMsg::Start { to, message_type } => {
                let target = EndTarget { to, message_type };
                let state = turn.state_mut();
                if state.target.is_none() {
                    state.target = Some(target.clone());
                }
                if let Some(end) = state.end.clone() {
                    deliver(turn, &target, end).await?;
                }
                ask(turn, Goal::Run)
            }
            ContainerMsg::Stop => ask(turn, Goal::Stop),
            ContainerMsg::Remove => ask(turn, Goal::Remove),
            ContainerMsg::Check => rearm(turn),
            ContainerMsg::Poll => {
                if turn.state().armed.as_ref() != Some(&turn.message().id) {
                    return Ok(());
                }
                let state = turn.state_mut();
                state.armed = None;
                state.due = None;
                self.step(turn).await
            }
        }
    }
}

/// True while a poll has something to do: a goal, not removed, and no end yet unless it is to be removed.
fn work_left<R>(state: &ContainerState<R>) -> bool {
    state.goal.is_some() && state.phase != Phase::Removed && (state.end.is_none() || state.goal == Some(Goal::Remove))
}

/// Moves the goal forward to `goal` and arms a poll at once, skipping a backoff; a goal asked again re-arms instead.
fn ask<J: ContainerJob>(turn: &mut ContainerTurn<J>, goal: Goal) -> HandlerResult {
    let state = turn.state_mut();
    if state.goal.is_some_and(|current| current >= goal) {
        return rearm(turn);
    }
    state.goal = Some(goal);
    if !work_left(state) {
        return Ok(());
    }
    let now = turn.current_time();
    arm(turn, now)
}

/// Replaces the armed poll with a fresh one due when it was, or at once when that passed or none is saved; nothing with
/// no work left.
fn rearm<J: ContainerJob>(turn: &mut ContainerTurn<J>) -> HandlerResult {
    if !work_left(turn.state()) {
        return Ok(());
    }
    let now = turn.current_time();
    let due = turn.state().due.filter(|due| *due > now).unwrap_or(now);
    arm(turn, due)
}

/// Cancels the armed poll and arms one due at `at`, with the shared retry policy.
fn arm<J: ContainerJob>(turn: &mut ContainerTurn<J>, at: Timestamp) -> HandlerResult {
    if let Some(old) = turn.state_mut().armed.take()
        && old != turn.message().id
    {
        turn.cancel(&old)?;
    }
    let mut options = SendOptions::new().at(at).retry(retry_policy());
    options.correlation_id = turn.message().correlation_id.clone();
    let id = turn.send_self_with(ContainerMsg::Poll, options).map_err(HandlerError::permanent_from)?;
    let state = turn.state_mut();
    state.armed = Some(id);
    state.due = Some(at);
    Ok(())
}

/// Arms the next poll `wait` from now.
fn arm_after<J: ContainerJob>(turn: &mut ContainerTurn<J>, wait: Duration) -> HandlerResult {
    let now = turn.current_time();
    let at = time::add(now, wait).ok().filter(|at| *at <= MAX_INSTANT).unwrap_or(MAX_INSTANT);
    arm(turn, at)
}

/// Sends `end` to `target`. A send the registry refuses is saved as undelivered, not an error, so a removal is never
/// blocked; a failed read of the registry fails the turn, to be tried again.
async fn deliver<J: ContainerJob>(turn: &mut ContainerTurn<J>, target: &EndTarget, end: ContainerEnd) -> HandlerResult {
    let payload = serde_json::to_value(ContainerEnded { key: turn.self_ref().key.clone(), end })?;
    let options = SendOptions::new().retry(retry_policy());
    match turn.send_raw(&target.to, &target.message_type, payload, options).await {
        Ok(_) => turn.state_mut().undelivered = None,
        Err(e @ TurnError::Store(_)) => return Err(e.into()),
        Err(e) => turn.state_mut().undelivered = Some(text::reason(&e.to_string())),
    }
    Ok(())
}

/// The status's exit code, when it has one.
fn exit_code(status: &ContainerStatus) -> Option<i64> {
    match status {
        ContainerStatus::Exited { code } => Some(*code),
        ContainerStatus::Failed { code, .. } => *code,
        _ => None,
    }
}

impl<J: ContainerJob> ContainerActor<J> {
    /// One step toward the goal, from the phase.
    async fn step(&self, turn: &mut ContainerTurn<J>) -> HandlerResult {
        let state = turn.state();
        let (Some(goal), phase) = (state.goal, state.phase) else {
            return Ok(());
        };
        if phase == Phase::Removed || (phase == Phase::Ended && goal != Goal::Remove) {
            return Ok(());
        }
        let name = match self.settings.name(&turn.self_ref().key) {
            Ok(name) => name,
            // No container can have this name: nothing to remove.
            Err(_) if phase == Phase::Ended => {
                turn.state_mut().phase = Phase::Removed;
                return Ok(());
            }
            Err(reason) => return self.end(turn, Outcome::Failed, None, reason, String::new()).await,
        };
        match (goal, phase) {
            (Goal::Run, Phase::New) => self.create(turn, &name).await,
            (Goal::Run, _) => self.follow(turn, &name).await,
            (_, Phase::Ended) => self.remove(turn, &name).await,
            _ => self.stop(turn, &name).await,
        }
    }

    /// Builds the spec, creates and starts the container, and sets the deadline.
    async fn create(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        let mut spec = match self.job.spec(name, &turn.state().request) {
            Ok(spec) => spec,
            Err(reason) => return self.end(turn, Outcome::Failed, None, reason, String::new()).await,
        };
        spec.name = name.to_owned();
        if let Err(e) = self.runtime.create(&spec).await {
            if matches!(e.kind, ErrorKind::ImageNotFound | ErrorKind::InvalidSpec) {
                return self.end(turn, Outcome::Failed, None, e.to_string(), String::new()).await;
            }
            return self.error(turn, e).await;
        }
        if let Err(e) = self.runtime.start(name).await {
            return self.error(turn, e).await;
        }
        let now = turn.current_time();
        let deadline = self.settings.timeout.and_then(|timeout| time::add(now, timeout).ok());
        let state = turn.state_mut();
        state.phase = Phase::Started;
        state.errors = 0;
        state.deadline = deadline.filter(|deadline| *deadline <= MAX_INSTANT);
        arm_after(turn, self.settings.poll_interval)
    }

    /// Inspects a started container: follows it while it runs, stops it at its deadline, and records its end.
    async fn follow(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        let status = match self.runtime.inspect(name).await {
            Ok(status) => status,
            Err(e) => return self.error(turn, e).await,
        };
        match status {
            ContainerStatus::Running | ContainerStatus::Created => {
                if status == ContainerStatus::Created
                    && let Err(e) = self.runtime.start(name).await
                {
                    return self.error(turn, e).await;
                }
                turn.state_mut().errors = 0;
                let now = turn.current_time();
                if turn.state().deadline.is_some_and(|deadline| now >= deadline) {
                    turn.state_mut().goal = Some(Goal::Stop);
                    return arm(turn, now);
                }
                arm_after(turn, self.settings.poll_interval)
            }
            ContainerStatus::Exited { code } => {
                let Some(logs) = self.logs(turn, name).await? else { return Ok(()) };
                self.end(turn, Outcome::Exited, Some(code), format!("exited with code {code}"), logs).await
            }
            ContainerStatus::Failed { code, reason } => {
                let Some(logs) = self.logs(turn, name).await? else { return Ok(()) };
                self.end(turn, Outcome::Failed, code, reason, logs).await
            }
            ContainerStatus::Missing => {
                self.end(turn, Outcome::Failed, None, "the container went missing".to_owned(), String::new()).await
            }
        }
    }

    /// Stops the container and records its end: `stopped`, or `timedOut` past its deadline; `stopped` without logs when
    /// it is missing, as nothing was made or it is gone.
    async fn stop(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        if let Err(e) = self.runtime.stop(name, self.settings.stop_grace).await {
            return self.error(turn, e).await;
        }
        let status = match self.runtime.inspect(name).await {
            Ok(status) => status,
            Err(e) => return self.error(turn, e).await,
        };
        if status == ContainerStatus::Missing {
            let reason = "stopped; no container was found".to_owned();
            return self.end(turn, Outcome::Stopped, None, reason, String::new()).await;
        }
        let Some(logs) = self.logs(turn, name).await? else { return Ok(()) };
        let now = turn.current_time();
        let (outcome, reason) = match turn.state().deadline {
            Some(deadline) if now >= deadline => {
                (Outcome::TimedOut, format!("ran past its deadline, {}", time::format_instant(deadline)))
            }
            _ => (Outcome::Stopped, "stopped".to_owned()),
        };
        self.end(turn, outcome, exit_code(&status), reason, logs).await
    }

    /// Removes an ended container.
    async fn remove(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        if let Err(e) = self.runtime.remove(name).await {
            return self.error(turn, e).await;
        }
        let state = turn.state_mut();
        state.phase = Phase::Removed;
        state.errors = 0;
        Ok(())
    }

    /// The container's logs as stored, or `None` when they could not be read and the next poll is armed. A failure
    /// counts like any runtime error; at the maximum in a row the end gets `(no logs: …)` instead.
    async fn logs(&self, turn: &mut ContainerTurn<J>, name: &str) -> Result<Option<String>, HandlerError> {
        let text = match self.runtime.logs(name, self.settings.log_lines).await {
            Ok(text) => text,
            Err(e) => {
                let state = turn.state_mut();
                state.errors = state.errors.saturating_add(1);
                if state.errors < self.settings.max_errors {
                    let wait = self.settings.backoff(state.errors);
                    arm_after(turn, wait)?;
                    return Ok(None);
                }
                format!("(no logs: {e})")
            }
        };
        Ok(Some(text::logs(&text, self.settings.log_lines, self.settings.log_bytes)))
    }

    /// Counts a runtime error and arms the next poll after the backoff; at the maximum in a row, a container without an
    /// end ends `failed`, while a removal keeps trying.
    async fn error(&self, turn: &mut ContainerTurn<J>, error: ContainerError) -> HandlerResult {
        let state = turn.state_mut();
        state.errors = state.errors.saturating_add(1);
        let errors = state.errors;
        if errors >= self.settings.max_errors && state.end.is_none() {
            let reason = format!("the runtime failed {errors} times in a row: {error}");
            return self.end(turn, Outcome::Failed, None, reason, String::new()).await;
        }
        arm_after(turn, self.settings.backoff(errors))
    }

    /// Records the end, sends it to the target in the same commit, and arms the removal when that is the goal.
    async fn end(
        &self,
        turn: &mut ContainerTurn<J>,
        outcome: Outcome,
        code: Option<i64>,
        reason: String,
        logs: String,
    ) -> HandlerResult {
        let end = ContainerEnd { outcome, code, reason: text::reason(&reason), logs };
        let state = turn.state_mut();
        state.end = Some(end.clone());
        state.phase = Phase::Ended;
        state.errors = 0;
        if let Some(target) = state.target.clone() {
            deliver(turn, &target, end).await?;
        }
        if turn.state().goal == Some(Goal::Remove) {
            let now = turn.current_time();
            return arm(turn, now);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefix_must_be_a_name() {
        for prefix in ["app-", "a", "9.x_y-"] {
            assert!(ContainerSettings::new(prefix).is_ok(), "{prefix}");
        }
        for prefix in ["", "-app", ".app", "_app", "app/", "app ", "äpp"] {
            let refused = ContainerSettings::new(prefix);
            assert!(refused.is_err_and(|e| e.to_string().contains("must start with")), "{prefix}");
        }
    }

    #[test]
    fn the_defaults_are_the_designs() {
        let settings = ContainerSettings::new("app-").unwrap();
        let second = Duration::from_secs(1);
        assert_eq!((settings.poll_interval, settings.max_backoff, settings.max_errors), (2 * second, 60 * second, 10));
        assert_eq!((settings.timeout, settings.stop_grace), (None, 10 * second));
        assert_eq!((settings.log_lines, settings.log_bytes), (200, 16 * 1024));
    }

    #[test]
    fn the_name_is_the_prefix_and_the_key_as_given() {
        let settings = ContainerSettings::new("app-").unwrap();
        assert_eq!(settings.name("j1").unwrap(), "app-j1");
        assert_eq!(settings.name("a.b_c-9").unwrap(), "app-a.b_c-9");
        for key in ["a/b", "a b", "a@b", "é", "a%2Fb"] {
            let refused = settings.name(key);
            assert!(refused.as_ref().is_err_and(|e| e.contains(&format!("{key:?}"))), "{key}: {refused:?}");
        }
        assert!(settings.name(&"k".repeat(MAX_NAME_LENGTH - 4)).is_ok(), "200 characters");
        assert!(settings.name(&"k".repeat(MAX_NAME_LENGTH - 3)).is_err(), "201 characters");
    }

    #[test]
    fn the_backoff_doubles_up_to_its_maximum() {
        let mut settings = ContainerSettings::new("app-").unwrap();
        settings.poll_interval = Duration::from_secs(2);
        settings.max_backoff = Duration::from_secs(60);
        let waits: Vec<u64> = (1..=6).map(|errors| settings.backoff(errors).as_secs()).collect();
        assert_eq!(waits, [4, 8, 16, 32, 60, 60]);
        assert_eq!(settings.backoff(40), settings.max_backoff, "no overflow");
        assert_eq!(settings.backoff(u32::MAX), settings.max_backoff);
    }
}
