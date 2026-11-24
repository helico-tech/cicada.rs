//! The job graph, on virtual time with `TestActors` (ADR 0059, ADR 0060): ready jobs in graph order up to the in-flight
//! limit, each job message in a turn of its own, a failed dispatch that fails only its attempt, the attempt fence on
//! results, a failed job that skips its dependents, the watch on each target, and the end report.

mod common;

use cicada::prelude::*;
use cicada::store::{Message, OutcomeStatus};
use cicada::{FailureCause, WatchTarget};
use cicada_actors::{
    AttemptOutcome, Job, JobGraph, JobGraphActor, JobGraphEnd, JobGraphMsg, JobGraphState, JobGraphStatus, JobOutcome,
    JobStart, JobStatus,
};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// `builder/{id}`: keeps every job message it is sent, fails its handler on one whose input has `"fail": true`, and
/// relays a result to a run when told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Builder {
    id: String,
}

impl ActorRef for Builder {
    const TYPE: &'static str = "builder";
    type Message = BuilderMsg;
}

fn builder(id: &str) -> Builder {
    Builder { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum BuilderMsg {
    Build(JobStart),
    /// Sends `result` to the run `run`.
    Relay {
        run: String,
        result: JobGraphMsg,
    },
    /// Sends `result` to the run `run` and stops, in one turn.
    ReportAndStop {
        run: String,
        result: JobGraphMsg,
    },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Starts {
    #[serde(default)]
    log: Vec<JobStart>,
}

struct BuilderActor;

impl Actor for BuilderActor {
    type Ref = Builder;
    type State = Starts;

    async fn handle(&self, turn: &mut Turn<Self>, message: BuilderMsg) -> HandlerResult {
        match message {
            BuilderMsg::Build(start) if start.input["fail"] == json!(true) => {
                return Err(HandlerError::msg("the build broke"));
            }
            BuilderMsg::Build(start) => turn.state_mut().log.push(start),
            BuilderMsg::Relay { run, result } => {
                turn.send(&JobGraph::named(run), result)?;
            }
            BuilderMsg::ReportAndStop { run, result } => {
                turn.send(&JobGraph::named(run), result)?;
                turn.stop();
            }
        }
        Ok(())
    }
}

/// `ci/{id}`: the report target, which keeps every end report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Ci {
    id: String,
}

impl ActorRef for Ci {
    const TYPE: &'static str = "ci";
    type Message = CiMsg;
}

fn ci(id: &str) -> Ci {
    Ci { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CiMsg {
    GraphEnded(JobGraphEnd),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Reports {
    #[serde(default)]
    log: Vec<JobGraphEnd>,
}

struct CiActor;

impl Actor for CiActor {
    type Ref = Ci;
    type State = Reports;

    async fn handle(&self, turn: &mut Turn<Self>, message: CiMsg) -> HandlerResult {
        let CiMsg::GraphEnded(end) = message;
        turn.state_mut().log.push(end);
        Ok(())
    }
}

fn run() -> JobGraph {
    JobGraph::named("run-7")
}

/// A node with `builder/a`, `builder/b`, `builder/other`, `ci/main`, `worker/a`, and the witness, which watches every
/// run; no run yet.
async fn node() -> TestActors {
    let test = TestActors::builder()
        .with(|node| node.host(JobGraphActor).host(BuilderActor).host(CiActor).host(WorkerActor).host(WitnessActor))
        .build()
        .await
        .unwrap();
    for id in ["a", "b", "other"] {
        test.create(&builder(id), &Starts::default()).await;
    }
    test.create(&ci("main"), &Reports::default()).await;
    test.create(&worker("a"), &WorkerState::default()).await;
    witness(&test, "cicada.job-graph").await;
    test
}

/// The job `name` for `builder/a`, needing `needs`, with input `{"job": name}`.
fn job(name: &str, needs: &[&str]) -> Job {
    Job::new(name, &builder("a"), "Build", json!({"job": name})).unwrap().with_needs(needs.iter().copied())
}

/// The job `name` with its job message to `to`.
fn job_to(name: &str, to: &str) -> Job {
    let mut job = job(name, &[]);
    job.to = ActorAddress::parse(to).unwrap();
    job
}

/// Creates the run of `jobs` with in-flight limit `limit`, reporting to `ci/main`.
async fn create(test: &TestActors, jobs: Vec<Job>, limit: u32) {
    test.create(&run(), &JobGraphState::new(jobs, limit, &ci("main"), "GraphEnded").unwrap()).await;
}

/// Creates the run, sends `Start` and runs until idle.
async fn start(test: &TestActors, jobs: Vec<Job>, limit: u32) {
    create(test, jobs, limit).await;
    test.send(&run(), JobGraphMsg::Start).await;
    test.run_until_idle().await;
}

async fn state(test: &TestActors) -> JobGraphState {
    test.state::<_, JobGraphState>(&run()).await
}

async fn run_status(test: &TestActors) -> JobGraphStatus {
    state(test).await.status
}

/// The status of each job, in graph order.
async fn statuses(test: &TestActors) -> Vec<(String, JobStatus)> {
    state(test).await.jobs.into_iter().map(|j| (j.name, j.status)).collect()
}

/// The job messages `builder/{id}` got.
async fn starts(test: &TestActors, id: &str) -> Vec<JobStart> {
    test.state::<_, Starts>(&builder(id)).await.log
}

/// The jobs whose messages `builder/{id}` got, in order.
async fn started(test: &TestActors, id: &str) -> Vec<String> {
    starts(test, id).await.into_iter().map(|s| s.job).collect()
}

/// The job message of `job` that `builder/a` got last.
async fn start_of(test: &TestActors, job: &str) -> JobStart {
    starts(test, "a").await.into_iter().rev().find(|s| s.job == job).expect("a job message")
}

/// Reports the last attempt of `job` at `builder/a` succeeded with `{"built": job}`, and runs until idle.
async fn succeed(test: &TestActors, job: &str) -> Message {
    let start = start_of(test, job).await;
    let m = test.send(&run(), JobGraphMsg::succeeded(&start, json!({"built": job})).unwrap()).await;
    test.run_until_idle().await;
    m
}

/// Reports the last attempt of `job` at `builder/a` failed, and runs until idle.
async fn fail(test: &TestActors, job: &str) {
    let start = start_of(test, job).await;
    test.send(&run(), JobGraphMsg::failed(&start, format!("{job} broke"))).await;
    test.run_until_idle().await;
}

async fn reports(test: &TestActors, id: &str) -> Vec<JobGraphEnd> {
    test.state::<_, Reports>(&ci(id)).await.log
}

/// The `Failed` the witness was told of the run's messages of `message_type`.
async fn failed_of(test: &TestActors, message_type: &str) -> Vec<cicada::Failed> {
    told(test).await.into_iter().filter(|f| f.message_type == message_type).collect()
}

use JobStatus::{Failed, Running, Skipped, Succeeded, Waiting};

fn named(statuses: &[(&str, JobStatus)]) -> Vec<(String, JobStatus)> {
    statuses.iter().map(|(n, s)| (n.to_string(), *s)).collect()
}

/// Scenario: A run from its create.
#[tokio::test(start_paused = true)]
async fn a_run_from_its_create_sends_no_job_message() {
    let test = node().await;
    create(&test, vec![job("a", &[]), job("b", &["a"])], 2).await;
    test.run_until_idle().await;
    let state = state(&test).await;
    assert_eq!(state.status, JobGraphStatus::Running);
    assert_eq!(statuses(&test).await, named(&[("a", Waiting), ("b", Waiting)]));
    assert!(state.jobs.iter().all(|j| j.attempts.is_empty()));
    assert!(started(&test, "a").await.is_empty());
}

/// Scenario: A diamond graph.
#[tokio::test(start_paused = true)]
async fn a_diamond_starts_each_job_once_its_needs_succeeded() {
    let test = node().await;
    let jobs = vec![job("a", &[]), job("b", &["a"]), job("c", &["a"]), job("d", &["b", "c"])];
    start(&test, jobs, 4).await;
    assert_eq!(started(&test, "a").await, ["a"]);
    let first = start_of(&test, "a").await;
    assert_eq!(first, JobStart { run: "run-7".into(), job: "a".into(), attempt: 1, input: json!({"job": "a"}) });
    succeed(&test, "a").await;
    assert_eq!(started(&test, "a").await, ["a", "b", "c"]);
    succeed(&test, "b").await;
    assert_eq!(started(&test, "a").await, ["a", "b", "c"], "d waits for c");
    assert_eq!(statuses(&test).await, named(&[("a", Succeeded), ("b", Succeeded), ("c", Running), ("d", Waiting)]));
    succeed(&test, "c").await;
    assert_eq!(started(&test, "a").await, ["a", "b", "c", "d"]);
    succeed(&test, "d").await;
    let state = state(&test).await;
    assert_eq!(state.status, JobGraphStatus::Succeeded);
    assert_eq!(state.jobs[3].output, Some(json!({"built": "d"})));
    let attempt = &state.jobs[0].attempts[0];
    assert_eq!((attempt.attempt, attempt.outcome), (1, Some(AttemptOutcome::Succeeded)));
    assert!(attempt.dispatched_at.is_some() && attempt.ended_at.is_some());
}

/// Scenario: The in-flight limit.
#[tokio::test(start_paused = true)]
async fn no_more_jobs_run_than_the_in_flight_limit() {
    let test = node().await;
    start(&test, vec![job("j1", &[]), job("j2", &[]), job("j3", &[]), job("j4", &[])], 2).await;
    assert_eq!(started(&test, "a").await, ["j1", "j2"]);
    succeed(&test, "j2").await;
    assert_eq!(started(&test, "a").await, ["j1", "j2", "j3"], "the next in graph order");
    assert_eq!(statuses(&test).await, named(&[("j1", Running), ("j2", Succeeded), ("j3", Running), ("j4", Waiting)]));
}

/// Scenario: A second `Start` changes nothing.
#[tokio::test(start_paused = true)]
async fn a_second_start_changes_nothing() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &["a"])], 2).await;
    let version = test.version(&run()).await;
    let m = test.send(&run(), JobGraphMsg::Start).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(test.version(&run()).await, version);
    assert_eq!(started(&test, "a").await, ["a"]);
}

/// Each job message goes in a turn of its own: the commit that starts the jobs only marks them and sends `Dispatch`.
#[tokio::test(start_paused = true)]
async fn the_commit_that_starts_a_job_sends_the_run_a_dispatch() {
    let test = node().await;
    create(&test, vec![job("a", &[]), job("b", &[])], 2).await;
    test.send(&run(), JobGraphMsg::Start).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(statuses(&test).await, named(&[("a", Running), ("b", Running)]));
    assert!(state(&test).await.jobs.iter().all(|j| j.attempts[0].dispatched_at.is_none()));
    assert!(started(&test, "a").await.is_empty());
    let pending: Vec<(String, Value)> =
        test.pending(&run()).await.into_iter().map(|m| (m.message_type, m.payload)).collect();
    let dispatch = |job: &str| ("Dispatch".to_owned(), json!({"job": job, "attempt": 1}));
    assert_eq!(pending, [dispatch("a"), dispatch("b")]);
    let watches = test.system().watches(Some(&addr(&run())), None, None, 10).await.unwrap().items;
    let targets: Vec<String> = watches.iter().map(|w| w.target.to_string()).collect();
    assert_eq!(targets, ["cicada.job-graph/run-7"], "the run watches itself");
    test.run_until_idle().await;
    assert_eq!(started(&test, "a").await, ["a", "b"]);
    assert!(state(&test).await.jobs.iter().all(|j| j.attempts[0].dispatched_at.is_some()));
    let message = test.pending(&builder("a")).await;
    assert!(message.is_empty());
}

/// Scenario: A gone target.
#[tokio::test(start_paused = true)]
async fn a_gone_target_fails_only_its_attempt() {
    let test = node().await;
    start(&test, vec![job_to("a", "builder/ghost"), job("b", &[])], 2).await;
    assert_eq!(started(&test, "a").await, ["b"]);
    let failed = failed_of(&test, "Dispatch").await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].cause, FailureCause::Commit);
    let state = state(&test).await;
    assert_eq!(state.jobs[0].status, Failed);
    let attempt = &state.jobs[0].attempts[0];
    let reason = attempt.reason.as_deref().unwrap();
    assert!(reason.starts_with("the job message was not sent (commit): "), "{reason}");
    assert_eq!((attempt.outcome, attempt.dispatched_at), (Some(AttemptOutcome::Failed), None));
    assert!(attempt.ended_at.is_some());
    let m = succeed(&test, "b").await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "b reports");
    assert_eq!(run_status(&test).await, JobGraphStatus::Failed);
}

