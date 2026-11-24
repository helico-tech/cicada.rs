//! The job graph, on virtual time with `TestActors` (ADR 0059, ADR 0060): ready jobs in graph order up to the in-flight
//! limit, each job message in a turn of its own, a failed dispatch that fails only its attempt, the attempt fence on
//! results, a failed job that skips its dependents, the watch on each target, `Cancel` and the cancel message, `Retry`
//! and the numbered end report.

mod common;

use cicada::prelude::*;
use cicada::store::{Message, OutcomeStatus};
use cicada::{FailureCause, WatchTarget};
use cicada_actors::{
    AttemptOutcome, Job, JobCancel, JobGraph, JobGraphActor, JobGraphEnd, JobGraphMsg, JobGraphState, JobGraphStatus,
    JobOutcome, JobStart, JobStatus,
};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// `builder/{id}`: keeps every job message and cancel message it is sent, fails its handler on a job message whose
/// input has `"fail": true` or whose attempt is the input's `"failAttempt"`, and relays a result to a run when told.
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
    CancelBuild(JobCancel),
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
    #[serde(default)]
    cancels: Vec<JobCancel>,
}

struct BuilderActor;

impl Actor for BuilderActor {
    type Ref = Builder;
    type State = Starts;

    async fn handle(&self, turn: &mut Turn<Self>, message: BuilderMsg) -> HandlerResult {
        match message {
            BuilderMsg::Build(start)
                if start.input["fail"] == json!(true) || start.input["failAttempt"] == start.attempt =>
            {
                return Err(HandlerError::msg("the build broke"));
            }
            BuilderMsg::Build(start) => turn.state_mut().log.push(start),
            BuilderMsg::CancelBuild(cancel) => turn.state_mut().cancels.push(cancel),
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

use JobStatus::{Cancelled, Failed, Running, Skipped, Succeeded, Waiting};

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
    let expected: JobGraphEnd = serde_json::from_value(json!({"run": "run-7", "end": 1, "status": "failed", "jobs": [
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
    assert_eq!(reports[1].end, 1, "the same end number");
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

/// A `Cancel` of `job`.
fn cancel(job: &str) -> JobGraphMsg {
    JobGraphMsg::Cancel { job: Some(job.into()) }
}

/// A `Cancel` of every job.
fn cancel_all() -> JobGraphMsg {
    JobGraphMsg::Cancel { job: None }
}

/// The cancel messages `builder/{id}` got.
async fn cancels(test: &TestActors, id: &str) -> Vec<JobCancel> {
    test.state::<_, Starts>(&builder(id)).await.cancels
}

/// The types of the messages waiting for the run.
async fn pending_types(test: &TestActors) -> Vec<String> {
    test.pending(&run()).await.into_iter().map(|m| m.message_type).collect()
}

/// Scenario: Cancel one running job: its dependent is skipped in the commit of the `Cancel`, its attempt ends
/// `cancelled`, its target is unwatched, and a later result for its attempt is refused for good.
#[tokio::test(start_paused = true)]
async fn cancel_of_one_running_job_skips_its_dependent_and_refuses_its_late_result() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &["a"])], 2).await;
    let start = start_of(&test, "a").await;
    let m = test.send(&run(), cancel("a")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(statuses(&test).await, named(&[("a", Cancelled), ("b", Skipped)]));
    let state = state(&test).await;
    let attempt = &state.jobs[0].attempts[0];
    assert_eq!((attempt.outcome, attempt.reason.as_deref()), (Some(AttemptOutcome::Cancelled), Some("cancelled")));
    assert!(attempt.ended_at.is_some() && attempt.dispatched_at.is_some());
    assert_eq!(state.status, JobGraphStatus::Cancelled, "nothing runs or can start, and no job failed");
    assert!(watchers_of(&test, "a").await.is_empty());
    test.run_until_idle().await;
    assert!(cancels(&test, "a").await.is_empty(), "a job without a cancelType is not told");
    let error = assert_refused(&test, JobGraphMsg::succeeded(&start, ()).unwrap()).await;
    assert!(error.contains("not running: it is cancelled"), "{error}");
}

/// Scenario: Cancel the whole run: the running and waiting jobs are cancelled, the succeeded one stays, and the run
/// ends `cancelled` and is reported.
#[tokio::test(start_paused = true)]
async fn cancel_of_the_whole_run_cancels_every_waiting_and_running_job_and_reports() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[]), job("c", &["b"]), job("d", &["b"])], 2).await;
    succeed(&test, "a").await;
    assert_eq!(statuses(&test).await, named(&[("a", Succeeded), ("b", Running), ("c", Waiting), ("d", Waiting)]));
    test.send(&run(), cancel_all()).await;
    test.run_until_idle().await;
    assert_eq!(statuses(&test).await, named(&[("a", Succeeded), ("b", Cancelled), ("c", Cancelled), ("d", Cancelled)]));
    assert_eq!(run_status(&test).await, JobGraphStatus::Cancelled);
    let expected: JobGraphEnd =
        serde_json::from_value(json!({"run": "run-7", "end": 1, "status": "cancelled", "jobs": [
        {"name": "a", "status": "succeeded", "attempt": 1, "output": {"built": "a"}, "reason": null},
        {"name": "b", "status": "cancelled", "attempt": 1, "output": null, "reason": "cancelled"},
        {"name": "c", "status": "cancelled", "attempt": null, "output": null, "reason": null},
        {"name": "d", "status": "cancelled", "attempt": null, "output": null, "reason": null}]}))
        .unwrap();
    assert_eq!(reports(&test, "main").await, [expected]);
}

