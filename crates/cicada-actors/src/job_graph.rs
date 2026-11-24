//! The job graph (ADR 0059): one actor per run that holds a graph of jobs given as data, starts each job once every job
//! it needs succeeded, up to an in-flight limit, takes an explicit result per attempt, and reports the run's end.

use std::collections::{HashMap, HashSet};

use cicada::prelude::*;
use cicada::runtime::limits::MAX_TYPE_LENGTH;
use cicada::time::Timestamp;
use cicada::turn::TurnError;
use cicada::{Failed, Stopped};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::EncodeError;
use crate::supervisor::retry_policy;

/// The most jobs of a graph.
const MAX_JOBS: usize = 100;
/// The highest in-flight limit.
const MAX_IN_FLIGHT: u32 = 100;
/// The longest job name, in characters.
const MAX_NAME_LENGTH: usize = 64;
/// The largest input or output, in bytes of JSON.
const MAX_VALUE_BYTES: usize = 16 * 1024;
/// The deepest input or output, counted as the core counts a payload.
const MAX_VALUE_DEPTH: usize = 32;
/// The longest reason, in characters.
const MAX_REASON_LENGTH: usize = 2_000;
/// The most attempts a job keeps; the oldest is dropped.
const MAX_ATTEMPTS: usize = 20;

/// A job graph run, `cicada.job-graph/<run>`: one for each run of a graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobGraph {
    /// The name of the run: the key of its address.
    pub run: String,
}

impl JobGraph {
    /// The run named `run`.
    pub fn named(run: impl Into<String>) -> Self {
        Self { run: run.into() }
    }
}

impl ActorRef for JobGraph {
    const TYPE: &'static str = "cicada.job-graph";
    type Message = JobGraphMsg;
}

/// The messages of a [`JobGraph`] run. `Dispatch` is its own: it acts on one only when it sent it itself. A job name is
/// 1 through 64 ASCII letters, digits, `-`, `_` or `.`; a message with one out of bounds is refused for good.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum JobGraphMsg {
    /// Starts every ready job, in graph order, while fewer jobs run than the in-flight limit. Send it with the create,
    /// in one commit; once nothing more can start it does nothing, so a second one is harmless.
    Start,
    /// The result of one attempt of a job, from its worker or whoever the work was handed to: the sender is not
    /// checked. Taken only when the job is `running` and `attempt` is its current attempt: the attempt ends with the
    /// outcome and reason, the job takes the outcome and keeps `output`, and ready jobs start. Any other result is
    /// refused for good with the reason and the state unchanged, so the store keeps it as a failed message of the run.
    Result {
        /// The job.
        #[schemars(length(min = 1, max = 64))]
        job: String,
        /// The attempt it reports, from the [`JobStart`].
        attempt: u32,
        /// How the attempt ended.
        outcome: JobOutcome,
        /// What the job made, any JSON value of at most 16 KiB and 32 levels; absent or null: none.
        #[serde(default)]
        output: Option<Value>,
        /// Why it ended so, at most 2,000 characters; absent or null: none.
        #[serde(default)]
        #[schemars(length(max = 2000))]
        reason: Option<String>,
    },
    /// Sends the end report to the report target when the run has ended; on a running run it does nothing. The run
    /// sends it itself in the commit that ends it; anyone may send it again, for a report that was lost or whose
    /// target was gone, so a report target treats a repeated report for the same run as one.
    Report,
    /// Sends the job message of `attempt` when it is still the job's current attempt, not yet dispatched, while the
    /// job is running. Sent by the run to itself, with a retry policy.
    Dispatch {
        /// The job.
        #[schemars(length(min = 1, max = 64))]
        job: String,
        /// The attempt to dispatch.
        attempt: u32,
    },
    /// A message to the run itself or to a job's target ended `failed`: a failed `Dispatch` of a current attempt not yet
    /// dispatched ends that attempt and its job `failed`, as no worker got the job message; so does the failure of the
    /// job message of a running job's current attempt at its target. Any other failure is ignored. Only a notification
    /// that the store wrote counts.
    Failed(Failed),
    /// A job's target went away: every running job whose job message was sent to it ends `failed`. Only a notification
    /// that the store wrote counts.
    Stopped(Stopped),
}

impl JobGraphMsg {
    /// A `Result` that reports the attempt of `start` `succeeded` with `output`; `()` or `null` is no output. Fails for
    /// an output that does not serialize to JSON. Its bounds are checked by the run.
    ///
    /// ```
    /// use cicada_actors::{JobGraphMsg, JobStart};
    ///
    /// let start = JobStart { run: "run-7".into(), job: "build".into(), attempt: 1, input: serde_json::json!({}) };
    /// let result = JobGraphMsg::succeeded(&start, serde_json::json!({"artifact": "app.tar"}))?;
    /// assert_eq!(serde_json::to_value(&result)?, serde_json::json!({"type": "Result", "job": "build", "attempt": 1,
    ///     "outcome": "succeeded", "output": {"artifact": "app.tar"}, "reason": null}));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn succeeded(start: &JobStart, output: impl Serialize) -> Result<Self, EncodeError> {
        let output = serde_json::to_value(output).map_err(|e| EncodeError(e.to_string()))?;
        Ok(Self::Result {
            job: start.job.clone(),
            attempt: start.attempt,
            outcome: JobOutcome::Succeeded,
            output: (!output.is_null()).then_some(output),
            reason: None,
        })
    }

    /// A `Result` that reports the attempt of `start` `failed` with `reason`, cut to 2,000 characters.
    ///
    /// ```
    /// use cicada_actors::{JobGraphMsg, JobStart};
    ///
    /// let start = JobStart { run: "run-7".into(), job: "test".into(), attempt: 1, input: serde_json::json!({}) };
    /// let result = JobGraphMsg::failed(&start, "3 tests failed");
    /// assert_eq!(serde_json::to_value(&result)?, serde_json::json!({"type": "Result", "job": "test", "attempt": 1,
    ///     "outcome": "failed", "output": null, "reason": "3 tests failed"}));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn failed(start: &JobStart, reason: impl Into<String>) -> Self {
        Self::Result {
            job: start.job.clone(),
            attempt: start.attempt,
            outcome: JobOutcome::Failed,
            output: None,
            reason: Some(cut(reason.into())),
        }
    }
}

/// The status of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum JobGraphStatus {
    /// A job runs or can start.
    Running,
    /// Ended: every job succeeded.
    Succeeded,
    /// Ended: a job failed.
    Failed,
    /// Ended: a job was cancelled, none failed; not set by this release.
    Cancelled,
}

/// The status of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    /// Not started: some job it needs has not succeeded, or no room was free.
    Waiting,
    /// Its current attempt is open: dispatched or about to be.
    Running,
    /// Its last attempt succeeded.
    Succeeded,
    /// Its last attempt failed; the jobs that need it are skipped.
    Failed,
    /// Never run, without an attempt: a job it needs, directly or through other jobs, failed, was skipped or cancelled.
    Skipped,
    /// Cancelled; not set by this release.
    Cancelled,
}

/// How an attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum JobOutcome {
    /// The job did its work.
    Succeeded,
    /// The job did not do its work.
    Failed,
}

impl From<JobOutcome> for JobStatus {
    fn from(outcome: JobOutcome) -> Self {
        match outcome {
            JobOutcome::Succeeded => JobStatus::Succeeded,
            JobOutcome::Failed => JobStatus::Failed,
        }
    }
}

/// A job of a graph, as given at the create: a message to a target, sent once every job it needs succeeded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// Its name: 1 through 64 ASCII letters, digits, `-`, `_` or `.`, unique in the graph.
    pub name: String,
    /// The jobs that must succeed before it starts, each named once.
    pub needs: Vec<String>,
    /// The target of its job message: any actor, also of another type than the other jobs'.
    pub to: ActorAddress,
    /// The message type of its job message, whose payload is a [`JobStart`].
    pub message_type: String,
    /// Its input, passed in the [`JobStart`]: any JSON value of at most 16 KiB and 32 levels.
    pub input: Value,
}