/// Scenario: An unregistered target type. And a type that does not take the message.
#[tokio::test(start_paused = true)]
async fn a_dispatch_the_send_refuses_fails_for_good_and_fails_its_attempt() {
    for target in ["nobody/x", "worker/a"] {
        let test = node().await;
        start(&test, vec![job_to("a", target), job("b", &[])], 2).await;
        assert_eq!(started(&test, "a").await, ["b"], "{target}");
        let failed = failed_of(&test, "Dispatch").await;
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert_eq!((failed[0].cause, failed[0].attempts), (FailureCause::Refused, 1), "not tried again");
        let state = state(&test).await;
        assert_eq!(state.jobs[0].status, Failed);
        let reason = state.jobs[0].attempts[0].reason.clone().unwrap();
        assert!(reason.starts_with("the job message was not sent (refused): "), "{reason}");
        assert_eq!(state.jobs[1].status, Running, "the run goes on");
    }
}

/// Plants a run with `a` running its attempt 1, not yet dispatched, and `b` waiting on it.
async fn undispatched(test: &TestActors) {
    let planted = json!({"maxInFlight": 2, "reportTo": "ci/main", "reportType": "GraphEnded", "jobs": [
        {"name": "a", "needs": [], "to": "builder/a", "messageType": "Build", "input": {}, "status": "running",
         "attempts": [{"attempt": 1, "dispatchedAt": null, "endedAt": null, "outcome": null, "reason": null}]},
        {"name": "b", "needs": ["a"], "to": "builder/a", "messageType": "Build", "input": {}}]});
    test.create(&run(), &planted).await;
}