/// Scenario: Cancel the whole run after a failure: it ends `failed`.
#[tokio::test(start_paused = true)]
async fn cancel_of_the_whole_run_after_a_failure_ends_it_failed() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 2).await;
    fail(&test, "a").await;
    test.send(&run(), cancel_all()).await;
    test.run_until_idle().await;
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Cancelled)]));
    assert_eq!(run_status(&test).await, JobGraphStatus::Failed);
    assert_eq!(reports(&test, "main").await[0].status, JobGraphStatus::Failed);
}

/// Scenario: Cancel one job of a run that otherwise succeeds: it ends `cancelled`.
#[tokio::test(start_paused = true)]
async fn cancel_of_one_job_of_a_run_that_otherwise_succeeds_ends_it_cancelled() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 2).await;
    test.send(&run(), cancel("a")).await;
    test.run_until_idle().await;
    assert_eq!(run_status(&test).await, JobGraphStatus::Running);
    assert_eq!(watchers_of(&test, "a").await, ["cicada.job-graph/run-7"], "b still runs on builder/a");
    succeed(&test, "b").await;
    assert_eq!(statuses(&test).await, named(&[("a", Cancelled), ("b", Succeeded)]));
    assert_eq!(run_status(&test).await, JobGraphStatus::Cancelled);
}

/// Scenario: A cancel frees a slot: the next job starts in the commit of the `Cancel`.
#[tokio::test(start_paused = true)]
async fn a_cancel_frees_a_slot_for_the_next_job_in_its_commit() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 1).await;
    test.send(&run(), cancel("a")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(statuses(&test).await, named(&[("a", Cancelled), ("b", Running)]));
    assert_eq!(pending_types(&test).await, ["Dispatch"]);
    test.run_until_idle().await;
    assert_eq!(started(&test, "a").await, ["a", "b"]);
}

/// Scenario: Cancel of an ended job. And of a skipped job, a repeated `Cancel` and a `Cancel` of an ended run, which
/// change nothing either.
#[tokio::test(start_paused = true)]
async fn cancel_of_an_ended_or_skipped_job_or_an_ended_run_changes_nothing() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[]), job("c", &["b"]), job("d", &[])], 3).await;
    succeed(&test, "a").await;
    fail(&test, "b").await;
    test.send(&run(), cancel("d")).await;
    test.run_until_idle().await;
    assert_eq!(statuses(&test).await, named(&[("a", Succeeded), ("b", Failed), ("c", Skipped), ("d", Cancelled)]));
    let reported = reports(&test, "main").await.len();
    let version = test.version(&run()).await;
    for message in [cancel("a"), cancel("b"), cancel("c"), cancel("d"), cancel_all()] {
        let m = test.send(&run(), message).await;
        test.run_until_idle().await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    }
    assert_eq!(test.version(&run()).await, version);
    assert_eq!(reports(&test, "main").await.len(), reported);

    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 2).await;
    succeed(&test, "a").await;
    let version = test.version(&run()).await;
    let m = test.send(&run(), cancel("a")).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(test.version(&run()).await, version, "a running run, a succeeded job");
    assert_eq!(statuses(&test).await, named(&[("a", Succeeded), ("b", Running)]));
}