impl Job {
    /// The job `name`, which sends `message_type` with a [`JobStart`] that carries `input` to `target`, and needs no
    /// other job. Fails for a ref without an address and an input that does not serialize to JSON; the other bounds are
    /// checked by [`JobGraphState::new`]. Whether the target's type takes the message type is checked at the dispatch.
    pub fn new<R: ActorRef>(
        name: impl Into<String>,
        target: &R,
        message_type: impl Into<String>,
        input: impl Serialize,
    ) -> Result<Self, InvalidJobGraph> {
        let to = target.address().map_err(|e| InvalidJobGraph(e.to_string()))?;
        let input = serde_json::to_value(input).map_err(|e| InvalidJobGraph(e.to_string()))?;
        Ok(Self { name: name.into(), needs: Vec::new(), to, message_type: message_type.into(), input })
    }

    /// This job, needing `needs`.
    pub fn with_needs(mut self, needs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.needs = needs.into_iter().map(Into::into).collect();
        self
    }
}

/// One attempt of a job: the last one of a job is its current attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobAttempt {
    /// Its number, per job from 1.
    #[schemars(range(min = 1))]
    pub attempt: u32,
    /// The store time of the commit that sent its job message; null until then, and for good when that failed or when
    /// a `Result` ended the attempt before its `Dispatch` ran (the sender of a result is not checked).
    pub dispatched_at: Option<Timestamp>,
    /// The store time it ended; null while open.
    pub ended_at: Option<Timestamp>,
    /// How it ended; set exactly when it ended.
    pub outcome: Option<JobOutcome>,
    /// Why it ended so, at most 2,000 characters; only on an ended attempt.
    #[schemars(length(max = 2000))]
    pub reason: Option<String>,
}

/// A job of a run: its definition, its status, its output and its attempts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobEntry {
    /// Its name: 1 through 64 ASCII letters, digits, `-`, `_` or `.`, unique in the graph.
    #[schemars(length(min = 1, max = 64))]
    pub name: String,
    /// The jobs that must succeed before it starts, each a job of the graph other than itself, named once.
    pub needs: Vec<String>,
    /// The target of its job message.
    pub to: ActorAddress,
    /// The message type of its job message, whose payload is a [`JobStart`]; not empty, at most 200 characters,
    /// without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub message_type: String,
    /// Its input: any JSON value of at most 16 KiB and 32 levels.
    pub input: Value,
    /// Its status, matching its last attempt; absent: `waiting`.
    #[serde(default = "waiting")]
    pub status: JobStatus,
    /// The output of its last result; absent or null: none.
    #[serde(default)]
    pub output: Option<Value>,
    /// Its attempts, oldest first, at most 20, numbered consecutively; the last is the current one and the only one
    /// that may be open. Absent: none.
    #[serde(default)]
    pub attempts: Vec<JobAttempt>,
}

fn waiting() -> JobStatus {
    JobStatus::Waiting
}

impl From<Job> for JobEntry {
    fn from(job: Job) -> Self {
        Self {
            name: job.name,
            needs: job.needs,
            to: job.to,
            message_type: job.message_type,
            input: job.input,
            status: JobStatus::Waiting,
            output: None,
            attempts: Vec::new(),
        }
    }
}

/// The payload of a job message, sent as the job's message type to its target. A worker's family takes it as a variant
/// such as `Build(JobStart)`, and answers with [`JobGraphMsg::succeeded`] or [`JobGraphMsg::failed`] to
/// `JobGraph::named(run)`.
///
/// On the wire: `{"run":"run-7","job":"build","attempt":1,"input":{"ref":"main"}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobStart {
    /// The run: the key of its address.
    pub run: String,
    /// The job.
    pub job: String,
    /// The attempt: name it in the result.
    pub attempt: u32,
    /// The job's input.
    pub input: Value,
}

/// One job in the end report of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobReport {
    /// The job.
    pub name: String,
    /// Its status at the end.
    pub status: JobStatus,
    /// The number of its last attempt; null when it never started.
    pub attempt: Option<u32>,
    /// The output of its last result; null: none.
    pub output: Option<Value>,
    /// The reason of its last attempt; null: none.
    pub reason: Option<String>,
}

/// The end report of a run, sent as its report type to its report target. A host's family takes it as a variant such
/// as `GraphEnded(JobGraphEnd)`. It can arrive more than once: treat a repeat for the same run as one.
///
/// On the wire: `{"run":"run-7","status":"succeeded","jobs":[{"name":"build","status":"succeeded","attempt":1,
/// "output":null,"reason":null}]}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobGraphEnd {
    /// The run: the key of its address.
    pub run: String,
    /// How it ended.
    pub status: JobGraphStatus,
    /// Each job, in graph order.
    pub jobs: Vec<JobReport>,
}

/// A graph, a state or a message of a run out of its bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidJobGraph(String);

/// The state of a [`JobGraph`] run: its graph, in-flight limit and report target, given at its create, its status, and
/// per job its status, output and attempts. Build it with `JobGraphState::new`; a decoded state is checked the same
/// way, so a bad graph never becomes a run.
///
/// On the wire: `{"maxInFlight":2,"reportTo":"ci/run-7","reportType":"GraphEnded","status":"running","endedAt":null,
/// "jobs":[{"name":"build","needs":[],"to":"builder/run-7","messageType":"Build","input":{},"status":"waiting",
/// "output":null,"attempts":[]}]}`; `status`, `endedAt` and a job's `status`, `output` and `attempts` may be left out
/// of a create.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct JobGraphState {
    /// The most jobs that run at once, dispatched or not.
    pub max_in_flight: u32,
    /// The target of the end report.
    pub report_to: ActorAddress,
    /// The message type of the end report, whose payload is a [`JobGraphEnd`].
    pub report_type: String,
    /// The status of the run.
    pub status: JobGraphStatus,
    /// The store time it ended; set exactly when it ended.
    pub ended_at: Option<Timestamp>,
    /// The jobs, in graph order: the order ready jobs start in.
    pub jobs: Vec<JobEntry>,
}

/// The wire form of a [`JobGraphState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RawState {
    /// The most jobs that run at once, dispatched or not; 1 through 100.
    #[schemars(range(min = 1, max = 100))]
    max_in_flight: u32,
    /// The target of the end report.
    report_to: ActorAddress,
    /// The message type of the end report, whose payload is a `JobGraphEnd`; not empty, at most 200 characters,
    /// without NUL.
    #[schemars(length(min = 1, max = 200))]
    report_type: String,
    /// The status of the run: ended exactly when no job runs and none can start, then `succeeded` when every job
    /// succeeded, else `failed` when a job failed, else `cancelled`. Absent: `running`.
    #[serde(default = "running")]
    status: JobGraphStatus,
    /// The store time it ended; set exactly when it ended. Absent: null.
    #[serde(default)]
    ended_at: Option<Timestamp>,
    /// The jobs, in graph order, 1 through 100, without a cycle; at most `maxInFlight` run, a started job's needs all
    /// succeeded, and a job is skipped, without an attempt, exactly when it did not start and a job it needs failed,
    /// was skipped or cancelled.
    #[schemars(length(min = 1, max = 100))]
    jobs: Vec<JobEntry>,
}

fn running() -> JobGraphStatus {
    JobGraphStatus::Running
}

impl TryFrom<RawState> for JobGraphState {
    type Error = InvalidJobGraph;

    fn try_from(raw: RawState) -> Result<Self, InvalidJobGraph> {
        let state = Self {
            max_in_flight: raw.max_in_flight,
            report_to: raw.report_to,
            report_type: raw.report_type,
            status: raw.status,
            ended_at: raw.ended_at,
            jobs: raw.jobs,
        };
        state.check()?;
        Ok(state)
    }
}