/// Scenario: A message the run did not send itself: a client's `Dispatch` for a running job, and one for a waiting
/// job of a run not started.
#[tokio::test(start_paused = true)]
async fn a_dispatch_sent_by_a_client_sends_nothing() {
    let test = node().await;
    undispatched(&test).await;
    let version = test.version(&run()).await;
    let m = test.send(&run(), JobGraphMsg::Dispatch { job: "a".into(), attempt: 1 }).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(started(&test, "a").await.is_empty());
    assert_eq!(test.version(&run()).await, version);

    let test = node().await;
    create(&test, vec![job("a", &[])], 2).await;
    test.send(&run(), JobGraphMsg::Dispatch { job: "a".into(), attempt: 1 }).await;
    test.run_until_idle().await;
    assert!(started(&test, "a").await.is_empty());
    assert_eq!(statuses(&test).await, named(&[("a", Waiting)]));
}

/// Scenario: A message the run did not send itself: a `Failed` that names one of its dispatches.
#[tokio::test(start_paused = true)]
async fn a_failed_sent_by_a_client_changes_no_job() {
    let test = node().await;
    undispatched(&test).await;
    let dispatch = cicada::message::encode(&JobGraphMsg::Dispatch { job: "a".into(), attempt: 1 }).unwrap();
    let planted = cicada::Failed {
        actor: addr(&run()),
        message_id: MessageId::new("m-planted").unwrap(),
        message_type: dispatch.message_type,
        payload: dispatch.payload,
        correlation_id: None,
        from: Some(addr(&run())),
        error: "made up".into(),
        attempts: 1,
        cause: FailureCause::Commit,
    };
    let version = test.version(&run()).await;
    let m = test.send(&run(), JobGraphMsg::Failed(planted)).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(statuses(&test).await, named(&[("a", Running), ("b", Waiting)]));
    assert_eq!(test.version(&run()).await, version);
}