/// Scenario: Cancel of an unknown job. And of a job name out of bounds.
#[tokio::test(start_paused = true)]
async fn cancel_of_an_unknown_job_is_refused_for_good() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let error = assert_refused(&test, cancel("nope")).await;
    assert!(error.contains("no job named 'nope'"), "{error}");
    for name in ["", "a b"] {
        let error = assert_refused(&test, cancel(name)).await;
        assert!(error.contains("job name"), "{error}");
        assert_refused(&test, JobGraphMsg::SendCancel { job: name.into(), attempt: 1 }).await;
    }
    assert_eq!(statuses(&test).await, named(&[("a", Running)]));
}

/// Scenario: A failed dispatch reported after the cancel: the job stays `cancelled`.
#[tokio::test(start_paused = true)]
async fn a_failed_dispatch_reported_after_the_cancel_leaves_the_job_cancelled() {
    let test = node().await;
    create(&test, vec![job_to("a", "builder/ghost").with_cancel_type("CancelBuild")], 2).await;
    test.send(&run(), JobGraphMsg::Start).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    test.send(&run(), cancel("a")).await;
    assert_eq!(pending_types(&test).await, ["Dispatch", "Cancel"]);
    test.advance_time_by(std::time::Duration::from_secs(600)).await;
    assert_eq!(failed_of(&test, "Dispatch").await.len(), 1);
    assert!(failed_of(&test, "SendCancel").await.is_empty(), "a job not dispatched is not told");
    let state = state(&test).await;
    assert_eq!(state.jobs[0].status, Cancelled);
    let attempt = &state.jobs[0].attempts[0];
    assert_eq!((attempt.outcome, attempt.dispatched_at), (Some(AttemptOutcome::Cancelled), None));
    assert_eq!(state.status, JobGraphStatus::Cancelled);
}

/// Scenario: A worker is told: its target gets the cancel type with the run, the job and the attempt, in a later turn
/// of the run, with the shared retry policy and the correlation id of the `Cancel`.
#[tokio::test(start_paused = true)]
async fn a_worker_is_told_through_the_jobs_cancel_type_in_a_later_turn() {
    let test = node().await;
    start(&test, vec![job("a", &[]).with_cancel_type("CancelBuild")], 2).await;
    test.system().pause_mailbox(&addr(&builder("a"))).await.unwrap();
    test.send_with(&run(), cancel("a"), SendOptions::new().correlation_id("c9")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(statuses(&test).await, named(&[("a", Cancelled)]));
    let pending = test.pending(&run()).await;
    let send_cancel = pending.iter().find(|m| m.message_type == "SendCancel").expect("a SendCancel");
    assert_eq!(send_cancel.payload, json!({"job": "a", "attempt": 1}));
    assert!(test.pending(&builder("a")).await.is_empty(), "not in the commit of the Cancel");
    test.run_until_idle().await;
    let told = test.pending(&builder("a")).await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].message_type, "CancelBuild");
    assert_eq!(told[0].payload, json!({"run": "run-7", "job": "a", "attempt": 1}));
    assert_eq!(told[0].correlation_id.as_deref(), Some("c9"));
    assert!(told[0].retry.is_some(), "with the shared retry policy");
    test.system().resume_mailbox(&addr(&builder("a"))).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(cancels(&test, "a").await, [JobCancel { run: "run-7".into(), job: "a".into(), attempt: 1 }]);
}