impl JobGraphState {
    /// A run of `jobs` that runs at most `max_in_flight` of them at once and reports its end to `report_to` as
    /// `report_type`; running, with every job waiting. Fails for a ref without an address, a cycle, a need that names no
    /// job of the graph, is repeated or names the job itself, a repeated job name, no jobs or more than 100, a name that
    /// is not 1 through 64 ASCII letters, digits, `-`, `_` or `.`, a message or report type that is empty, longer than
    /// 200 characters or holds a NUL, an input over 16 KiB as JSON or deeper than 32 levels, and an in-flight limit
    /// outside 1 through 100.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Ci { run: String }
    /// # impl ActorRef for Ci { const TYPE: &'static str = "ci"; type Message = CiMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum CiMsg { Build(cicada_actors::JobStart), GraphEnded(cicada_actors::JobGraphEnd) }
    /// use cicada_actors::{Job, JobGraphState};
    ///
    /// let ci = Ci { run: "run-7".into() };
    /// let jobs = vec![
    ///     Job::new("build", &ci, "Build", serde_json::json!({"ref": "main"}))?,
    ///     Job::new("test", &ci, "Build", serde_json::json!({"suite": "all"}))?.with_needs(["build"]),
    /// ];
    /// let state = JobGraphState::new(jobs, 2, &ci, "GraphEnded")?;
    /// assert_eq!(serde_json::to_value(&state)?["jobs"][1]["needs"], serde_json::json!(["build"]));
    /// let cycle = vec![Job::new("a", &ci, "Build", ())?.with_needs(["a"])];
    /// assert!(JobGraphState::new(cycle, 2, &ci, "GraphEnded").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new<R: ActorRef>(
        jobs: Vec<Job>,
        max_in_flight: u32,
        report_to: &R,
        report_type: impl Into<String>,
    ) -> Result<Self, InvalidJobGraph> {
        let report_to = report_to.address().map_err(|e| InvalidJobGraph(e.to_string()))?;
        let state = Self {
            max_in_flight,
            report_to,
            report_type: report_type.into(),
            status: JobGraphStatus::Running,
            ended_at: None,
            jobs: jobs.into_iter().map(JobEntry::from).collect(),
        };
        state.check()?;
        Ok(state)
    }

    fn check(&self) -> Result<(), InvalidJobGraph> {
        if !(1..=MAX_IN_FLIGHT).contains(&self.max_in_flight) {
            return Err(invalid(format!("maxInFlight must be 1 through {MAX_IN_FLIGHT}, was {}", self.max_in_flight)));
        }
        check_type("reportType", &self.report_type)?;
        if self.jobs.is_empty() || self.jobs.len() > MAX_JOBS {
            return Err(invalid(format!("a graph has 1 through {MAX_JOBS} jobs, had {}", self.jobs.len())));
        }
        let mut index = HashMap::new();
        for (i, job) in self.jobs.iter().enumerate() {
            check_name(&job.name)?;
            if index.insert(job.name.as_str(), i).is_some() {
                return Err(invalid(format!("the job '{}' is in the graph twice", job.name)));
            }
        }
        for job in &self.jobs {
            check_type("messageType", &job.message_type)
                .map_err(|e| invalid(format!("the job '{}': {e}", job.name)))?;
            check_value("input", &job.input).map_err(|e| invalid(format!("the job '{}': {e}", job.name)))?;
            let mut seen = HashSet::new();
            for need in &job.needs {
                if need == &job.name {
                    return Err(invalid(format!("the job '{}' needs itself", job.name)));
                }
                if !index.contains_key(need.as_str()) {
                    return Err(invalid(format!(
                        "the job '{}' needs '{need}', which is no job of the graph",
                        job.name
                    )));
                }
                if !seen.insert(need.as_str()) {
                    return Err(invalid(format!("the job '{}' needs '{need}' twice", job.name)));
                }
            }
        }
        self.check_acyclic(&index)?;
        for job in &self.jobs {
            check_job(job)?;
        }
        self.check_skipped()?;
        self.check_progress()
    }

    /// Kahn's algorithm: takes jobs whose needs are all taken until none is left. A job left over is in a cycle or
    /// needs one; following needs that are left over from it for as many steps as there are jobs ends inside a cycle.
    fn check_acyclic(&self, index: &HashMap<&str, usize>) -> Result<(), InvalidJobGraph> {
        let mut pending: Vec<usize> = self.jobs.iter().map(|j| j.needs.len()).collect();
        let mut dependents = vec![Vec::new(); self.jobs.len()];
        for (i, job) in self.jobs.iter().enumerate() {
            for need in &job.needs {
                dependents[index[need.as_str()]].push(i);
            }
        }
        let mut ready: Vec<usize> = (0..self.jobs.len()).filter(|&i| pending[i] == 0).collect();
        let mut taken = 0;
        while let Some(i) = ready.pop() {
            taken += 1;
            for &d in &dependents[i] {
                pending[d] -= 1;
                if pending[d] == 0 {
                    ready.push(d);
                }
            }
        }
        let Some(mut at) = (0..self.jobs.len()).find(|&i| pending[i] > 0) else {
            return Ok(());
        };
        debug_assert!(taken < self.jobs.len());
        for _ in 0..self.jobs.len() {
            let left = self.jobs[at].needs.iter().map(|n| index[n.as_str()]).find(|&j| pending[j] > 0);
            at = left.expect("a job left over needs a job left over");
        }
        Err(invalid(format!("the graph has a cycle through the job '{}'", self.jobs[at].name)))
    }

    /// Checks that started jobs had their needs, the in-flight limit, and that the run is ended exactly when no job
    /// runs and none can start, with a status that follows its jobs.
    fn check_progress(&self) -> Result<(), InvalidJobGraph> {
        let statuses = self.statuses();
        let mut running = 0;
        let mut ready = 0;
        for job in &self.jobs {
            let had_needs = job.needs.iter().all(|n| statuses[n.as_str()] == JobStatus::Succeeded);
            if !job.attempts.is_empty() && !had_needs {
                return Err(invalid(format!("the job '{}' started before every job it needs succeeded", job.name)));
            }
            running += usize::from(job.status == JobStatus::Running);
            ready += usize::from(job.status == JobStatus::Waiting && had_needs);
        }
        if running > self.max_in_flight as usize {
            return Err(invalid(format!("at most maxInFlight ({}) jobs run, had {running}", self.max_in_flight)));
        }
        let ended = self.status != JobGraphStatus::Running;
        if ended != (running == 0 && ready == 0) {
            return Err(invalid(format!(
                "a run is ended exactly when no job runs and none can start: it is {}, with {running} running and \
                 {ready} ready",
                name_of(self.status)
            )));
        }
        if ended != self.ended_at.is_some() {
            return Err(invalid("endedAt is set exactly when the run ended"));
        }
        let end = self.end_status();
        if ended && self.status != end {
            return Err(invalid(format!(
                "an ended run is succeeded when every job succeeded, else failed when a job failed, else cancelled: \
                 it is {}, not {}",
                name_of(self.status),
                name_of(end)
            )));
        }
        Ok(())
    }

    /// Checks that a skipped job has no attempt and a need that failed, was skipped or cancelled, and that no waiting
    /// job has such a need.
    fn check_skipped(&self) -> Result<(), InvalidJobGraph> {
        let statuses = self.statuses();
        for job in &self.jobs {
            let blocked = job.needs.iter().any(|n| blocks(statuses[n.as_str()]));
            match job.status {
                JobStatus::Skipped if !job.attempts.is_empty() => {
                    return Err(invalid(format!("the skipped job '{}' has an attempt", job.name)));
                }
                JobStatus::Skipped if !blocked => {
                    return Err(invalid(format!(
                        "the job '{}' is skipped, but no job it needs failed, was skipped or cancelled",
                        job.name
                    )));
                }
                JobStatus::Waiting if blocked => {
                    return Err(invalid(format!(
                        "the job '{}' is waiting, but a job it needs failed, was skipped or cancelled",
                        job.name
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The waiting jobs that need a job that failed, was skipped or cancelled, directly or through other waiting jobs:
    /// the jobs [`Self::skip_blocked`] skips. A loop until nothing changes, at most one pass per job, no recursion.
    fn blocked(&self) -> Vec<usize> {
        let mut statuses = self.statuses();
        let mut blocked = Vec::new();
        loop {
            let mut more = false;
            for (i, job) in self.jobs.iter().enumerate() {
                let name = job.name.as_str();
                if statuses[name] == JobStatus::Waiting && job.needs.iter().any(|n| blocks(statuses[n.as_str()])) {
                    statuses.insert(name, JobStatus::Skipped);
                    blocked.push(i);
                    more = true;
                }
            }
            if !more {
                return blocked;
            }
        }
    }

    /// Skips every waiting job that needs a job that failed, was skipped or cancelled, directly or through others. A
    /// skipped job gets no attempt.
    fn skip_blocked(&mut self) {
        for i in self.blocked() {
            self.jobs[i].status = JobStatus::Skipped;
        }
    }

    /// The status of the run once it ended: `succeeded` when every job succeeded, else `failed` when a job failed,
    /// else `cancelled`.
    fn end_status(&self) -> JobGraphStatus {
        if self.jobs.iter().all(|j| j.status == JobStatus::Succeeded) {
            JobGraphStatus::Succeeded
        } else if self.jobs.iter().any(|j| j.status == JobStatus::Failed) {
            JobGraphStatus::Failed
        } else {
            JobGraphStatus::Cancelled
        }
    }

    fn statuses(&self) -> HashMap<&str, JobStatus> {
        self.jobs.iter().map(|j| (j.name.as_str(), j.status)).collect()
    }

    fn position(&self, job: &str) -> Option<usize> {
        self.jobs.iter().position(|j| j.name == job)
    }
}

/// Checks the attempts of a job and that its status matches its last one.
fn check_job(job: &JobEntry) -> Result<(), InvalidJobGraph> {
    let name = &job.name;
    if let Some(output) = &job.output {
        check_value("output", output).map_err(|e| invalid(format!("the job '{name}': {e}")))?;
    }
    if job.attempts.len() > MAX_ATTEMPTS {
        return Err(invalid(format!(
            "the job '{name}' keeps at most {MAX_ATTEMPTS} attempts, had {}",
            job.attempts.len()
        )));
    }
    if job.attempts.first().is_some_and(|a| a.attempt == 0) {
        return Err(invalid(format!("the attempts of the job '{name}' are numbered from 1")));
    }
    if job.attempts.windows(2).any(|w| w[0].attempt.checked_add(1) != Some(w[1].attempt)) {
        return Err(invalid(format!("the attempt numbers of the job '{name}' must be consecutive")));
    }
    let last = job.attempts.len().checked_sub(1);
    for (i, attempt) in job.attempts.iter().enumerate() {
        let open = attempt.ended_at.is_none();
        if open && Some(i) != last {
            return Err(invalid(format!("only the last attempt of the job '{name}' may be open")));
        }
        if open == attempt.outcome.is_some() {
            return Err(invalid(format!("an attempt of the job '{name}' has an outcome exactly when it ended")));
        }
        if let Some(reason) = &attempt.reason {
            if open {
                return Err(invalid(format!("an open attempt of the job '{name}' has no reason")));
            }
            check_reason(reason)?;
        }
    }
    let last = job.attempts.last();
    let open = last.is_some_and(|a| a.ended_at.is_none());
    let matches = match job.status {
        JobStatus::Waiting => last.is_none(),
        JobStatus::Running => open,
        JobStatus::Succeeded => last.is_some_and(|a| a.outcome == Some(JobOutcome::Succeeded)),
        JobStatus::Failed => last.is_some_and(|a| a.outcome == Some(JobOutcome::Failed)),
        JobStatus::Skipped | JobStatus::Cancelled => !open,
    };
    if !matches {
        return Err(invalid(format!(
            "the status {} of the job '{name}' does not match its last attempt",
            name_of(job.status)
        )));
    }
    Ok(())
}

/// True for a status that keeps the jobs that need it from ever starting: `failed`, `skipped` or `cancelled`.
fn blocks(status: JobStatus) -> bool {
    matches!(status, JobStatus::Failed | JobStatus::Skipped | JobStatus::Cancelled)
}

fn invalid(reason: impl Into<String>) -> InvalidJobGraph {
    InvalidJobGraph(reason.into())
}

/// The wire name of a status.
fn name_of(status: impl Serialize) -> String {
    serde_json::to_value(status).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn check_name(name: &str) -> Result<(), InvalidJobGraph> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if name.is_empty() || name.len() > MAX_NAME_LENGTH || !name.chars().all(allowed) {
        return Err(invalid(format!(
            "a job name is 1 through {MAX_NAME_LENGTH} ASCII letters, digits, '-', '_' or '.', was {name:?}"
        )));
    }
    Ok(())
}

fn check_type(field: &str, message_type: &str) -> Result<(), InvalidJobGraph> {
    let length = message_type.chars().count();
    if length == 0 || length > MAX_TYPE_LENGTH || message_type.contains('\0') {
        return Err(invalid(format!(
            "{field} must be 1 through {MAX_TYPE_LENGTH} characters without NUL, had {length}"
        )));
    }
    Ok(())
}

fn check_reason(reason: &str) -> Result<(), InvalidJobGraph> {
    let length = reason.chars().count();
    if length > MAX_REASON_LENGTH {
        return Err(invalid(format!("a reason is at most {MAX_REASON_LENGTH} characters, had {length}")));
    }
    Ok(())
}

/// Checks that `value` is at most 16 KiB as JSON and 32 levels deep.
fn check_value(field: &str, value: &Value) -> Result<(), InvalidJobGraph> {
    let bytes = serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))?.len();
    if bytes > MAX_VALUE_BYTES {
        return Err(invalid(format!("{field} is at most {MAX_VALUE_BYTES} bytes of JSON, had {bytes}")));
    }
    if depth(value) > MAX_VALUE_DEPTH {
        return Err(invalid(format!("{field} is at most {MAX_VALUE_DEPTH} levels deep")));
    }
    Ok(())
}

/// The arrays and objects inside each other in `value`, as the core counts them: a scalar is 0. Values come from JSON
/// that a parser read, so the recursion is bounded by its limit.
fn depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
        Value::Object(fields) => 1 + fields.values().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// `reason` cut to 2,000 characters.
fn cut(reason: String) -> String {
    match reason.char_indices().nth(MAX_REASON_LENGTH) {
        Some((end, _)) => reason[..end].to_owned(),
        None => reason,
    }
}

/// The job graph type. It has no settings: the graph, the in-flight limit and the report target are in the state of
/// each run, given at its create; a create of a run that exists changes nothing. It never stops itself.
///
/// After `Start` and every change of a job's status (a taken `Result`, a failed dispatch, a failure seen by the watch on
/// a target), in that commit, the run skips, starts and checks its end. It skips every waiting job that needs a failed,
/// skipped or cancelled job, directly or through others: it becomes `skipped`, without an attempt. It starts every ready
/// job (waiting, with every job it needs succeeded) in graph order while fewer than `maxInFlight` run. Starting a job
/// marks it `running` with a new attempt in that commit, which also sends the run a `Dispatch` with the crate's shared
/// retry policy and watches the run itself. That `Dispatch`, while the job is running, sends the job message, a
/// [`JobStart`] as the job's message type, to its target with the shared retry policy, sets the attempt's
/// `dispatchedAt` and watches the target; so a gone target of one job never rejects the commit of another. A send the
/// call refuses (a target type not registered, a type that does not take the message type) fails the `Dispatch` for
/// good (cause `refused`), a gone target rejects its commit (`commit`), and the `Failed` of the run's own `Dispatch`
/// ends that attempt and its job `failed` with the reason `the job message was not sent (<cause>): <error>`, while the
/// job is running; the run goes on.
///
/// The watch on a target ends a running job `failed` when its worker fails it: a `Failed` the store wrote about the
/// job message of its current attempt, from the run, ends it with the reason `the job message failed at the worker
/// (<cause>): <error>`, and a `Stopped` the store wrote about the target ends every running job dispatched to it with
/// the reason `the worker went away (<cause>)`. Other failures of a watched target, and notifications a client sent,
/// change nothing. The run unwatches a target once none of its running jobs has it. The run cannot see work the worker
/// handed on, a worker that lost its outside work without failing the job message, stopping or reporting, or the
/// failure of a later message of the worker; a target must stay until its job reports. A `Stopped` does not say which
/// instance went away, and a result whose turn was rescheduled can be overtaken by a `Stopped` of its worker, which
/// fails the job and refuses the result.
///
/// When no job runs and none can start, the run ends in that commit: `succeeded` when every job succeeded, else
/// `failed` when a job failed, else `cancelled`. It sends itself a `Report`, which sends the [`JobGraphEnd`] in a turn
/// of its own: a gone report target fails only that `Report`, and a later one sends it. Bad input is refused for good
/// with the state unchanged: a job name, output or reason out of bounds, and a result for an unknown job, a job not
/// running, or an attempt that is not current; a read of the store that fails is tried again.
#[derive(Debug, Clone, Copy, Default)]
pub struct JobGraphActor;

impl Actor for JobGraphActor {
    type Ref = JobGraph;
    type State = JobGraphState;

    async fn handle(&self, turn: &mut Turn<Self>, message: JobGraphMsg) -> HandlerResult {
        match message {
            JobGraphMsg::Start => settle(turn, &[]),
            JobGraphMsg::Result { job, attempt, outcome, output, reason } => {
                check_name(&job).map_err(HandlerError::permanent)?;
                if let Some(output) = &output {
                    check_value("output", output).map_err(HandlerError::permanent)?;
                }
                if let Some(reason) = &reason {
                    check_reason(reason).map_err(HandlerError::permanent)?;
                }
                let to = take_result(turn, &job, attempt, outcome, output, reason)?;
                settle(turn, &[to])
            }
            JobGraphMsg::Report => report(turn).await,
            JobGraphMsg::Dispatch { job, attempt } => {
                check_name(&job).map_err(HandlerError::permanent)?;
                if turn.message().from.as_ref() == Some(turn.self_address()) {
                    dispatch(turn, &job, attempt).await?;
                }
                Ok(())
            }
            JobGraphMsg::Failed(failed) if turn.message().notification => {
                match undispatched(turn, &failed).or_else(|| job_message_failed(turn, &failed)) {
                    Some(to) => settle(turn, &[to]),
                    None => Ok(()),
                }
            }
            JobGraphMsg::Stopped(stopped) if turn.message().notification => {
                let ended = worker_gone(turn, &stopped);
                if ended.is_empty() { Ok(()) } else { settle(turn, &ended) }
            }
            JobGraphMsg::Failed(_) | JobGraphMsg::Stopped(_) => Ok(()),
        }
    }
}

/// The send options of the run's messages: the shared retry policy and this message's correlation id.
fn options(turn: &Turn<JobGraphActor>) -> SendOptions {
    let mut options = SendOptions::new().retry(retry_policy());
    options.correlation_id = turn.message().correlation_id.clone();
    options
}

/// The one rule after every change of a job's status, in its commit, on a running run: skips the waiting jobs that need
/// a failed, skipped or cancelled one; starts the ready jobs in graph order up to the in-flight limit, each with a new
/// attempt and a `Dispatch`; ends the run when no job runs and none can start; and unwatches each target in `ended`, the
/// targets of the jobs that just ended, that no running job has any more. Changes nothing otherwise.
fn settle(turn: &mut Turn<JobGraphActor>, ended: &[ActorAddress]) -> HandlerResult {
    if turn.state().status != JobGraphStatus::Running {
        return Ok(());
    }
    if !turn.state().blocked().is_empty() {
        turn.state_mut().skip_blocked();
    }
    let state = turn.state();
    let statuses = state.statuses();
    let mut running = state.jobs.iter().filter(|j| j.status == JobStatus::Running).count();
    let mut started = Vec::new();
    for (i, job) in state.jobs.iter().enumerate() {
        if running >= state.max_in_flight as usize {
            break;
        }
        if job.status == JobStatus::Waiting && job.needs.iter().all(|n| statuses[n.as_str()] == JobStatus::Succeeded) {
            running += 1;
            started.push(i);
        }
    }
    for &i in &started {
        let options = options(turn);
        let job = &mut turn.state_mut().jobs[i];
        let attempt = match job.attempts.last() {
            None => 1,
            Some(last) => {
                last.attempt.checked_add(1).ok_or_else(|| HandlerError::permanent("no attempt numbers left"))?
            }
        };
        job.attempts.push(JobAttempt { attempt, dispatched_at: None, ended_at: None, outcome: None, reason: None });
        if job.attempts.len() > MAX_ATTEMPTS {
            job.attempts.remove(0);
        }
        job.status = JobStatus::Running;
        let dispatch = JobGraphMsg::Dispatch { job: job.name.clone(), attempt };
        turn.send_self_with(dispatch, options)?;
    }
    if !started.is_empty() {
        let itself = turn.self_address().clone();
        turn.watch(&itself)?;
    }
    if running == 0 {
        let now = turn.now();
        let state = turn.state_mut();
        state.status = state.end_status();
        state.ended_at = Some(now);
        let options = options(turn);
        turn.send_self_with(JobGraphMsg::Report, options)?;
    }
    let mut unwatched: Vec<&ActorAddress> = Vec::new();
    for to in ended {
        let state = turn.state();
        let still = state.jobs.iter().any(|j| j.status == JobStatus::Running && &j.to == to);
        if to == turn.self_address() || still || unwatched.contains(&to) {
            continue;
        }
        unwatched.push(to);
        turn.unwatch(to)?;
    }
    Ok(())
}

/// Ends the current attempt of `job` with the result and returns the job's target, or refuses the result for good, with
/// the state unchanged.
fn take_result(
    turn: &mut Turn<JobGraphActor>,
    job: &str,
    attempt: u32,
    outcome: JobOutcome,
    output: Option<Value>,
    reason: Option<String>,
) -> Result<ActorAddress, HandlerError> {
    let state = turn.state();
    let Some(i) = state.position(job) else {
        return Err(HandlerError::permanent(format!("the run has no job named '{job}'")));
    };
    let entry = &state.jobs[i];
    if entry.status != JobStatus::Running {
        let status = name_of(entry.status);
        return Err(HandlerError::permanent(format!("the job '{job}' is not running: it is {status}")));
    }
    let current = entry.attempts.last().map_or(0, |a| a.attempt);
    if current != attempt {
        return Err(HandlerError::permanent(format!(
            "attempt {attempt} of the job '{job}' is not its current attempt {current}"
        )));
    }
    let now = turn.now();
    let entry = &mut turn.state_mut().jobs[i];
    let last = entry.attempts.last_mut().expect("a running job has an attempt");
    last.ended_at = Some(now);
    last.outcome = Some(outcome);
    last.reason = reason;
    entry.status = outcome.into();
    entry.output = output;
    Ok(entry.to.clone())
}

/// Sends the job message of `attempt` when it is the current attempt of the running `job`, not yet dispatched, sets its
/// `dispatchedAt` and watches the job's target. A send the registry refuses is refused for good; a failed read of the
/// store is tried again.
async fn dispatch(turn: &mut Turn<JobGraphActor>, job: &str, attempt: u32) -> HandlerResult {
    let state = turn.state();
    let Some(i) = state.position(job) else {
        return Ok(());
    };
    let entry = &state.jobs[i];
    let current = entry.attempts.last().is_some_and(|a| a.attempt == attempt && a.dispatched_at.is_none());
    if entry.status != JobStatus::Running || !current {
        return Ok(());
    }
    let start = JobStart { run: turn.self_ref().run.clone(), job: job.to_owned(), attempt, input: entry.input.clone() };
    let (to, message_type) = (entry.to.clone(), entry.message_type.clone());
    let options = options(turn);
    match turn.send_raw(&to, &message_type, serde_json::to_value(start)?, options).await {
        Ok(_) => {}
        // Tried again by the retry policy of the incoming message.
        Err(e @ TurnError::Store(_)) => return Err(e.into()),
        // No registered type, a type that does not take the message, or a field out of bounds.
        Err(e) => return Err(HandlerError::permanent_from(e)),
    }
    turn.watch(&to)?;
    let now = turn.now();
    if let Some(last) = turn.state_mut().jobs[i].attempts.last_mut() {
        last.dispatched_at = Some(now);
    }
    Ok(())
}

/// Ends the attempt of a `Dispatch` that the run sent itself and that failed while the attempt was current, running and
/// not dispatched: its commit never happened, so no worker has the job message. Returns the job's target when it ended
/// one.
fn undispatched(turn: &mut Turn<JobGraphActor>, failed: &Failed) -> Option<ActorAddress> {
    let itself = turn.self_address();
    if &failed.actor != itself || failed.from.as_ref() != Some(itself) {
        return None;
    }
    let Ok(JobGraphMsg::Dispatch { job, attempt }) =
        cicada::message::decode::<JobGraphMsg>(&failed.message_type, &failed.payload)
    else {
        return None;
    };
    let state = turn.state();
    let i = state.position(&job)?;
    let entry = &state.jobs[i];
    let current = entry.attempts.last().is_some_and(|a| a.attempt == attempt && a.dispatched_at.is_none());
    if entry.status != JobStatus::Running || !current {
        return None;
    }
    let reason = format!("the job message was not sent ({}): {}", failed.cause, failed.error);
    Some(fail_current(turn, i, reason))
}

/// Ends the running job whose current job message a `Failed` is about: sent by the run to the job's target as its
/// message type, with a [`JobStart`] of this run, that job and its current attempt. Returns the job's target when it
/// ended one.
fn job_message_failed(turn: &mut Turn<JobGraphActor>, failed: &Failed) -> Option<ActorAddress> {
    if failed.from.as_ref() != Some(turn.self_address()) {
        return None;
    }
    let start = JobStart::deserialize(&failed.payload).ok()?;
    if start.run != turn.self_ref().run {
        return None;
    }
    let state = turn.state();
    let i = state.position(&start.job)?;
    let entry = &state.jobs[i];
    let current = entry.attempts.last().is_some_and(|a| a.attempt == start.attempt);
    if entry.status != JobStatus::Running
        || !current
        || entry.to != failed.actor
        || entry.message_type != failed.message_type
    {
        return None;
    }
    let reason = format!("the job message failed at the worker ({}): {}", failed.cause, failed.error);
    Some(fail_current(turn, i, reason))
}

/// Ends every running job whose job message was sent to the actor that went away; returns their targets.
fn worker_gone(turn: &mut Turn<JobGraphActor>, stopped: &Stopped) -> Vec<ActorAddress> {
    let gone: Vec<usize> = (turn.state().jobs.iter().enumerate())
        .filter(|(_, j)| j.status == JobStatus::Running && j.to == stopped.actor)
        .filter(|(_, j)| j.attempts.last().is_some_and(|a| a.dispatched_at.is_some()))
        .map(|(i, _)| i)
        .collect();
    let reason = format!("the worker went away ({})", name_of(stopped.cause));
    gone.into_iter().map(|i| fail_current(turn, i, reason.clone())).collect()
}

/// Ends the open current attempt of the job at `i`, and the job, `failed` with `reason`, cut to 2,000 characters;
/// returns the job's target.
fn fail_current(turn: &mut Turn<JobGraphActor>, i: usize, reason: String) -> ActorAddress {
    let now = turn.now();
    let entry = &mut turn.state_mut().jobs[i];
    if let Some(last) = entry.attempts.last_mut() {
        last.ended_at = Some(now);
        last.outcome = Some(JobOutcome::Failed);
        last.reason = Some(cut(reason));
    }
    entry.status = JobStatus::Failed;
    entry.to.clone()
}

/// Sends the end report of an ended run to its report target; nothing on a running run. A send the registry refuses is
/// refused for good; a failed read of the store is tried again.
async fn report(turn: &mut Turn<JobGraphActor>) -> HandlerResult {
    let state = turn.state();
    if state.status == JobGraphStatus::Running {
        return Ok(());
    }
    let jobs = state.jobs.iter().map(|job| {
        let last = job.attempts.last();
        JobReport {
            name: job.name.clone(),
            status: job.status,
            attempt: last.map(|a| a.attempt),
            output: job.output.clone(),
            reason: last.and_then(|a| a.reason.clone()),
        }
    });
    let end = JobGraphEnd { run: turn.self_ref().run.clone(), status: state.status, jobs: jobs.collect() };
    let (to, message_type) = (state.report_to.clone(), state.report_type.clone());
    let options = options(turn);
    match turn.send_raw(&to, &message_type, serde_json::to_value(end)?, options).await {
        Ok(_) => Ok(()),
        // Tried again by the retry policy of the incoming message.
        Err(e @ TurnError::Store(_)) => Err(e.into()),
        // No registered type, a type that does not take the message, or a payload out of bounds.
        Err(e) => Err(HandlerError::permanent_from(e)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Ci;

    impl ActorRef for Ci {
        const TYPE: &'static str = "ci";
        type Message = JobGraphMsg;
    }

    fn job(name: &str, needs: &[&str]) -> Job {
        Job::new(name, &Ci, "Build", json!({})).unwrap().with_needs(needs.iter().copied())
    }

    fn build(jobs: Vec<Job>) -> Result<JobGraphState, InvalidJobGraph> {
        JobGraphState::new(jobs, 2, &Ci, "GraphEnded")
    }

    /// A job's JSON, with `extra` fields over a waiting job without needs.
    fn entry(name: &str, needs: &[&str], extra: Value) -> Value {
        let mut json = json!({"name": name, "needs": needs, "to": "ci/_", "messageType": "Build", "input": {}});
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    }

    fn state(jobs: Vec<Value>, extra: Value) -> Value {
        let mut json = json!({"maxInFlight": 2, "reportTo": "ci/_", "reportType": "GraphEnded", "jobs": jobs});
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    }

    fn attempt(n: u32, outcome: Option<&str>) -> Value {
        let ended = outcome.map(|_| "2026-01-01T00:00:01Z");
        json!({"attempt": n, "dispatchedAt": "2026-01-01T00:00:00Z", "endedAt": ended, "outcome": outcome,
            "reason": null})
    }

    fn nested(depth: usize) -> Value {
        (0..depth).fold(Value::Null, |inner, _| Value::Array(vec![inner]))
    }

    fn assert_refused(built: Result<JobGraphState, InvalidJobGraph>, json: Value, why: &[&str]) {
        let built_error = built.expect_err("the constructor refuses it").to_string();
        let decoded = serde_json::from_value::<JobGraphState>(json).expect_err("the decoder refuses it").to_string();
        for part in why {
            assert!(built_error.contains(part), "{built_error} names {part}");
            assert!(decoded.contains(part), "{decoded} names {part}");
        }
    }

    #[test]
    fn a_run_from_its_create_is_running_with_every_job_waiting() {
        let built = build(vec![job("a", &[]), job("b", &["a"])]).unwrap();
        assert_eq!((built.status, built.ended_at), (JobGraphStatus::Running, None));
        assert!(
            built.jobs.iter().all(|j| j.status == JobStatus::Waiting && j.attempts.is_empty() && j.output.is_none())
        );
        let decoded: JobGraphState =
            serde_json::from_value(state(vec![entry("a", &[], json!({})), entry("b", &["a"], json!({}))], json!({})))
                .unwrap();
        assert_eq!(decoded, built, "the status, endedAt and a job's status, output and attempts may be left out");
        assert_eq!(serde_json::from_value::<JobGraphState>(serde_json::to_value(&built).unwrap()).unwrap(), built);
    }

    #[test]
    fn a_cycle_is_refused_and_a_job_of_it_named() {
        let jobs = vec![job("a", &["c"]), job("b", &["a"]), job("c", &["b"]), job("d", &["a"])];
        let json = state(
            vec![
                entry("a", &["c"], json!({})),
                entry("b", &["a"], json!({})),
                entry("c", &["b"], json!({})),
                entry("d", &["a"], json!({})),
            ],
            json!({}),
        );
        let error = build(jobs.clone()).unwrap_err().to_string();
        assert!(error.contains("cycle"), "{error}");
        assert!(["'a'", "'b'", "'c'"].iter().any(|n| error.contains(n)), "{error} names a job of the cycle");
        assert_refused(build(jobs), json, &["cycle"]);
        let downstream = vec![job("d", &["a"]), job("a", &["b"]), job("b", &["a"])];
        let error = build(downstream).unwrap_err().to_string();
        assert!(!error.contains("'d'"), "{error}: d needs the cycle but is not in it");
    }

    #[test]
    fn needs_that_name_no_job_repeat_or_name_the_job_itself_are_refused() {
        let cases: [(Vec<Job>, Vec<Value>, &[&str]); 3] = [
            (
                vec![job("deploy", &["test"])],
                vec![entry("deploy", &["test"], json!({}))],
                &["'deploy'", "'test'", "no job"],
            ),
            (
                vec![job("a", &[]), job("b", &["a", "a"])],
                vec![entry("a", &[], json!({})), entry("b", &["a", "a"], json!({}))],
                &["'b'", "'a'", "twice"],
            ),
            (vec![job("a", &["a"])], vec![entry("a", &["a"], json!({}))], &["'a'", "itself"]),
        ];
        for (jobs, entries, why) in cases {
            assert_refused(build(jobs), state(entries, json!({})), why);
        }
    }

    #[test]
    fn every_bound_of_a_graph_is_refused_by_the_constructor_and_the_decoder() {
        let many: Vec<Job> = (0..=MAX_JOBS).map(|i| job(&format!("j{i}"), &[])).collect();
        let many_json: Vec<Value> = (0..=MAX_JOBS).map(|i| entry(&format!("j{i}"), &[], json!({}))).collect();
        assert_refused(build(many), state(many_json, json!({})), &["1 through 100 jobs"]);
        assert_refused(build(vec![]), state(vec![], json!({})), &["1 through 100 jobs"]);
        assert_refused(
            build(vec![job("a", &[]), job("a", &[])]),
            state(vec![entry("a", &[], json!({})), entry("a", &[], json!({}))], json!({})),
            &["'a'", "twice"],
        );
        let long = "n".repeat(MAX_NAME_LENGTH + 1);
        for name in ["", long.as_str(), "build step", "bü", "a/b"] {
            assert_refused(
                build(vec![job(name, &[])]),
                state(vec![entry(name, &[], json!({}))], json!({})),
                &["job name"],
            );
        }
        let longest = "n".repeat(MAX_NAME_LENGTH);
        assert!(build(vec![job(&longest, &[]), job("A-z_0.9", &[&longest])]).is_ok());
        let long_type = "T".repeat(MAX_TYPE_LENGTH + 1);
        for message_type in ["", long_type.as_str(), "Bu\0ild"] {
            let mut bad = job("a", &[]);
            bad.message_type = message_type.into();
            let json = state(vec![entry("a", &[], json!({"messageType": message_type}))], json!({}));
            assert_refused(build(vec![bad]), json, &["messageType"]);
            let built = JobGraphState::new(vec![job("a", &[])], 2, &Ci, message_type);
            let json = state(vec![entry("a", &[], json!({}))], json!({"reportType": message_type}));
            assert_refused(built, json, &["reportType"]);
        }
        let big = json!("x".repeat(MAX_VALUE_BYTES));
        for (input, why) in [(big, "bytes"), (nested(MAX_VALUE_DEPTH + 1), "levels")] {
            let mut bad = job("a", &[]);
            bad.input = input.clone();
            let json = state(vec![entry("a", &[], json!({"input": input}))], json!({}));
            assert_refused(build(vec![bad]), json, &["input", why]);
        }
        let mut fits = job("a", &[]);
        fits.input = nested(MAX_VALUE_DEPTH);
        assert!(build(vec![fits]).is_ok());
        let mut fits = job("a", &[]);
        fits.input = json!("x".repeat(MAX_VALUE_BYTES - 2));
        assert!(build(vec![fits]).is_ok(), "16 KiB with its quotes");
        for limit in [0, MAX_IN_FLIGHT + 1] {
            let built = JobGraphState::new(vec![job("a", &[])], limit, &Ci, "GraphEnded");
            let json = state(vec![entry("a", &[], json!({}))], json!({"maxInFlight": limit}));
            assert_refused(built, json, &["maxInFlight must be 1 through 100"]);
        }
        assert!(JobGraphState::new(vec![job("a", &[])], MAX_IN_FLIGHT, &Ci, "GraphEnded").is_ok());
    }

    #[test]
    fn a_ref_without_an_address_is_refused() {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct Bad;
        impl ActorRef for Bad {
            const TYPE: &'static str = "no type name";
            type Message = JobGraphMsg;
        }
        assert!(Job::new("a", &Bad, "Build", ()).is_err());
        assert!(JobGraphState::new(vec![job("a", &[])], 2, &Bad, "GraphEnded").is_err());
    }

    fn refused(json: Value, why: &str) {
        let decoded = serde_json::from_value::<JobGraphState>(json.clone());
        assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {why}: {json}");
    }

    fn taken(json: Value) {
        let decoded = serde_json::from_value::<JobGraphState>(json.clone());
        assert!(decoded.is_ok(), "{decoded:?}: {json}");
    }

    #[test]
    fn the_attempts_of_a_job_are_checked() {
        let running = |attempts: Vec<Value>| {
            state(vec![entry("a", &[], json!({"status": "running", "attempts": attempts}))], json!({}))
        };
        let open =
            |n: u32| json!({"attempt": n, "dispatchedAt": null, "endedAt": null, "outcome": null, "reason": null});
        taken(running(vec![open(1)]));
        let mut kept: Vec<Value> = (1..MAX_ATTEMPTS as u32).map(|n| attempt(n, Some("failed"))).collect();
        kept.push(open(MAX_ATTEMPTS as u32));
        taken(running(kept.clone()));
        let mut later: Vec<Value> = (6..5 + MAX_ATTEMPTS as u32).map(|n| attempt(n, Some("failed"))).collect();
        later.push(open(5 + MAX_ATTEMPTS as u32));
        taken(running(later));
        kept.insert(0, attempt(0, Some("failed")));
        refused(running(kept), "at most 20 attempts");
        refused(running(vec![open(0)]), "numbered from 1");
        refused(running(vec![attempt(1, Some("failed")), open(3)]), "consecutive");
        refused(running(vec![attempt(2, Some("failed")), open(1)]), "consecutive");
        refused(running(vec![open(1), open(2)]), "only the last attempt");
        let mut no_outcome = attempt(1, None);
        no_outcome["endedAt"] = json!("2026-01-01T00:00:01Z");
        refused(
            state(vec![entry("a", &[], json!({"status": "failed", "attempts": [no_outcome]}))], json!({})),
            "outcome",
        );
        let mut open_reason = open(1);
        open_reason["reason"] = json!("why");
        refused(running(vec![open_reason]), "no reason");
        let mut long_reason = attempt(1, Some("failed"));
        long_reason["reason"] = json!("r".repeat(MAX_REASON_LENGTH + 1));
        let failed = |a: Value| {
            state(
                vec![entry("a", &[], json!({"status": "failed", "attempts": [a]}))],
                json!({"status": "failed",
                "endedAt": "2026-01-01T00:00:02Z"}),
            )
        };
        refused(failed(long_reason), "reason is at most 2000");
        let mut longest = attempt(1, Some("failed"));
        longest["reason"] = json!("r".repeat(MAX_REASON_LENGTH));
        taken(failed(longest));
        let big = json!({"status": "succeeded", "output": "x".repeat(MAX_VALUE_BYTES), "attempts": [attempt(1, Some("succeeded"))]});
        refused(
            state(vec![entry("a", &[], big)], json!({"status": "succeeded", "endedAt": "2026-01-01T00:00:02Z"})),
            "output",
        );
    }

    #[test]
    fn a_job_status_must_match_its_last_attempt() {
        let ended = json!({"status": "failed", "endedAt": "2026-01-01T00:00:02Z"});
        for (status, attempts) in [
            ("waiting", vec![attempt(1, Some("failed"))]),
            ("running", vec![]),
            ("running", vec![attempt(1, Some("failed"))]),
            ("succeeded", vec![attempt(1, Some("failed"))]),
            ("failed", vec![attempt(1, Some("succeeded"))]),
            ("failed", vec![attempt(1, None)]),
            ("skipped", vec![attempt(1, None)]),
            ("cancelled", vec![attempt(1, None)]),
        ] {
            refused(
                state(vec![entry("a", &[], json!({"status": status, "attempts": attempts}))], ended.clone()),
                "does not match",
            );
        }
    }

    #[test]
    fn an_inconsistent_run_is_refused() {
        let at = "2026-01-01T00:00:02Z";
        let job = |name: &str, needs: &[&str], status: &str, outcome: Option<&str>| {
            let attempts = if status == "waiting" { vec![] } else { vec![attempt(1, outcome)] };
            entry(name, needs, json!({"status": status, "attempts": attempts}))
        };
        let three_running =
            vec![job("a", &[], "running", None), job("b", &[], "running", None), job("c", &[], "running", None)];
        refused(state(three_running, json!({})), "at most maxInFlight (2) jobs run, had 3");
        refused(
            state(vec![job("a", &[], "running", None), job("b", &["a"], "running", None)], json!({})),
            "started before every job it needs succeeded",
        );
        taken(state(vec![job("a", &[], "succeeded", Some("succeeded")), job("b", &["a"], "waiting", None)], json!({})));
        refused(state(vec![job("a", &[], "failed", Some("failed"))], json!({})), "ended exactly when no job runs");
        refused(
            state(vec![job("a", &[], "running", None)], json!({"status": "failed", "endedAt": at})),
            "ended exactly when no job runs",
        );
        refused(
            state(vec![job("a", &[], "waiting", None)], json!({"status": "failed", "endedAt": at})),
            "ended exactly when no job runs",
        );
        refused(state(vec![job("a", &[], "failed", Some("failed"))], json!({"status": "failed"})), "endedAt");
        refused(state(vec![job("a", &[], "running", None)], json!({"endedAt": at})), "endedAt");
        refused(
            state(vec![job("a", &[], "failed", Some("failed"))], json!({"status": "succeeded", "endedAt": at})),
            "it is succeeded, not failed",
        );
        refused(
            state(vec![job("a", &[], "succeeded", Some("succeeded"))], json!({"status": "failed", "endedAt": at})),
            "it is failed, not succeeded",
        );
        taken(state(
            vec![job("a", &[], "failed", Some("failed")), bare("b", &["a"], "skipped")],
            json!({"status": "failed", "endedAt": at}),
        ));
        taken(state(vec![job("a", &[], "succeeded", Some("succeeded")), job("b", &["a"], "running", None)], json!({})));
    }

    /// A job's JSON with `status` and no attempts.
    fn bare(name: &str, needs: &[&str], status: &str) -> Value {
        entry(name, needs, json!({"status": status}))
    }

    #[test]
    fn inconsistent_skipped_jobs_and_end_statuses_are_refused() {
        let at = "2026-01-01T00:00:02Z";
        let ended = |status: &str| json!({"status": status, "endedAt": at});
        let failed = entry("a", &[], json!({"status": "failed", "attempts": [attempt(1, Some("failed"))]}));
        let succeeded = entry("a", &[], json!({"status": "succeeded", "attempts": [attempt(1, Some("succeeded"))]}));
        refused(state(vec![succeeded.clone(), bare("b", &["a"], "skipped")], ended("succeeded")), "no job it needs");
        refused(state(vec![bare("b", &[], "skipped")], ended("cancelled")), "no job it needs");
        let skipped_attempt =
            entry("b", &["a"], json!({"status": "skipped", "attempts": [attempt(1, Some("failed"))]}));
        refused(state(vec![failed.clone(), skipped_attempt], ended("failed")), "skipped job 'b' has an attempt");
        refused(state(vec![failed.clone(), bare("b", &["a"], "waiting")], ended("failed")), "'b' is waiting");
        refused(
            state(
                vec![bare("a", &[], "cancelled"), bare("b", &["a"], "waiting"), bare("c", &["b"], "waiting")],
                ended("cancelled"),
            ),
            "'b' is waiting",
        );
        refused(
            state(vec![failed.clone(), bare("b", &["a"], "skipped")], ended("succeeded")),
            "it is succeeded, not failed",
        );
        refused(state(vec![succeeded.clone()], ended("cancelled")), "it is cancelled, not succeeded");
        refused(state(vec![bare("x", &[], "cancelled"), failed.clone()], ended("cancelled")), "not failed");
        refused(state(vec![bare("a", &[], "cancelled")], ended("failed")), "it is failed, not cancelled");
        taken(state(
            vec![bare("x", &[], "cancelled"), bare("b", &["x"], "skipped"), bare("c", &["b"], "skipped")],
            ended("cancelled"),
        ));
        let mut both = failed;
        both["name"] = json!("f");
        taken(state(vec![bare("x", &[], "cancelled"), both, bare("b", &["f", "x"], "skipped")], ended("failed")));
    }

    /// The decoded state of `jobs`, running, with in-flight limit 100.
    fn running(jobs: Vec<Value>) -> JobGraphState {
        serde_json::from_value(state(jobs, json!({"maxInFlight": 100}))).unwrap()
    }

    fn skipped_names(state: &JobGraphState) -> Vec<&str> {
        state.jobs.iter().filter(|j| j.status == JobStatus::Skipped).map(|j| j.name.as_str()).collect()
    }

    #[test]
    fn the_skip_loop_skips_every_waiting_job_that_needs_a_blocked_one_through_any_number_of_jobs() {
        let failed =
            |name: &str| entry(name, &[], json!({"status": "failed", "attempts": [attempt(1, Some("failed"))]}));
        let open = json!({"attempt": 1, "dispatchedAt": null, "endedAt": null, "outcome": null, "reason": null});
        let ended: JobAttempt = serde_json::from_value(attempt(1, Some("failed"))).unwrap();
        for (status, attempts) in [(JobStatus::Failed, vec![ended]), (JobStatus::Cancelled, vec![])] {
            // Dependents listed before the jobs they need, so one pass in graph order is not enough.
            let mut state = running(vec![
                bare("d", &["c"], "waiting"),
                bare("c", &["b"], "waiting"),
                bare("b", &["a"], "waiting"),
                entry("a", &[], json!({"status": "running", "attempts": [open.clone()]})),
                entry("e", &[], json!({"status": "running", "attempts": [open.clone()]})),
                bare("f", &["e"], "waiting"),
                bare("g", &[], "waiting"),
            ]);
            (state.jobs[3].status, state.jobs[3].attempts) = (status, attempts);
            state.skip_blocked();
            assert_eq!(skipped_names(&state), ["d", "c", "b"]);
            assert!(state.jobs.iter().filter(|j| j.status == JobStatus::Skipped).all(|j| j.attempts.is_empty()));
            assert_eq!(state.jobs[5].status, JobStatus::Waiting, "f needs a running job");
            assert_eq!(state.jobs[6].status, JobStatus::Waiting, "g needs nothing");
            assert!(state.check().is_ok(), "{:?}", state.check());
            let before = state.clone();
            state.skip_blocked();
            assert_eq!(state, before, "a second pass changes nothing");
        }
        // A job skipped already blocks the jobs that need it.
        let mut state = running(vec![
            failed("a"),
            bare("b", &["a"], "skipped"),
            entry("x", &[], json!({"status": "running", "attempts": [open]})),
        ]);
        state.jobs.push(JobEntry::from(job("c", &["b"])));
        state.skip_blocked();
        assert_eq!(skipped_names(&state), ["b", "c"]);
    }

    #[test]
    fn the_end_status_follows_the_jobs() {
        let succeeded =
            |n: &str| entry(n, &[], json!({"status": "succeeded", "attempts": [attempt(1, Some("succeeded"))]}));
        let failed = |n: &str| entry(n, &[], json!({"status": "failed", "attempts": [attempt(1, Some("failed"))]}));
        let at = json!({"maxInFlight": 2});
        let of = |jobs: Vec<Value>| {
            let mut json = state(jobs, at.clone());
            json["status"] = json!("failed");
            json["endedAt"] = json!("2026-01-01T00:00:02Z");
            let raw: RawState = serde_json::from_value(json).unwrap();
            JobGraphState {
                max_in_flight: raw.max_in_flight,
                report_to: raw.report_to,
                report_type: raw.report_type,
                status: raw.status,
                ended_at: raw.ended_at,
                jobs: raw.jobs,
            }
            .end_status()
        };
        assert_eq!(of(vec![succeeded("a"), succeeded("b")]), JobGraphStatus::Succeeded);
        assert_eq!(of(vec![succeeded("a"), failed("b"), bare("c", &["b"], "skipped")]), JobGraphStatus::Failed);
        assert_eq!(of(vec![bare("a", &[], "cancelled"), failed("b")]), JobGraphStatus::Failed, "a failure wins");
        assert_eq!(
            of(vec![succeeded("a"), bare("b", &[], "cancelled"), bare("c", &["b"], "skipped")]),
            JobGraphStatus::Cancelled
        );
    }

    #[test]
    fn the_failed_helper_cuts_the_reason() {
        let start = JobStart { run: "r".into(), job: "a".into(), attempt: 3, input: Value::Null };
        let JobGraphMsg::Result { reason, attempt, outcome, .. } = JobGraphMsg::failed(&start, "é".repeat(2_500))
        else {
            panic!("a result");
        };
        assert_eq!((reason.unwrap().chars().count(), attempt, outcome), (MAX_REASON_LENGTH, 3, JobOutcome::Failed));
        let JobGraphMsg::Result { output, .. } = JobGraphMsg::succeeded(&start, ()).unwrap() else {
            panic!("a result");
        };
        assert_eq!(output, None, "() is no output");
        assert_eq!(cut("short".into()), "short");
    }

    #[test]
    fn depth_counts_as_the_core_does() {
        assert_eq!(depth(&json!(1)), 0);
        assert_eq!(depth(&json!({"a": {"b": 1}})), 2);
        assert_eq!(depth(&json!([[], {}])), 2);
        assert_eq!(depth(&nested(5)), 5);
    }
}