/// Sends `message` to the run and checks that it is refused for good, with the state unchanged; returns the error.
async fn assert_refused(test: &TestActors, message: JobGraphMsg) -> String {
    let before = state(test).await;
    let version = test.version(&run()).await;
    let m = test.send(&run(), message).await;
    test.run_until_idle().await;
    assert_eq!(status(test, &m).await, Some(OutcomeStatus::Failed));
    let failed = told(test).await.into_iter().find(|f| f.message_id == m.id).expect("a Failed");
    assert_eq!((failed.cause, failed.attempts), (FailureCause::Refused, 1), "{}", failed.error);
    assert_eq!(test.version(&run()).await, version);
    assert_eq!(state(test).await, before);
    failed.error
}

/// A `Result` for `job` and `attempt`.
fn result(job: &str, attempt: u32) -> JobGraphMsg {
    JobGraphMsg::Result { job: job.into(), attempt, outcome: JobOutcome::Succeeded, output: None, reason: None }
}

/// Scenario: A result for another attempt.
#[tokio::test(start_paused = true)]
async fn a_result_for_another_attempt_is_refused_for_good() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let error = assert_refused(&test, result("a", 2)).await;
    assert!(error.contains("attempt 2") && error.contains("current attempt 1"), "{error}");
    assert_refused(&test, result("a", 0)).await;
    let error = assert_refused(&test, result("nope", 1)).await;
    assert!(error.contains("no job named 'nope'"), "{error}");
    assert_eq!(statuses(&test).await, named(&[("a", Running)]));
}