/// Scenario: A gone worker: the `Cancel` commits and only the `SendCancel` fails.
#[tokio::test(start_paused = true)]
async fn a_gone_workers_send_cancel_fails_alone() {
    let test = node().await;
    start(&test, vec![job("a", &[]).with_cancel_type("CancelBuild"), job("b", &["a"])], 2).await;
    let m = test.send(&run(), cancel("a")).await;
    test.system().delete_actor(&addr(&builder("a"))).await.unwrap();
    assert_eq!(pending_types(&test).await, ["Cancel", "Stopped"]);
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(statuses(&test).await, named(&[("a", Cancelled), ("b", Skipped)]));
    let failed = failed_of(&test, "SendCancel").await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].cause, FailureCause::Commit);
    assert_eq!(run_status(&test).await, JobGraphStatus::Cancelled);
    assert_eq!(reports(&test, "main").await.len(), 1);
}

/// Scenario: A job not yet dispatched: its target gets neither the job message nor the cancel message.
#[tokio::test(start_paused = true)]
async fn a_job_cancelled_before_its_dispatch_is_never_sent_anything() {
    let test = node().await;
    create(&test, vec![job("a", &[]).with_cancel_type("CancelBuild")], 2).await;
    test.send(&run(), JobGraphMsg::Start).await;
    test.send(&run(), cancel("a")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(statuses(&test).await, named(&[("a", Running)]));
    assert_eq!(pending_types(&test).await, ["Cancel", "Dispatch"], "the Cancel runs before the Dispatch");
    test.run_until_idle().await;
    assert_eq!(statuses(&test).await, named(&[("a", Cancelled)]));
    assert!(started(&test, "a").await.is_empty());
    assert!(cancels(&test, "a").await.is_empty());
    assert!(failed_of(&test, "Dispatch").await.is_empty(), "the Dispatch sends nothing and is done");
    assert_eq!(state(&test).await.jobs[0].attempts[0].dispatched_at, None);
}

/// A `SendCancel` the run did not send itself sends nothing, also for an attempt that was dispatched and cancelled.
#[tokio::test(start_paused = true)]
async fn a_send_cancel_sent_by_a_client_sends_nothing() {
    let test = node().await;
    start(&test, vec![job("a", &[]).with_cancel_type("CancelBuild"), job("b", &[]).with_cancel_type("CancelBuild")], 2)
        .await;
    test.send(&run(), cancel("a")).await;
    test.run_until_idle().await;
    assert_eq!(cancels(&test, "a").await.len(), 1);
    let version = test.version(&run()).await;
    for attempt in [1, 2] {
        for job in ["a", "b"] {
            let m = test.send(&run(), JobGraphMsg::SendCancel { job: job.into(), attempt }).await;
            test.run_until_idle().await;
            assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
        }
    }
    assert_eq!(cancels(&test, "a").await.len(), 1, "no second cancel message");
    assert_eq!(test.version(&run()).await, version);
}

/// A `Retry` of `job`.
fn retry(job: &str) -> JobGraphMsg {
    JobGraphMsg::Retry { job: job.into() }
}

/// The attempt numbers of the job at `i`.
async fn attempts_of(test: &TestActors, i: usize) -> Vec<u32> {
    state(test).await.jobs[i].attempts.iter().map(|a| a.attempt).collect()
}

/// Scenario: Retry a failed job: it starts with attempt 2 in the commit of the `Retry`, its output is cleared, and its
/// job message names attempt 2.
#[tokio::test(start_paused = true)]
async fn retry_of_a_failed_job_starts_attempt_two_in_its_commit() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 2).await;
    let failed = JobGraphMsg::Result {
        job: "a".into(),
        attempt: 1,
        outcome: JobOutcome::Failed,
        output: Some(json!({"log": "exit 1"})),
        reason: Some("exit 1".into()),
    };
    test.send(&run(), failed).await;
    test.run_until_idle().await;
    assert_eq!(state(&test).await.jobs[0].output, Some(json!({"log": "exit 1"})));
    test.send(&run(), retry("a")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    let state = state(&test).await;
    let a = &state.jobs[0];
    assert_eq!((a.status, a.output.as_ref()), (Running, None), "running again, its output cleared");
    assert_eq!(attempts_of(&test, 0).await, [1, 2], "the old attempt is kept");
    assert_eq!(a.attempts[0].reason.as_deref(), Some("exit 1"));
    let pending = test.pending(&run()).await;
    let dispatch = pending.iter().find(|m| m.message_type == "Dispatch").expect("a Dispatch");
    assert_eq!(dispatch.payload, json!({"job": "a", "attempt": 2}));
    test.run_until_idle().await;
    assert_eq!(start_of(&test, "a").await.attempt, 2);
    assert_eq!(watchers_of(&test, "a").await, ["cicada.job-graph/run-7"]);
    succeed(&test, "a").await;
    succeed(&test, "b").await;
    assert_eq!(run_status(&test).await, JobGraphStatus::Succeeded);
}

/// Scenario: Skipped dependents come back: those skipped only because of the retried job wait again, one that also
/// needs another failed job stays skipped.
#[tokio::test(start_paused = true)]
async fn retry_brings_back_the_dependents_skipped_only_because_of_the_retried_job() {
    let test = node().await;
    let jobs = vec![job("a", &[]), job("x", &[]), job("b", &["a"]), job("c", &["b"]), job("d", &["a", "x"])];
    start(&test, jobs, 2).await;
    fail(&test, "a").await;
    fail(&test, "x").await;
    assert_eq!(
        statuses(&test).await,
        named(&[("a", Failed), ("x", Failed), ("b", Skipped), ("c", Skipped), ("d", Skipped)])
    );
    assert_eq!(run_status(&test).await, JobGraphStatus::Failed);
    test.send(&run(), retry("a")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(
        statuses(&test).await,
        named(&[("a", Running), ("x", Failed), ("b", Waiting), ("c", Waiting), ("d", Skipped)])
    );
    test.run_until_idle().await;
    succeed(&test, "a").await;
    succeed(&test, "b").await;
    succeed(&test, "c").await;
    assert_eq!(
        statuses(&test).await,
        named(&[("a", Succeeded), ("x", Failed), ("b", Succeeded), ("c", Succeeded), ("d", Skipped)])
    );
    assert_eq!(run_status(&test).await, JobGraphStatus::Failed);
}

/// Scenario: A late result of a replaced attempt: refused for good, and the job still runs its new attempt.
#[tokio::test(start_paused = true)]
async fn a_late_result_of_a_replaced_attempt_is_refused_for_good() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let first = start_of(&test, "a").await;
    fail(&test, "a").await;
    test.send(&run(), retry("a")).await;
    test.run_until_idle().await;
    let error = assert_refused(&test, JobGraphMsg::succeeded(&first, ()).unwrap()).await;
    assert!(error.contains("attempt 1") && error.contains("current attempt 2"), "{error}");
    assert_eq!(statuses(&test).await, named(&[("a", Running)]));
    assert_eq!(attempts_of(&test, 0).await, [1, 2]);
}