/// Scenario: A late result. And a result for a job that never started.
#[tokio::test(start_paused = true)]
async fn a_late_result_is_refused_for_good() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &["a"]), job("c", &["b"])], 2).await;
    succeed(&test, "a").await;
    let late = JobGraphMsg::Result {
        job: "a".into(),
        attempt: 1,
        outcome: JobOutcome::Failed,
        output: Some(json!("other")),
        reason: None,
    };
    let error = assert_refused(&test, late).await;
    assert!(error.contains("not running: it is succeeded"), "{error}");
    assert_eq!(state(&test).await.jobs[0].output, Some(json!({"built": "a"})));
    let error = assert_refused(&test, result("c", 1)).await;
    assert!(error.contains("it is waiting"), "{error}");
}

/// Scenario: A result from another actor.
#[tokio::test(start_paused = true)]
async fn a_result_from_another_actor_is_taken() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let start = start_of(&test, "a").await;
    let result = JobGraphMsg::succeeded(&start, json!(1)).unwrap();
    test.send(&builder("other"), BuilderMsg::Relay { run: "run-7".into(), result }).await;
    test.run_until_idle().await;
    let state = state(&test).await;
    assert_eq!((state.jobs[0].status, state.jobs[0].output.clone()), (Succeeded, Some(json!(1))));
}

/// Scenario: An output too large. And an output too deep, a reason too long and a job name out of bounds.
#[tokio::test(start_paused = true)]
async fn a_result_out_of_bounds_is_refused_for_good() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let start = start_of(&test, "a").await;
    let big = JobGraphMsg::succeeded(&start, "x".repeat(17 * 1024)).unwrap();
    let error = assert_refused(&test, big).await;
    assert!(error.contains("output is at most 16384 bytes"), "{error}");
    let deep = (0..33).fold(Value::Null, |inner, _| Value::Array(vec![inner]));
    let error = assert_refused(&test, JobGraphMsg::succeeded(&start, deep).unwrap()).await;
    assert!(error.contains("levels"), "{error}");
    let long = JobGraphMsg::Result {
        job: "a".into(),
        attempt: 1,
        outcome: JobOutcome::Failed,
        output: None,
        reason: Some("r".repeat(2_001)),
    };
    let error = assert_refused(&test, long).await;
    assert!(error.contains("at most 2000 characters"), "{error}");
    for name in ["", "a b"] {
        let error = assert_refused(&test, result(name, 1)).await;
        assert!(error.contains("job name"), "{error}");
        assert_refused(&test, JobGraphMsg::Dispatch { job: name.into(), attempt: 1 }).await;
    }
    assert_eq!(statuses(&test).await, named(&[("a", Running)]));
}

/// Scenario: A failed job: its dependents are skipped, through any number of jobs, in the commit of its result.
#[tokio::test(start_paused = true)]
async fn a_failed_job_skips_its_dependents_transitively_and_only_them() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &["a"]), job("c", &["b"]), job("d", &[])], 2).await;
    assert_eq!(started(&test, "a").await, ["a", "d"]);
    let start = start_of(&test, "a").await;
    test.send(&run(), JobGraphMsg::failed(&start, "a broke")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Skipped), ("c", Skipped), ("d", Running)]));
    let mid = state(&test).await;
    assert!(mid.jobs[1..3].iter().all(|j| j.attempts.is_empty()), "a skipped job has no attempt");
    assert_eq!(mid.status, JobGraphStatus::Running);
    succeed(&test, "d").await;
    let state = state(&test).await;
    assert_eq!(state.status, JobGraphStatus::Failed);
    assert!(state.ended_at.is_some());
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Skipped), ("c", Skipped), ("d", Succeeded)]));
    assert_eq!(started(&test, "a").await, ["a", "d"], "b and c never start");
    let report = &reports(&test, "main").await[0];
    let skipped: Vec<_> = report.jobs.iter().filter(|j| j.status == Skipped).map(|j| j.attempt).collect();
    assert_eq!(skipped, [None, None]);
}

/// Scenario: Every job succeeds: the run ends in the commit of the last result, and its report follows in a turn of its
/// own.
#[tokio::test(start_paused = true)]
async fn the_run_ends_in_the_commit_of_its_last_result() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let start = start_of(&test, "a").await;
    test.send(&run(), JobGraphMsg::succeeded(&start, ()).unwrap()).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    let state = state(&test).await;
    assert_eq!((state.status, state.ended_at.is_some()), (JobGraphStatus::Succeeded, true));
    let pending: Vec<String> = test.pending(&run()).await.into_iter().map(|m| m.message_type).collect();
    assert_eq!(pending, ["Report"]);
    assert!(reports(&test, "main").await.is_empty());
    test.run_until_idle().await;
    assert_eq!(reports(&test, "main").await.len(), 1);
}

/// Scenario: The end report.
#[tokio::test(start_paused = true)]
async fn the_end_report_names_each_job_with_its_status_output_and_reason() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 2).await;
    succeed(&test, "a").await;
    fail(&test, "b").await;
    let reports = reports(&test, "main").await;
    let expected: JobGraphEnd = serde_json::from_value(json!({"run": "run-7", "status": "failed", "jobs": [
        {"name": "a", "status": "succeeded", "attempt": 1, "output": {"built": "a"}, "reason": null},
        {"name": "b", "status": "failed", "attempt": 1, "output": null, "reason": "b broke"}]}))
    .unwrap();
    assert_eq!(reports, [expected]);
}

/// Scenario: A report sent again. And a `Report` to a running run, which does nothing.
#[tokio::test(start_paused = true)]
async fn a_report_sent_again_sends_the_same_report() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let version = test.version(&run()).await;
    test.send(&run(), JobGraphMsg::Report).await;
    test.run_until_idle().await;
    assert!(reports(&test, "main").await.is_empty(), "a running run reports nothing");
    assert_eq!(test.version(&run()).await, version);
    succeed(&test, "a").await;
    let version = test.version(&run()).await;
    test.send(&run(), JobGraphMsg::Report).await;
    test.run_until_idle().await;
    let reports = reports(&test, "main").await;
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0], reports[1]);
    assert_eq!(test.version(&run()).await, version);
}

/// Scenario: A gone report target.
#[tokio::test(start_paused = true)]
async fn a_gone_report_target_fails_only_the_report_and_a_later_report_sends_it() {
    let test = node().await;
    let state = JobGraphState::new(vec![job("a", &[])], 2, &ci("gone"), "GraphEnded").unwrap();
    test.create(&run(), &state).await;
    test.send(&run(), JobGraphMsg::Start).await;
    test.run_until_idle().await;
    let m = succeed(&test, "a").await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "the result is taken");
    assert_eq!(run_status(&test).await, JobGraphStatus::Succeeded);
    let failed = failed_of(&test, "Report").await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].cause, FailureCause::Commit);
    test.create(&ci("gone"), &Reports::default()).await;
    test.send(&run(), JobGraphMsg::Report).await;
    test.run_until_idle().await;
    let reports = reports(&test, "gone").await;
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, JobGraphStatus::Succeeded);
}

/// The job message and the report carry the correlation id of the message that made them, with a retry policy.
#[tokio::test(start_paused = true)]
async fn a_job_message_carries_the_correlation_id_of_the_start() {
    let test = node().await;
    create(&test, vec![job("a", &[])], 2).await;
    test.system().pause_mailbox(&addr(&builder("a"))).await.unwrap();
    test.send_with(&run(), JobGraphMsg::Start, SendOptions::new().correlation_id("c1")).await;
    test.run_until_idle().await;
    let pending = test.pending(&builder("a")).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].correlation_id.as_deref(), Some("c1"));
    assert!(pending[0].retry.is_some(), "with the shared retry policy");
    let of_builder = test.system().watches(None, Some(&WatchTarget::from(addr(&builder("a")))), None, 10).await;
    let watchers: Vec<String> = of_builder.unwrap().items.iter().map(|w| w.watcher.to_string()).collect();
    assert_eq!(watchers, ["cicada.job-graph/run-7"], "the turn that sent the job message watches its target");
}