/// A worker's `Failed` about the job message of a replaced attempt is ignored, and so is, for the retried job, one about
/// another job on the same worker, which fails only that job.
#[tokio::test(start_paused = true)]
async fn a_failed_about_a_replaced_attempt_or_another_job_on_a_shared_worker_leaves_the_retried_job_running() {
    let test = node().await;
    let mut a = job("a", &[]);
    a.input = json!({"job": "a", "failAttempt": 1});
    let mut b = job("b", &[]);
    b.input = json!({"job": "b", "fail": true});
    start(&test, vec![a, b], 2).await;
    // Attempt 1's job message is still being tried at the worker when its failed result comes in.
    let first = JobStart { run: "run-7".into(), job: "a".into(), attempt: 1, input: json!({}) };
    test.send(&run(), JobGraphMsg::failed(&first, "timed out")).await;
    test.send(&run(), retry("a")).await;
    test.run_until_idle().await;
    assert_eq!(statuses(&test).await, named(&[("a", Running), ("b", Running)]));
    assert_eq!(attempts_of(&test, 0).await, [1, 2]);
    test.advance_time_by(std::time::Duration::from_secs(600)).await;
    let mut failed: Vec<(String, u64)> = (test.failures().await.into_iter())
        .filter(|f| f.envelope.message_type == "Build")
        .map(|f| {
            (f.envelope.payload["job"].as_str().unwrap().to_owned(), f.envelope.payload["attempt"].as_u64().unwrap())
        })
        .collect();
    failed.sort();
    assert_eq!(failed, [("a".to_owned(), 1), ("b".to_owned(), 1)], "both failed at the worker");
    assert_eq!(statuses(&test).await, named(&[("a", Running), ("b", Failed)]));
    assert!(reason_of(&test, 1).await.starts_with("the job message failed at the worker (handler): "));
    assert_eq!(start_of(&test, "a").await.attempt, 2, "the worker took attempt 2");
    succeed(&test, "a").await;
    assert_eq!(statuses(&test).await, named(&[("a", Succeeded), ("b", Failed)]));
    assert_eq!(run_status(&test).await, JobGraphStatus::Failed);
}

/// Scenario: Retry before the cancel notice ran: the `Cancel` and the `Retry` wait while the clock moves on, so the
/// `SendCancel` that the `Cancel` commits is due after the `Retry` (G5), runs after it, and still tells the worker of
/// attempt 1.
#[tokio::test(start_paused = true)]
async fn a_retry_before_the_cancel_notice_ran_still_tells_the_worker_of_the_old_attempt() {
    let test = node().await;
    start(&test, vec![job("a", &[]).with_cancel_type("CancelBuild")], 2).await;
    test.send(&run(), cancel("a")).await;
    test.send(&run(), retry("a")).await;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(statuses(&test).await, named(&[("a", Cancelled)]));
    assert_eq!(pending_types(&test).await, ["Retry", "SendCancel", "Report"]);
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(statuses(&test).await, named(&[("a", Running)]), "attempt 2 starts before the cancel notice ran");
    assert_eq!(attempts_of(&test, 0).await, [1, 2]);
    assert_eq!(pending_types(&test).await[0], "SendCancel");
    test.run_until_idle().await;
    assert_eq!(start_of(&test, "a").await.attempt, 2);
    assert_eq!(cancels(&test, "a").await, [JobCancel { run: "run-7".into(), job: "a".into(), attempt: 1 }]);
    assert!(reports(&test, "main").await.is_empty(), "the Report found the run running again");
}

/// Scenario: Retry after the cancel notice ran: attempt 2 starts, and no second cancel is sent.
#[tokio::test(start_paused = true)]
async fn a_retry_after_the_cancel_notice_ran_sends_no_second_cancel() {
    let test = node().await;
    start(&test, vec![job("a", &[]).with_cancel_type("CancelBuild")], 2).await;
    test.send(&run(), cancel("a")).await;
    test.run_until_idle().await;
    assert_eq!(cancels(&test, "a").await.len(), 1);
    assert_eq!(reports(&test, "main").await.len(), 1);
    test.send(&run(), retry("a")).await;
    test.run_until_idle().await;
    assert_eq!(statuses(&test).await, named(&[("a", Running)]));
    assert_eq!(start_of(&test, "a").await.attempt, 2);
    assert_eq!(cancels(&test, "a").await.len(), 1, "no second cancel message");
}

/// Scenario: Retry within the in-flight limit: the retried job waits for a free slot.
#[tokio::test(start_paused = true)]
async fn a_retried_job_waits_for_a_free_slot() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 1).await;
    fail(&test, "a").await;
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Running)]));
    test.send(&run(), retry("a")).await;
    test.run_until_idle().await;
    assert_eq!(statuses(&test).await, named(&[("a", Waiting), ("b", Running)]));
    assert_eq!(attempts_of(&test, 0).await, [1], "no new attempt yet");
    succeed(&test, "b").await;
    assert_eq!(statuses(&test).await, named(&[("a", Running), ("b", Succeeded)]));
    assert_eq!(start_of(&test, "a").await.attempt, 2);
}