/// The watchers of `builder/{id}`.
async fn watchers_of(test: &TestActors, id: &str) -> Vec<String> {
    let target = WatchTarget::from(addr(&builder(id)));
    let watches = test.system().watches(None, Some(&target), None, 10).await.unwrap().items;
    watches.iter().map(|w| w.watcher.to_string()).collect()
}

/// The reason of the last attempt of the job at `i`.
async fn reason_of(test: &TestActors, i: usize) -> String {
    state(test).await.jobs[i].attempts.last().and_then(|a| a.reason.clone()).expect("a reason")
}

/// Scenario: The job message fails at the worker.
#[tokio::test(start_paused = true)]
async fn a_job_message_that_fails_at_the_worker_fails_its_job_and_skips_its_dependents() {
    let test = node().await;
    let mut a = job("a", &[]);
    a.input = json!({"fail": true});
    start(&test, vec![a, job("b", &["a"]), job("c", &[])], 2).await;
    assert_eq!(statuses(&test).await, named(&[("a", Running), ("b", Waiting), ("c", Running)]));
    test.advance_time_by(std::time::Duration::from_secs(600)).await;
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Skipped), ("c", Running)]));
    let reason = reason_of(&test, 0).await;
    assert!(reason.starts_with("the job message failed at the worker (handler): "), "{reason}");
    assert!(reason.contains("the build broke"), "{reason}");
    assert_eq!(watchers_of(&test, "a").await, ["cicada.job-graph/run-7"], "c still runs on builder/a");
    succeed(&test, "c").await;
    assert_eq!(run_status(&test).await, JobGraphStatus::Failed);
    assert!(watchers_of(&test, "a").await.is_empty(), "unwatched once no running job has it");
}

/// Scenario: The worker is deleted.
#[tokio::test(start_paused = true)]
async fn a_deleted_worker_fails_every_job_dispatched_to_it() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[]), job("c", &["a"])], 2).await;
    assert_eq!(started(&test, "a").await, ["a", "b"]);
    test.system().delete_actor(&addr(&builder("a"))).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Failed), ("c", Skipped)]));
    for i in [0, 1] {
        assert_eq!(reason_of(&test, i).await, "the worker went away (deleted)");
    }
    assert_eq!(run_status(&test).await, JobGraphStatus::Failed);
    assert_eq!(reports(&test, "main").await.len(), 1);
}

/// Scenario: A slot freed by a gone worker: `b` starts in the commit that fails `a`.
#[tokio::test(start_paused = true)]
async fn a_slot_freed_by_a_gone_worker_starts_the_next_job() {
    let test = node().await;
    start(&test, vec![job_to("a", "builder/b"), job("b", &[])], 1).await;
    assert_eq!(statuses(&test).await, named(&[("a", Running), ("b", Waiting)]));
    test.system().delete_actor(&addr(&builder("b"))).await.unwrap();
    let pending: Vec<String> = test.pending(&run()).await.into_iter().map(|m| m.message_type).collect();
    assert_eq!(pending, ["Stopped"]);
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Running)]));
    test.run_until_idle().await;
    assert_eq!(started(&test, "a").await, ["b"]);
}

/// Scenario: Another message of the worker fails.
#[tokio::test(start_paused = true)]
async fn another_failed_message_of_a_shared_worker_changes_no_job() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let version = test.version(&run()).await;
    let start = start_of(&test, "a").await;
    let result = JobGraphMsg::succeeded(&start, ()).unwrap();
    let orphan = test.send(&builder("a"), BuilderMsg::Relay { run: "ghost".into(), result }).await;
    let fails = JobStart { input: json!({"fail": true}), ..start };
    let other = test.send(&builder("a"), BuilderMsg::Build(fails)).await;
    test.advance_time_by(std::time::Duration::from_secs(600)).await;
    assert_eq!(status(&test, &orphan).await, Some(OutcomeStatus::Failed));
    assert_eq!(status(&test, &other).await, Some(OutcomeStatus::Failed), "a copy of a's job message, from a client");
    assert_eq!(statuses(&test).await, named(&[("a", Running)]));
    assert_eq!(test.version(&run()).await, version);
    succeed(&test, "a").await;
    assert_eq!(run_status(&test).await, JobGraphStatus::Succeeded);
}