/// Scenario: A retried job cancelled before it starts: it is `cancelled`, its last attempt ended `failed`, and the end
/// report shows it without the old attempt's reason.
#[tokio::test(start_paused = true)]
async fn a_retried_job_cancelled_before_it_starts_is_reported_without_the_old_reason() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 1).await;
    fail(&test, "a").await;
    test.send(&run(), retry("a")).await;
    test.send(&run(), cancel("a")).await;
    test.run_until_idle().await;
    let state = state(&test).await;
    assert_eq!(state.jobs[0].status, Cancelled);
    assert_eq!(state.jobs[0].attempts[0].outcome, Some(AttemptOutcome::Failed));
    succeed(&test, "b").await;
    let expected: JobGraphEnd =
        serde_json::from_value(json!({"run": "run-7", "end": 1, "status": "cancelled", "jobs": [
        {"name": "a", "status": "cancelled", "attempt": 1, "output": null, "reason": null},
        {"name": "b", "status": "succeeded", "attempt": 1, "output": {"built": "b"}, "reason": null}]}))
        .unwrap();
    assert_eq!(reports(&test, "main").await, [expected]);
}

/// Scenario: A run that ends twice: its second report has end 2.
#[tokio::test(start_paused = true)]
async fn a_run_that_ends_twice_reports_end_two() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &[])], 2).await;
    succeed(&test, "b").await;
    fail(&test, "a").await;
    let first = state(&test).await;
    assert_eq!((first.status, first.ends), (JobGraphStatus::Failed, 1));
    test.send(&run(), retry("a")).await;
    test.run_until_idle().await;
    let reopened = state(&test).await;
    assert_eq!((reopened.status, reopened.ended_at, reopened.ends), (JobGraphStatus::Running, None, 1));
    succeed(&test, "a").await;
    let second = state(&test).await;
    assert_eq!((second.status, second.ends), (JobGraphStatus::Succeeded, 2));
    assert!(second.ended_at.is_some());
    let reports = reports(&test, "main").await;
    let ends: Vec<(u32, JobGraphStatus)> = reports.iter().map(|r| (r.end, r.status)).collect();
    assert_eq!(ends, [(1, JobGraphStatus::Failed), (2, JobGraphStatus::Succeeded)]);
    assert_eq!(reports[1].jobs[0].attempt, Some(2));
    assert_eq!(reports[1].jobs[0].reason, None);
}

/// Scenario: A report while running again sends nothing. And an end reopened before its `Report` ran is never
/// reported, so the numbers have a gap.
#[tokio::test(start_paused = true)]
async fn a_report_while_running_again_sends_nothing_and_an_end_reopened_before_its_report_is_skipped() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    let first = start_of(&test, "a").await;
    test.send(&run(), JobGraphMsg::failed(&first, "exit 1")).await;
    test.send(&run(), retry("a")).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the result ends the run");
    assert_eq!(state(&test).await.ends, 1);
    assert_eq!(pending_types(&test).await, ["Retry", "Report"]);
    test.run_until_idle().await;
    assert!(reports(&test, "main").await.is_empty(), "the Report found the run running again");
    let version = test.version(&run()).await;
    test.send(&run(), JobGraphMsg::Report).await;
    test.run_until_idle().await;
    assert!(reports(&test, "main").await.is_empty());
    assert_eq!(test.version(&run()).await, version);
    succeed(&test, "a").await;
    let reports = reports(&test, "main").await;
    assert_eq!(reports.len(), 1);
    assert_eq!((reports[0].end, reports[0].status), (2, JobGraphStatus::Succeeded));
}