/// Scenario: A worker that reports and stops, in the normal order: the result first, then the `Stopped`.
#[tokio::test(start_paused = true)]
async fn a_worker_that_reports_and_stops_in_one_turn_has_its_result_taken() {
    let test = node().await;
    start(&test, vec![job_to("a", "builder/b")], 2).await;
    let start = starts(&test, "b").await.pop().expect("a job message");
    let result = JobGraphMsg::succeeded(&start, json!("done")).unwrap();
    test.send(&builder("b"), BuilderMsg::ReportAndStop { run: "run-7".into(), result }).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the worker's turn");
    let pending: Vec<String> = test.pending(&run()).await.into_iter().map(|m| m.message_type).collect();
    assert_eq!(pending, ["Result", "Stopped"]);
    test.run_until_idle().await;
    let state = state(&test).await;
    assert_eq!((state.jobs[0].status, state.jobs[0].output.clone()), (Succeeded, Some(json!("done"))));
    assert_eq!(state.status, JobGraphStatus::Succeeded);
}

/// Scenario: A notification a client sent: a `Failed` that names a running job's message, and a `Stopped` about its
/// target.
#[tokio::test(start_paused = true)]
async fn notifications_a_client_sent_change_no_job() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let start = start_of(&test, "a").await;
    let failed = cicada::Failed {
        actor: addr(&builder("a")),
        message_id: MessageId::new("m-planted").unwrap(),
        message_type: "Build".into(),
        payload: serde_json::to_value(&start).unwrap(),
        correlation_id: None,
        from: Some(addr(&run())),
        error: "made up".into(),
        attempts: 7,
        cause: FailureCause::Handler,
    };
    let stopped = cicada::Stopped { actor: addr(&builder("a")), cause: cicada::StopCause::Deleted, by: None };
    let version = test.version(&run()).await;
    for message in [JobGraphMsg::Failed(failed), JobGraphMsg::Stopped(stopped)] {
        let m = test.send(&run(), message).await;
        test.run_until_idle().await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    }
    assert_eq!(statuses(&test).await, named(&[("a", Running)]));
    assert_eq!(test.version(&run()).await, version);
}

/// Scenario: A failed dispatch of a job that is no longer running: a client's result ends the job before the `Failed`
/// of its `Dispatch` is handled, and that `Failed` changes nothing.
#[tokio::test(start_paused = true)]
async fn a_failed_dispatch_of_a_job_that_got_its_result_first_changes_nothing() {
    let test = node().await;
    create(&test, vec![job_to("a", "builder/ghost")], 2).await;
    test.send(&run(), JobGraphMsg::Start).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    test.send(&run(), result("a", 1)).await;
    let pending: Vec<String> = test.pending(&run()).await.into_iter().map(|m| m.message_type).collect();
    assert_eq!(pending, ["Dispatch", "Result"]);
    test.advance_time_by(std::time::Duration::from_secs(600)).await;
    assert_eq!(failed_of(&test, "Dispatch").await.len(), 1);
    let state = state(&test).await;
    assert_eq!(state.jobs[0].status, Succeeded);
    assert_eq!(state.jobs[0].attempts[0].dispatched_at, None, "a result taken before its dispatch");
    assert_eq!(state.status, JobGraphStatus::Succeeded);
}

/// The run watches a target from the turn that sends a job message to it, and unwatches it once none of its running
/// jobs has it.
#[tokio::test(start_paused = true)]
async fn the_run_unwatches_a_target_once_no_running_job_has_it() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[]), job_to("c", "builder/b")], 3).await;
    assert_eq!(watchers_of(&test, "a").await, ["cicada.job-graph/run-7"]);
    assert_eq!(watchers_of(&test, "b").await, ["cicada.job-graph/run-7"]);
    succeed(&test, "a").await;
    assert_eq!(watchers_of(&test, "a").await, ["cicada.job-graph/run-7"], "b still runs on builder/a");
    let start = starts(&test, "b").await.pop().unwrap();
    test.send(&run(), JobGraphMsg::failed(&start, "c broke")).await;
    test.run_until_idle().await;
    assert!(watchers_of(&test, "b").await.is_empty());
    succeed(&test, "b").await;
    assert!(watchers_of(&test, "a").await.is_empty());
    let watches = test.system().watches(Some(&addr(&run())), None, None, 10).await.unwrap().items;
    let targets: Vec<String> = watches.iter().map(|w| w.target.to_string()).collect();
    assert_eq!(targets, ["cicada.job-graph/run-7"], "the run keeps its watch on itself");
}