/// Scenario: A waiting job with old attempts, decoded from a create, starts with the next attempt number.
#[tokio::test(start_paused = true)]
async fn a_decoded_waiting_job_with_old_attempts_starts_with_the_next_attempt() {
    let test = node().await;
    let old = json!({"attempt": 4, "dispatchedAt": "2026-01-01T00:00:00Z", "endedAt": "2026-01-01T00:00:01Z",
        "outcome": "failed", "reason": "exit 1"});
    let created = json!({"maxInFlight": 2, "reportTo": "ci/main", "reportType": "GraphEnded", "ends": 1, "jobs": [
        {"name": "a", "needs": [], "to": "builder/a", "messageType": "Build", "input": {}, "status": "waiting",
         "attempts": [old]}]});
    test.create(&run(), &created).await;
    test.send(&run(), JobGraphMsg::Start).await;
    test.run_until_idle().await;
    assert_eq!(attempts_of(&test, 0).await, [4, 5]);
    assert_eq!(start_of(&test, "a").await.attempt, 5);
    succeed(&test, "a").await;
    assert_eq!(reports(&test, "main").await[0].end, 2);
}

/// Scenario: Retry of a running job, also a second `Retry` of the same job. And of a waiting, succeeded or skipped job.
#[tokio::test(start_paused = true)]
async fn retry_of_a_job_that_is_not_failed_or_cancelled_is_refused_for_good() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &["a"]), job("c", &[]), job("d", &[])], 2).await;
    fail(&test, "a").await;
    succeed(&test, "c").await;
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Skipped), ("c", Succeeded), ("d", Running)]));
    test.send(&run(), retry("a")).await;
    test.run_until_idle().await;
    let error = assert_refused(&test, retry("a")).await;
    assert!(error.contains("the job 'a' is running"), "{error}");
    let error = assert_refused(&test, retry("b")).await;
    assert!(error.contains("the job 'b' is waiting"), "{error}");
    let error = assert_refused(&test, retry("c")).await;
    assert!(error.contains("is succeeded"), "{error}");
    fail(&test, "a").await;
    let error = assert_refused(&test, retry("b")).await;
    assert!(error.contains("is skipped"), "{error}");
}

/// Scenario: Retry of a job whose need failed: refused with a reason that names the need.
#[tokio::test(start_paused = true)]
async fn retry_of_a_job_whose_need_failed_is_refused_and_names_the_need() {
    let test = node().await;
    start(&test, vec![job("a", &[]), job("b", &["a"])], 2).await;
    test.send(&run(), cancel("b")).await;
    test.run_until_idle().await;
    fail(&test, "a").await;
    assert_eq!(statuses(&test).await, named(&[("a", Failed), ("b", Cancelled)]));
    let error = assert_refused(&test, retry("b")).await;
    assert!(error.contains("'a'") && error.contains("failed") && error.contains("retry its need first"), "{error}");
}

/// Scenario: Retry of an unknown job. And a job name out of bounds.
#[tokio::test(start_paused = true)]
async fn retry_of_an_unknown_job_or_a_name_out_of_bounds_is_refused_for_good() {
    let test = node().await;
    start(&test, vec![job("a", &[])], 2).await;
    fail(&test, "a").await;
    let error = assert_refused(&test, retry("nope")).await;
    assert!(error.contains("no job named 'nope'"), "{error}");
    let error = assert_refused(&test, retry("a b")).await;
    assert!(error.contains("job name"), "{error}");
}

/// A retry is refused when the job has no attempt number left, or the ended run no end number.
#[tokio::test(start_paused = true)]
async fn retry_without_an_attempt_or_end_number_left_is_refused_for_good() {
    for (attempt, ends, why) in [(u32::MAX, 1, "no attempt numbers left"), (1, u32::MAX, "no end numbers left")] {
        let test = node().await;
        let last = json!({"attempt": attempt, "dispatchedAt": null, "endedAt": "2026-01-01T00:00:01Z",
            "outcome": "failed", "reason": null});
        let created = json!({"maxInFlight": 2, "reportTo": "ci/main", "reportType": "GraphEnded", "status": "failed",
            "endedAt": "2026-01-01T00:00:01Z", "ends": ends, "jobs": [
            {"name": "a", "needs": [], "to": "builder/a", "messageType": "Build", "input": {}, "status": "failed",
             "attempts": [last]}]});
        test.create(&run(), &created).await;
        let error = assert_refused(&test, retry("a")).await;
        assert!(error.contains(why), "{error}");
    }
}
