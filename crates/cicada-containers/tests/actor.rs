//! The container actor on virtual time (ADR 0057): `TestActors` and the fake runtime, one scenario of the spec per
//! test, with a target actor that keeps every end it is sent. And its wire format: a test job's contract, whose type
//! name its host chooses, pinned in `tests/contracts`, with samples of every message, the state in each phase and an
//! end payload, taken by the schemas and the decoders (ADR 0026).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::time::{self, MAX_INSTANT, Timestamp};
use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_containers::{
    Call, Container, ContainerActor, ContainerEnd, ContainerEnded, ContainerError, ContainerJob, ContainerMsg,
    ContainerRuntime, ContainerSettings, ContainerSpec, ContainerState, ContainerStatus, ContainerSummary, CopyDir,
    CopyFile, ErrorKind, FailureKind, FakeRuntime, Goal, Inputs, Label, Method, NetworkSpec, OUT_OF_MEMORY, Outcome,
    PREFIX_LABEL, Phase, Swept, VolumeSpec,
};
use cicada_test::{TestActors, UPDATE_CONTRACTS_ENV, describe_changes};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

const SECOND: Duration = Duration::from_secs(1);
/// The secret the job puts in every spec; it must never reach the store.
const SECRET: &str = "s3cret-token";

/// The request of a test container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Run {
    /// The image to run.
    image: String,
}

/// The test job: builds a spec with a secret and a label from the request, and gives its inputs and timeouts.
struct TestJob {
    /// What `inputs` returns.
    inputs: Result<Inputs, String>,
    /// The timeout of a request by its image; the settings' for an image not here.
    timeouts: HashMap<String, Option<Duration>>,
}

impl TestJob {
    /// The job without inputs, with the settings' timeout.
    fn plain() -> Self {
        Self { inputs: Ok(Inputs::default()), timeouts: HashMap::new() }
    }
}

impl ContainerJob for TestJob {
    const TYPE: &'static str = "test.container";
    type Request = Run;

    fn spec(&self, name: &str, run: &Run) -> Result<ContainerSpec, String> {
        if run.image.is_empty() {
            return Err("no image".to_owned());
        }
        let mut spec = ContainerSpec::new(name, run.image.clone());
        spec.secret_env.insert("TOKEN".into(), SECRET.into());
        spec.labels.insert("made".into(), "actor".into());
        if run.image == "thief" {
            spec.labels.insert(PREFIX_LABEL.into(), "other-".into()); // the actor's own label wins
        }
        Ok(spec)
    }

    fn inputs(&self, _: &str, _: &Run) -> Result<Inputs, String> {
        self.inputs.clone()
    }

    fn timeout(&self, run: &Run, settings: Option<Duration>) -> Option<Duration> {
        self.timeouts.get(&run.image).copied().unwrap_or(settings)
    }
}

/// The target of the ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Watcher;

impl ActorRef for Watcher {
    const TYPE: &'static str = "test.watcher";
    type Message = WatcherMsg;
}

/// The messages of the target.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum WatcherMsg {
    /// A container's end.
    Ended(ContainerEnded),
}

/// The ends the target got, in order.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Ends {
    /// The ends.
    ends: Vec<ContainerEnded>,
}

struct WatcherActor;

impl Actor for WatcherActor {
    type Ref = Watcher;
    type State = Ends;

    async fn handle(&self, turn: &mut Turn<Self>, message: WatcherMsg) -> HandlerResult {
        let WatcherMsg::Ended(ended) = message;
        turn.state_mut().ends.push(ended);
        Ok(())
    }
}

type State = ContainerState<Run>;

fn settings() -> ContainerSettings {
    ContainerSettings::new("app-").unwrap()
}

/// A node that hosts the test job on `fake` with `settings`, and the target, which exists.
async fn harness(fake: &FakeRuntime, settings: ContainerSettings) -> TestActors {
    harness_on(Arc::new(fake.clone()), settings, TestJob::plain()).await
}

/// A node that hosts `job` on `runtime` with `settings`, and the target, which exists.
async fn harness_on(runtime: Arc<dyn ContainerRuntime>, settings: ContainerSettings, job: TestJob) -> TestActors {
    let test = TestActors::builder()
        .with(move |node| node.host(ContainerActor::new(job, runtime, settings)).host(WatcherActor))
        .build()
        .await
        .unwrap();
    test.create(&Watcher, &Ends::default()).await;
    test
}

/// The container keyed `key` of `image`, created.
async fn container(test: &TestActors, key: &str, image: &str) -> Container<TestJob> {
    let container = Container::<TestJob>::named(key);
    test.create(&container, &ContainerState::new(Run { image: image.into() })).await;
    container
}

fn start() -> ContainerMsg {
    ContainerMsg::start(&Watcher, "Ended").unwrap()
}

async fn ends(test: &TestActors) -> Vec<ContainerEnded> {
    test.state::<_, Ends>(&Watcher).await.ends
}

/// The calls of the fake, by method.
fn methods(fake: &FakeRuntime) -> Vec<&'static str> {
    fake.calls()
        .iter()
        .map(|call| match call {
            Call::Create(_) => "create",
            Call::MakeDirs { .. } => "make_dirs",
            Call::CopyIn { .. } => "copy_in",
            Call::Start(_) => "start",
            Call::Inspect(_) => "inspect",
            Call::Logs { .. } => "logs",
            Call::Stop { .. } => "stop",
            Call::Remove(_) => "remove",
            _ => "other",
        })
        .collect()
}

/// Scenario: A job with its own labels.
#[tokio::test(start_paused = true)]
async fn the_actor_labels_its_containers_with_its_prefix_after_the_job_spec() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    for (key, image) in [("j1", "alpine:3"), ("j2", "thief")] {
        let job = container(&test, key, image).await;
        test.send(&job, start()).await;
    }
    test.run_until_idle().await;

    let specs: Vec<Box<ContainerSpec>> = fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Create(spec) => Some(spec),
            _ => None,
        })
        .collect();
    assert_eq!(specs.len(), 2);
    for spec in &specs {
        assert_eq!(spec.labels.get("made").map(String::as_str), Some("actor"), "{}", spec.name);
        assert_eq!(spec.labels.get(PREFIX_LABEL).map(String::as_str), Some("app-"), "{}", spec.name);
    }
    let listed = fake.list(&Label::new(PREFIX_LABEL, "app-")).await.unwrap();
    assert_eq!(listed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["app-j1", "app-j2"]);
}

/// Scenario: The spec is built, not stored. Scenario: An exit is reported with its logs.
#[tokio::test(start_paused = true)]
async fn the_spec_is_built_at_the_create_and_an_exit_is_reported_with_its_logs() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;

    assert_eq!(methods(&fake), ["inspect", "create", "start"]);
    let Call::Create(spec) = &fake.calls()[1] else { panic!("{:?}", fake.calls()) };
    assert_eq!((spec.name.as_str(), spec.image.as_str()), ("app-j1", "alpine:3"));
    assert_eq!(spec.secret_env.get("TOKEN").map(String::as_str), Some(SECRET));
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Running);
    let stored = test.state_json(&job).await.unwrap().to_string();
    assert!(!stored.contains(SECRET) && !stored.contains("secretEnv") && !stored.contains("app-j1"), "{stored}");
    let state: State = test.state(&job).await;
    assert_eq!((state.goal, state.phase, state.errors), (Some(Goal::Run), Phase::Started, 0));

    fake.set_logs("app-j1", "done");
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 3 });
    test.advance_time_by(2 * SECOND).await;
    let state: State = test.state(&job).await;
    let end = state.end.clone().unwrap();
    assert_eq!((end.outcome, end.code, end.kind, end.logs.as_str()), (Outcome::Exited, Some(3), None, "done\n"));
    assert_eq!(state.phase, Phase::Ended);
    assert_eq!(ends(&test).await, [ContainerEnded { key: "j1".into(), end }]);
    assert_eq!(state.armed, None, "no work left");
    assert!(test.pending(&job).await.is_empty());
    assert!(test.failures().await.is_empty());
}

/// Scenario: A key that cannot name a container.
#[tokio::test(start_paused = true)]
async fn a_key_that_cannot_name_a_container_ends_failed_without_a_runtime_call() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "a/b", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let end = test.state::<_, State>(&job).await.end.unwrap();
    assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::NotStarted)));
    assert!(end.reason.contains("\"a/b\""), "{}", end.reason);
    assert!(fake.calls().is_empty());
    assert_eq!(ends(&test).await[0].key, "a/b", "the key as given");

    // Nothing can have this name, so a removal removes nothing.
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Removed);
    assert!(fake.calls().is_empty());
}

/// Scenario: A repeated step starts one container: the runtime holds the running container a step made before its
/// commit was lost.
#[tokio::test(start_paused = true)]
async fn a_repeated_step_finds_the_container_its_lost_commit_made() {
    let fake = FakeRuntime::new();
    let mut before = ContainerSpec::new("app-j1", "alpine:3");
    before.labels.insert("made".into(), "before".into());
    fake.create(&before).await.unwrap();
    fake.start("app-j1").await.unwrap();
    fake.set_logs("app-j1", "first run");
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;

    let kept = fake.list(&Label::new("made", "before")).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!((kept[0].name.as_str(), &kept[0].status), ("app-j1", &ContainerStatus::Running));
    assert!(fake.list(&Label::new("made", "actor")).await.unwrap().is_empty(), "no second container");
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Started);

    fake.exit_with("app-j1", ContainerStatus::Exited { code: 0 });
    test.advance_time_by(2 * SECOND).await;
    let end = test.state::<_, State>(&job).await.end.unwrap();
    assert_eq!((end.outcome, end.code, end.logs.as_str()), (Outcome::Exited, Some(0), "first run\n"));
}

/// Scenario: Stop before start.
#[tokio::test(start_paused = true)]
async fn a_stop_before_the_start_makes_nothing_and_the_start_gets_the_end() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, ContainerMsg::Stop).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    let end = state.end.clone().unwrap();
    assert_eq!((end.outcome, end.code, end.kind, end.logs.as_str()), (Outcome::Stopped, None, None, ""));
    assert_eq!((state.goal, state.errors), (Some(Goal::Stop), 0));
    assert_eq!(methods(&fake), ["stop", "inspect"]);
    assert!(ends(&test).await.is_empty(), "no target yet");

    test.send(&job, start()).await;
    test.run_until_idle().await;
    assert_eq!(methods(&fake), ["stop", "inspect"], "nothing is created");
    assert_eq!(ends(&test).await, [ContainerEnded { key: "j1".into(), end }]);
    let state: State = test.state(&job).await;
    assert_eq!((state.goal, state.target.unwrap().message_type.as_str()), (Some(Goal::Stop), "Ended"));
    assert!(test.pending(&job).await.is_empty());
}

/// Scenario: Remove with no start.
#[tokio::test(start_paused = true)]
async fn a_remove_with_no_start_ends_stopped_and_removes_nothing() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    assert_eq!(state.end.unwrap().outcome, Outcome::Stopped);
    assert_eq!((state.phase, state.errors, state.armed), (Phase::Removed, 0, None));
    assert_eq!(methods(&fake), ["stop", "inspect", "remove"]);
    assert!(test.failures().await.is_empty());
}

/// Scenario: A missing image, and a spec the runtime refuses: `notStarted`.
#[tokio::test(start_paused = true)]
async fn a_missing_image_ends_failed_at_the_first_poll() {
    for (kind, reason) in [(ErrorKind::ImageNotFound, "image not found"), (ErrorKind::InvalidSpec, "invalid spec")] {
        let fake = FakeRuntime::new();
        fake.fail_next(Method::Create, kind);
        let test = harness(&fake, settings()).await;
        let job = container(&test, "j1", "nope:1").await;
        test.send(&job, start()).await;
        test.run_until_idle().await;
        let state: State = test.state(&job).await;
        let end = state.end.unwrap();
        assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::NotStarted)));
        assert!(end.reason.starts_with(reason), "{}", end.reason);
        assert_eq!((state.errors, state.armed), (0, None), "no backoff");
        test.advance_time_by(120 * SECOND).await;
        assert_eq!(methods(&fake), ["inspect", "create"], "no further create");
    }
}

/// A spec the job refuses ends the container `failed` with its text, and nothing is made.
#[tokio::test(start_paused = true)]
async fn a_spec_the_job_refuses_ends_failed_with_its_text() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let end = test.state::<_, State>(&job).await.end.unwrap();
    assert_eq!(
        (end.outcome, end.kind, end.reason.as_str()),
        (Outcome::Failed, Some(FailureKind::NotStarted), "no image")
    );
    assert_eq!(methods(&fake), ["inspect"], "nothing is made");
}

/// Scenario: Remove after the exit: the end with its logs is recorded before the removal, and sent once.
#[tokio::test(start_paused = true)]
async fn a_remove_after_the_exit_removes_without_a_second_end() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.set_logs("app-j1", "bye");
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 0 });
    test.advance_time_by(2 * SECOND).await;
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;

    assert_eq!(methods(&fake), ["inspect", "create", "start", "inspect", "logs", "remove"]);
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Missing);
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.end.unwrap().logs.as_str()), (Phase::Removed, "bye\n"));
    assert_eq!(ends(&test).await.len(), 1);

    let calls = fake.calls().len();
    test.send(&job, ContainerMsg::Remove).await;
    test.send(&job, ContainerMsg::Check).await;
    test.run_until_idle().await;
    assert_eq!(fake.calls().len(), calls, "no work left");
    assert!(test.pending(&job).await.is_empty());
}

/// Remove while it runs: it is stopped, its end recorded with the stop's code and its logs, then it is removed.
#[tokio::test(start_paused = true)]
async fn a_remove_while_it_runs_stops_records_and_removes() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.set_logs("app-j1", "working");
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    assert_eq!(methods(&fake), ["inspect", "create", "start", "stop", "inspect", "logs", "remove"]);
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.code, end.kind, end.logs.as_str()), (Outcome::Stopped, Some(143), None, "working\n"));
    assert_eq!(state.phase, Phase::Removed);
    assert_eq!(ends(&test).await.len(), 1);
}

/// Scenario: An unknown target type.
#[tokio::test(start_paused = true)]
async fn an_end_refused_by_an_unknown_target_type_is_saved_as_undelivered() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    let to = ActorAddress::parse("nobody.home/x").unwrap();
    test.send(&job, ContainerMsg::Start { to, message_type: "Ended".into() }).await;
    test.run_until_idle().await;
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 1 });
    test.advance_time_by(2 * SECOND).await;
    let state: State = test.state(&job).await;
    assert_eq!(state.end.unwrap().outcome, Outcome::Exited);
    assert!(state.undelivered.is_some_and(|why| !why.is_empty()));

    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Removed);
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Missing);
    assert!(test.failures().await.is_empty(), "no poll fails");
}

/// Scenario: The deadline passes.
#[tokio::test(start_paused = true)]
async fn a_container_past_its_deadline_is_stopped_and_ends_timed_out() {
    let fake = FakeRuntime::new();
    let mut settings = settings();
    settings.timeout = Some(60 * SECOND);
    let test = harness(&fake, settings).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let deadline = test.state::<_, State>(&job).await.deadline.unwrap();
    assert_eq!(deadline, time::add(test.now(), 60 * SECOND).unwrap());

    test.advance_time_by(58 * SECOND).await;
    assert!(test.state::<_, State>(&job).await.end.is_none(), "it still runs before the deadline");
    test.advance_time_by(2 * SECOND).await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.code, end.kind), (Outcome::TimedOut, Some(143), None));
    assert_eq!(state.goal, Some(Goal::Stop));
    assert!(fake.calls().iter().any(|call| matches!(call, Call::Stop { grace, .. } if *grace == 10 * SECOND)));
}

/// Scenario: Too many errors in a row: the polls wait longer after each error, and the third ends it `failed`.
#[tokio::test(start_paused = true)]
async fn errors_in_a_row_back_off_and_end_failed_at_the_maximum() {
    let fake = FakeRuntime::new();
    let mut settings = settings();
    settings.max_errors = 3;
    let test = harness(&fake, settings).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.fail_always(Method::Inspect, ErrorKind::DaemonUnavailable);

    let wait = |state: &State, now: Timestamp| time::between(now, state.due.unwrap());
    test.advance_time_by(2 * SECOND).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.errors, wait(&state, test.now())), (1, 4 * SECOND));
    test.advance_time_by(4 * SECOND).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.errors, wait(&state, test.now())), (2, 8 * SECOND));
    test.advance_time_by(7 * SECOND).await;
    assert!(test.state::<_, State>(&job).await.end.is_none(), "still waiting");
    test.advance_time_by(SECOND).await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::RuntimeErrors)));
    assert!(end.reason.contains("3 times in a row") && end.reason.contains("daemon unavailable"), "{}", end.reason);
    assert_eq!((state.errors, state.armed), (0, None));
    assert!(test.failures().await.is_empty(), "no turn fails");
    assert_eq!(ends(&test).await.len(), 1);
}

/// A removal keeps trying past the maximum errors in a row, at the longest backoff, and an error resets on success.
#[tokio::test(start_paused = true)]
async fn a_removal_keeps_trying_past_the_maximum() {
    let fake = FakeRuntime::new();
    let mut settings = settings();
    settings.max_errors = 2;
    let test = harness(&fake, settings).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 0 });
    test.advance_time_by(2 * SECOND).await;
    fake.fail_always(Method::Remove, ErrorKind::DaemonUnavailable);
    test.send(&job, ContainerMsg::Remove).await;
    test.advance_time_by(300 * SECOND).await;
    let state: State = test.state(&job).await;
    assert!(state.errors > 2, "{}", state.errors);
    assert_eq!((state.phase, time::between(test.now(), state.due.unwrap()) <= 60 * SECOND), (Phase::Ended, true));
    fake.clear_failures();
    test.advance_time_by(60 * SECOND).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors), (Phase::Removed, 0));
}

/// A `Start` whose message type the state decoder would refuse, sent from code past the schema, is refused for good
/// with nothing saved, and the actor still takes messages.
#[tokio::test(start_paused = true)]
async fn a_start_with_a_message_type_out_of_bounds_is_refused_and_saves_nothing() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    let before = test.state_json(&job).await.unwrap();
    let to = Watcher.address().unwrap();
    // A NUL never gets this far: the core refuses it at the send.
    for message_type in [String::new(), "E".repeat(201)] {
        let sent = test.send(&job, ContainerMsg::Start { to: to.clone(), message_type }).await;
        test.run_until_idle().await;
        let outcome = test.outcome(&sent).await.unwrap();
        assert_eq!(outcome.envelope.attempt, 1, "refused for good");
        assert!(outcome.error.unwrap_or_default().contains("messageType must be"));
        assert_eq!(test.state_json(&job).await.unwrap(), before, "nothing saved");
    }
    assert!(fake.calls().is_empty());

    test.send(&job, start()).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.target.unwrap().message_type.as_str()), (Phase::Started, "Ended"));
}

/// Errors reset only when a whole step completes: logs that never can be read count on every poll, and at the limit
/// the end is saved with `(no logs: …)`.
#[tokio::test(start_paused = true)]
async fn logs_that_always_fail_end_at_the_limit_without_logs() {
    let fake = FakeRuntime::new();
    let mut settings = settings();
    settings.max_errors = 3;
    let test = harness(&fake, settings).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.fail_always(Method::Logs, ErrorKind::DaemonUnavailable);
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 2 });
    test.advance_time_by(6 * SECOND).await;
    assert_eq!(test.state::<_, State>(&job).await.errors, 2, "the inspects do not reset the count");
    test.advance_time_by(8 * SECOND).await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.code), (Outcome::Exited, Some(2)));
    assert!(end.logs.starts_with("(no logs: daemon unavailable"), "{}", end.logs);
    assert_eq!((state.errors, state.armed), (0, None));
    assert_eq!(ends(&test).await.len(), 1);
}

/// Scenario: An ended container without logs. A log the runtime can never read (`Unsupported`) ends the end at once
/// with `(no logs: …)`: not counted as an error, the read not retried.
#[tokio::test(start_paused = true)]
async fn logs_the_runtime_cannot_read_end_at_once_without_logs() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.fail_always(Method::Logs, ErrorKind::Unsupported);
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 0 });
    let before = fake.calls().len();
    test.advance_time_by(2 * SECOND).await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.code), (Outcome::Exited, Some(0)));
    assert!(end.logs.starts_with("(no logs: unsupported"), "{}", end.logs);
    assert_eq!((state.errors, state.armed), (0, None));
    assert_eq!(methods(&fake)[before..], ["inspect", "logs"], "one read, no retry");
    assert_eq!(ends(&test).await.len(), 1);
}

/// `Other` from a log read, which a passing daemon error gives too, still counts and is retried.
#[tokio::test(start_paused = true)]
async fn other_from_a_log_read_is_still_counted_and_retried() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.fail_next(Method::Logs, ErrorKind::Other);
    fake.set_logs("app-j1", "bye\n");
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 0 });
    test.advance_time_by(2 * SECOND).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.errors, state.end.is_none()), (1, true), "counted, no end yet");
    test.advance_time_by(10 * SECOND).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.errors, state.end.unwrap().logs.as_str()), (0, "bye\n"), "the retry read the logs");
}

/// With the goal `Remove`, an unreadable log ends the container at once and the removal goes on as before.
#[tokio::test(start_paused = true)]
async fn an_unreadable_log_does_not_hold_up_a_removal() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.fail_always(Method::Logs, ErrorKind::Unsupported);
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 3 });
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    assert_eq!(state.phase, Phase::Removed);
    let end = state.end.unwrap();
    assert_eq!(end.code, Some(3));
    assert!(end.logs.starts_with("(no logs: unsupported"), "{}", end.logs);
    assert_eq!(ends(&test).await.len(), 1);
}

/// A started container the runtime reports as created, whose start always fails, ends `failed` at the limit.
#[tokio::test(start_paused = true)]
async fn a_created_container_whose_start_always_fails_ends_failed_at_the_limit() {
    let fake = FakeRuntime::new();
    fake.create(&ContainerSpec::new("app-j1", "alpine:3")).await.unwrap();
    fake.fail_always(Method::Start, ErrorKind::Other);
    let mut settings = settings();
    settings.max_errors = 3;
    let test = harness(&fake, settings).await;
    let job = Container::<TestJob>::named("j1");
    let mut state = ContainerState::new(Run { image: "alpine:3".into() });
    state.goal = Some(Goal::Run);
    state.phase = Phase::Started;
    test.create(&job, &state).await;
    test.send(&job, ContainerMsg::Check).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, State>(&job).await.errors, 1);
    test.advance_time_by(4 * SECOND).await;
    assert_eq!(test.state::<_, State>(&job).await.errors, 2, "the inspects do not reset the count");
    test.advance_time_by(8 * SECOND).await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::RuntimeErrors)));
    assert!(end.reason.contains("3 times in a row"), "{}", end.reason);
    assert_eq!(methods(&fake).iter().filter(|m| **m == "start").count(), 3);
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Created);
}

/// Scenario: A purged poll comes back on `Check`, and the container is followed to its end.
#[tokio::test(start_paused = true)]
async fn a_purged_poll_comes_back_on_check() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let address = job.address().unwrap();
    assert_eq!(test.system().purge_mailbox(&address).await.unwrap(), Some(1));
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 0 });
    test.advance_time_by(60 * SECOND).await;
    assert!(test.state::<_, State>(&job).await.end.is_none(), "the poll is lost");

    test.send(&job, ContainerMsg::Check).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    assert_eq!(state.end.unwrap().outcome, Outcome::Exited);
    assert_eq!(ends(&test).await.len(), 1);
}

/// A goal asked again replaces the armed poll and keeps its due time; a stale poll is ignored.
#[tokio::test(start_paused = true)]
async fn a_goal_asked_again_keeps_the_backoff() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let before: State = test.state(&job).await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let after: State = test.state(&job).await;
    assert_ne!(after.armed, before.armed, "a fresh poll");
    assert_eq!(after.due, before.due, "due when the old one was");
    assert_eq!(test.pending(&job).await.len(), 1, "the old one is cancelled");
    test.send(&job, ContainerMsg::Poll).await;
    test.run_until_idle().await;
    assert_eq!(methods(&fake), ["inspect", "create", "start"], "a poll that is not armed does nothing");
}

/// Scenario: A NUL in the logs.
#[tokio::test(start_paused = true)]
async fn a_nul_in_the_logs_is_stored_as_the_replacement_character_within_the_caps() {
    let fake = FakeRuntime::new();
    let mut settings = settings();
    settings.log_bytes = 16;
    let test = harness(&fake, settings).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    fake.set_logs("app-j1", format!("{}\nend\0of it", "x".repeat(100)));
    fake.exit_with("app-j1", ContainerStatus::Exited { code: 0 });
    test.advance_time_by(2 * SECOND).await;
    let logs = test.state::<_, State>(&job).await.end.unwrap().logs;
    assert_eq!(logs, "xxx\nend\u{FFFD}of it\n", "the last line whole, then the last 16 bytes");
    assert!(logs.len() <= 16);
}

/// Scenario: A lost commit after start: the running container is adopted with a fresh deadline, and neither `spec`
/// nor `inputs`, which now fail, is called.
#[tokio::test(start_paused = true)]
async fn a_lost_commit_after_the_start_adopts_the_running_container() {
    let fake = FakeRuntime::new();
    fake.create(&ContainerSpec::new("app-j1", "alpine:3")).await.unwrap();
    fake.start("app-j1").await.unwrap();
    let mut settings = settings();
    settings.timeout = Some(60 * SECOND);
    let test = harness_on(Arc::new(fake.clone()), settings, with(Err("no inputs today".to_owned()))).await;
    // An empty image makes the job's `spec` fail.
    let job = container(&test, "j1", "").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.end, state.errors), (Phase::Started, None, 0), "adopted, no notStarted end");
    assert_eq!(state.deadline, Some(time::add(test.now(), 60 * SECOND).unwrap()), "a fresh deadline");
    assert_eq!(methods(&fake)[2..], ["inspect"], "no create, copies or start");

    fake.exit_with("app-j1", ContainerStatus::Exited { code: 0 });
    test.advance_time_by(2 * SECOND).await;
    let end = test.state::<_, State>(&job).await.end.unwrap();
    assert_eq!((end.outcome, end.kind), (Outcome::Exited, None));
}

/// Scenario: Kinds of failure: a start refused with `InvalidSpec` ends `notStarted` in the poll that tried it, without
/// reading logs or backing off.
#[tokio::test(start_paused = true)]
async fn a_refused_start_ends_not_started_at_once() {
    let fake = FakeRuntime::new();
    fake.fail_next(Method::Start, ErrorKind::InvalidSpec);
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.kind, end.logs.as_str()), (Outcome::Failed, Some(FailureKind::NotStarted), ""));
    assert!(end.reason.starts_with("invalid spec: scripted failure of Start"), "{}", end.reason);
    assert_eq!((state.errors, state.armed), (0, None), "no backoff");
    test.advance_time_by(120 * SECOND).await;
    assert_eq!(methods(&fake), ["inspect", "create", "start"], "no logs, no retry");
    assert_eq!(ends(&test).await.len(), 1);
}

/// A started container that the runtime reports `Created`, whose start it refuses with `InvalidSpec`, ends
/// `notStarted` too.
#[tokio::test(start_paused = true)]
async fn a_refused_start_of_a_created_container_being_followed_ends_not_started() {
    let fake = FakeRuntime::new();
    fake.create(&ContainerSpec::new("app-j1", "alpine:3")).await.unwrap();
    fake.fail_next(Method::Start, ErrorKind::InvalidSpec);
    let test = harness(&fake, settings()).await;
    let job = Container::<TestJob>::named("j1");
    let mut state = ContainerState::new(Run { image: "alpine:3".into() });
    state.goal = Some(Goal::Run);
    state.phase = Phase::Started;
    test.create(&job, &state).await;
    test.send(&job, ContainerMsg::Check).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::NotStarted)));
    assert_eq!((state.errors, state.armed), (0, None));
    assert_eq!(methods(&fake)[1..], ["inspect", "start"]);
}

/// Scenario: Kinds of failure, of a started container: `Failed` with `OUT_OF_MEMORY` is `outOfMemory`, with any other
/// reason `runtimeEnded`, and `Missing` is `missing`; each keeps the runtime's code and reason.
#[tokio::test(start_paused = true)]
async fn the_runtimes_failures_of_a_started_container_have_their_kinds() {
    let failed = |reason: &str| Some(ContainerStatus::Failed { code: Some(137), reason: reason.into() });
    let cases = [
        (failed(OUT_OF_MEMORY), FailureKind::OutOfMemory, Some(137), "out of memory"),
        (failed("dead"), FailureKind::RuntimeEnded, Some(137), "dead"),
        (failed("Out of memory"), FailureKind::RuntimeEnded, Some(137), "Out of memory"),
        (None, FailureKind::Missing, None, "the container went missing"),
    ];
    for (status, kind, code, reason) in cases {
        let fake = FakeRuntime::new();
        let test = harness(&fake, settings()).await;
        let job = container(&test, "j1", "alpine:3").await;
        test.send(&job, start()).await;
        test.run_until_idle().await;
        match status {
            Some(status) => fake.exit_with("app-j1", status),
            None => fake.remove("app-j1").await.unwrap(),
        }
        test.advance_time_by(2 * SECOND).await;
        let end = test.state::<_, State>(&job).await.end.unwrap();
        assert_eq!((end.outcome, end.kind, end.code, end.reason.as_str()), (Outcome::Failed, Some(kind), code, reason));
        assert_eq!(ends(&test).await[0].end, end);
    }
}

/// Scenario: A timeout per request: with 60 s in the settings, a request the job gives 10 s ends `timedOut` after
/// 10 s, one it gives 90 s after 90 s, one it gives none still runs after 120 s, and one it does not name gets 60 s.
#[tokio::test(start_paused = true)]
async fn each_request_gets_the_timeout_its_job_gives() {
    let fake = FakeRuntime::new();
    let mut settings = settings();
    settings.timeout = Some(60 * SECOND);
    let mut job = TestJob::plain();
    job.timeouts.insert("short:1".into(), Some(10 * SECOND));
    job.timeouts.insert("long:1".into(), Some(90 * SECOND));
    job.timeouts.insert("endless:1".into(), None);
    let test = harness_on(Arc::new(fake.clone()), settings, job).await;
    let mut containers = Vec::new();
    for (key, image) in [("short", "short:1"), ("long", "long:1"), ("endless", "endless:1"), ("plain", "alpine:3")] {
        let container = container(&test, key, image).await;
        test.send(&container, start()).await;
        containers.push(container);
    }
    test.run_until_idle().await;
    let started = test.now();
    let deadlines = async || {
        let mut deadlines = Vec::new();
        for container in &containers {
            let state: State = test.state(container).await;
            deadlines.push(state.deadline.map(|deadline| time::between(started, deadline)));
        }
        deadlines
    };
    assert_eq!(deadlines().await, [Some(10 * SECOND), Some(90 * SECOND), None, Some(60 * SECOND)]);

    let outcome = async |container: &Container<TestJob>| test.state::<_, State>(container).await.end.map(|e| e.outcome);
    test.advance_time_by(10 * SECOND).await;
    assert_eq!(outcome(&containers[0]).await, Some(Outcome::TimedOut));
    assert_eq!(outcome(&containers[1]).await, None);
    test.advance_time_by(50 * SECOND).await;
    assert_eq!(outcome(&containers[3]).await, Some(Outcome::TimedOut), "the settings' 60 s");
    assert_eq!(outcome(&containers[1]).await, None);
    test.advance_time_by(30 * SECOND).await;
    assert_eq!(outcome(&containers[1]).await, Some(Outcome::TimedOut));
    test.advance_time_by(30 * SECOND).await;
    assert_eq!(outcome(&containers[2]).await, None, "no limit");
    assert_eq!(fake.inspect("app-endless").await.unwrap(), ContainerStatus::Running);
}

/// A timeout past the largest instant gives the deadline `MAX_INSTANT`, not no limit.
#[tokio::test(start_paused = true)]
async fn a_deadline_past_the_largest_instant_is_capped() {
    let fake = FakeRuntime::new();
    let mut job = TestJob::plain();
    job.timeouts.insert("alpine:3".into(), Some(Duration::MAX));
    job.timeouts.insert("busybox:1".into(), Some(Duration::from_secs(9000 * 365 * 24 * 3600)));
    let test = harness_on(Arc::new(fake.clone()), settings(), job).await;
    for (key, image) in [("j1", "alpine:3"), ("j2", "busybox:1")] {
        let container = container(&test, key, image).await;
        test.send(&container, start()).await;
        test.run_until_idle().await;
        let state: State = test.state(&container).await;
        assert_eq!((state.phase, state.deadline), (Phase::Started, Some(MAX_INSTANT)), "{image}");
    }
}

/// Writes the runtime of `$ty`, a wrapper of the fake in its field `fake`, with every method but `create`, `inspect`
/// and `make_dirs` passed to the fake, and the methods given in braces.
macro_rules! wrapper {
    ($ty:ty { $($methods:tt)* }) => {
        #[async_trait]
        impl ContainerRuntime for $ty {
            async fn copy_in(&self, name: &str, files: &[CopyFile]) -> Result<(), ContainerError> {
                self.fake.copy_in(name, files).await
            }
            async fn start(&self, name: &str) -> Result<(), ContainerError> {
                self.fake.start(name).await
            }
            async fn logs(&self, name: &str, tail_lines: u32) -> Result<String, ContainerError> {
                self.fake.logs(name, tail_lines).await
            }
            async fn stop(&self, name: &str, grace: Duration) -> Result<(), ContainerError> {
                self.fake.stop(name, grace).await
            }
            async fn remove(&self, name: &str) -> Result<(), ContainerError> {
                self.fake.remove(name).await
            }
            async fn list(&self, label: &Label) -> Result<Vec<ContainerSummary>, ContainerError> {
                self.fake.list(label).await
            }
            async fn ensure_network(&self, spec: &NetworkSpec) -> Result<(), ContainerError> {
                self.fake.ensure_network(spec).await
            }
            async fn ensure_volume(&self, spec: &VolumeSpec) -> Result<(), ContainerError> {
                self.fake.ensure_volume(spec).await
            }
            async fn sweep(&self, label: &Label) -> Result<Swept, ContainerError> {
                self.fake.sweep(label).await
            }
            $($methods)*
        }
    };
}

/// A host's runtime written before `make_dirs`: the fake, with the trait's default `make_dirs`, which fails with
/// `Unsupported`.
struct Older {
    fake: FakeRuntime,
}

wrapper!(Older {
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError> {
        self.fake.create(spec).await
    }
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError> {
        self.fake.inspect(name).await
    }
});

/// The fake, where someone else removes the container once, just before the `at`-th inspect (from 1).
struct Removing {
    fake: FakeRuntime,
    at: usize,
    inspects: AtomicUsize,
}

impl Removing {
    /// The runtime of `fake`, which removes the container before the `at`-th inspect.
    fn runtime(fake: &FakeRuntime, at: usize) -> Arc<dyn ContainerRuntime> {
        Arc::new(Self { fake: fake.clone(), at, inspects: Default::default() })
    }
}

wrapper!(Removing {
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError> {
        self.fake.create(spec).await
    }
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError> {
        if self.inspects.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
            self.fake.remove(name).await?;
        }
        self.fake.inspect(name).await
    }
    async fn make_dirs(&self, name: &str, dirs: &[CopyDir]) -> Result<(), ContainerError> {
        self.fake.make_dirs(name, dirs).await
    }
});

/// The fake, where someone else removes the container right after its first create.
struct Vanishing {
    fake: FakeRuntime,
    removed: AtomicBool,
}

wrapper!(Vanishing {
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError> {
        self.fake.create(spec).await?;
        if !self.removed.swap(true, Ordering::SeqCst) {
            self.fake.remove(&spec.name).await?;
        }
        Ok(())
    }
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError> {
        self.fake.inspect(name).await
    }
    async fn make_dirs(&self, name: &str, dirs: &[CopyDir]) -> Result<(), ContainerError> {
        self.fake.make_dirs(name, dirs).await
    }
});

/// The folder `/out` at `0o777` and the file `/in/config.json`, which holds the secret.
fn out_and_config() -> Inputs {
    let mut inputs = Inputs::default();
    let mut out = CopyDir::new("/out");
    out.mode = 0o777;
    inputs.dirs.push(out);
    inputs.files.push(CopyFile::new("/in/config.json", SECRET));
    inputs
}

/// The test job with `inputs`.
fn with(inputs: Result<Inputs, String>) -> TestJob {
    TestJob { inputs, ..TestJob::plain() }
}

/// A started container of the job `j1` with `inputs` on `runtime`, after its first poll.
async fn started_with(runtime: Arc<dyn ContainerRuntime>, inputs: Inputs) -> (TestActors, Container<TestJob>) {
    let test = harness_on(runtime, settings(), with(Ok(inputs))).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    (test, job)
}

/// Scenario: Inputs before start: the folders, then the files, then the start; the state holds none of them.
#[tokio::test(start_paused = true)]
async fn inputs_are_made_and_written_between_create_and_start_and_never_stored() {
    let fake = FakeRuntime::new();
    let (test, job) = started_with(Arc::new(fake.clone()), out_and_config()).await;
    assert_eq!(methods(&fake), ["inspect", "create", "make_dirs", "copy_in", "start"]);
    let calls = fake.calls();
    assert_eq!(calls[2], Call::MakeDirs { name: "app-j1".into(), dirs: out_and_config().dirs });
    assert_eq!(calls[3], Call::CopyIn { name: "app-j1".into(), files: out_and_config().files });
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Running);
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors), (Phase::Started, 0));
    let stored = test.state_json(&job).await.unwrap().to_string();
    assert!(!stored.contains(SECRET) && !stored.contains("/out") && !stored.contains("config.json"), "{stored}");
}

/// Scenario: No inputs: only `inspect`, `create` and `start`.
#[tokio::test(start_paused = true)]
async fn a_job_without_inputs_only_creates_and_starts() {
    let fake = FakeRuntime::new();
    let (test, job) = started_with(Arc::new(fake.clone()), Inputs::default()).await;
    assert_eq!(methods(&fake), ["inspect", "create", "start"]);
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Started);
}

/// Scenario: A replayed step after start: the running container keeps what it has, and runs once.
#[tokio::test(start_paused = true)]
async fn a_replayed_step_after_the_start_puts_no_inputs_in() {
    let fake = FakeRuntime::new();
    fake.create(&ContainerSpec::new("app-j1", "alpine:3")).await.unwrap();
    fake.start("app-j1").await.unwrap();
    let (test, job) = started_with(Arc::new(fake.clone()), out_and_config()).await;
    assert_eq!(methods(&fake)[2..], ["inspect"], "adopted: no create, make_dirs, copy_in or start");
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Running);
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Started);
}

/// Scenario: Inputs that cannot be built: an `Err`, a file at a folder, at a folder's parent, at `/out/.`, and a path
/// the shared rule refuses each end the container `failed` with no runtime call.
#[tokio::test(start_paused = true)]
async fn inputs_that_cannot_be_built_end_failed_before_the_create() {
    let both = |dir: &str, file: &str| {
        let mut inputs = Inputs::default();
        inputs.dirs.push(CopyDir::new(dir));
        inputs.files.push(CopyFile::new(file, "x"));
        Ok(inputs)
    };
    let cases = [
        (Err("no config today".to_owned()), "no config today"),
        (both("/out", "/out"), "the file /out is at or above the folder /out:"),
        (both("/a/b", "/a"), "the file /a is at or above the folder /a/b:"),
        (both("/out/.", "/out"), "the file /out is at or above the folder /out/.:"),
        (both("/out", "in/x"), "invalid spec: copy into app-j1: relative path in/x"),
    ];
    for (inputs, reason) in cases {
        let fake = FakeRuntime::new();
        let test = harness_on(Arc::new(fake.clone()), settings(), with(inputs)).await;
        let job = container(&test, "j1", "alpine:3").await;
        test.send(&job, start()).await;
        test.run_until_idle().await;
        let state: State = test.state(&job).await;
        let end = state.end.unwrap();
        assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::NotStarted)));
        assert!(end.reason.starts_with(reason), "{reason}: {}", end.reason);
        assert_eq!((state.errors, state.armed), (0, None));
        assert_eq!(methods(&fake), ["inspect"], "{reason}: no create");
        assert_eq!(ends(&test).await.len(), 1);
    }
}

/// Scenario: A refused input: `InvalidSpec` from `copy_in` ends the container `failed`, and it is not started.
#[tokio::test(start_paused = true)]
async fn an_input_the_runtime_refuses_ends_failed_without_a_start() {
    let fake = FakeRuntime::new();
    fake.fail_next(Method::CopyIn, ErrorKind::InvalidSpec);
    let (test, job) = started_with(Arc::new(fake.clone()), out_and_config()).await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::NotStarted)));
    assert!(end.reason.starts_with("invalid spec: scripted failure of CopyIn"), "{}", end.reason);
    assert_eq!((state.errors, state.armed), (0, None), "no backoff");
    test.advance_time_by(120 * SECOND).await;
    assert_eq!(methods(&fake), ["inspect", "create", "make_dirs", "copy_in"], "no start, no retry");
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Created, "kept until Remove");
}

/// Scenario: A conflict while copying, while the container is still `Created`: the job's mistake.
#[tokio::test(start_paused = true)]
async fn a_conflict_while_the_container_is_still_created_ends_failed() {
    let fake = FakeRuntime::new();
    fake.fail_next(Method::MakeDirs, ErrorKind::Conflict);
    let (test, job) = started_with(Arc::new(fake.clone()), out_and_config()).await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::NotStarted)));
    assert!(end.reason.starts_with("conflict: scripted failure of MakeDirs"), "{}", end.reason);
    assert_eq!((state.errors, state.armed), (0, None));
    assert_eq!(methods(&fake), ["inspect", "create", "make_dirs", "inspect"], "inspected again");
}

/// Scenario: A conflict while copying, with the container gone at the new inspect: an error that backs off, and the
/// next poll creates the container again, puts the inputs in and starts it.
#[tokio::test(start_paused = true)]
async fn a_conflict_with_the_container_gone_backs_off_and_the_next_poll_starts_it() {
    let fake = FakeRuntime::new();
    fake.fail_next(Method::MakeDirs, ErrorKind::Conflict);
    let (test, job) = started_with(Removing::runtime(&fake, 2), out_and_config()).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors, state.end), (Phase::New, 1, None));
    assert_eq!(methods(&fake), ["inspect", "create", "make_dirs", "remove", "inspect"]);
    test.advance_time_by(4 * SECOND).await;
    assert_eq!(methods(&fake)[5..], ["inspect", "create", "make_dirs", "copy_in", "start"]);
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors), (Phase::Started, 0));
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Running);
}

/// Scenario: A runtime without make_dirs: the trait's default `Unsupported` ends the container `failed` at once.
#[tokio::test(start_paused = true)]
async fn a_runtime_without_make_dirs_ends_failed_at_once() {
    let fake = FakeRuntime::new();
    let (test, job) = started_with(Arc::new(Older { fake: fake.clone() }), out_and_config()).await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.kind), (Outcome::Failed, Some(FailureKind::NotStarted)));
    assert!(end.reason.starts_with("unsupported: "), "{}", end.reason);
    assert!(end.reason.contains("make_dirs is not supported by this runtime"), "{}", end.reason);
    assert_eq!((state.errors, state.armed), (0, None), "no backoff");
    test.advance_time_by(120 * SECOND).await;
    assert_eq!(methods(&fake), ["inspect", "create"], "nothing more reaches the fake");
}

/// Scenario: Another error from make_dirs: `Other`, such as a passing error of the daemon, counts and backs off like
/// any runtime error, and the next poll puts the inputs in.
#[tokio::test(start_paused = true)]
async fn another_error_from_make_dirs_backs_off_and_the_next_poll_starts_it() {
    let fake = FakeRuntime::new();
    fake.fail_next(Method::MakeDirs, ErrorKind::Other);
    let (test, job) = started_with(Arc::new(fake.clone()), out_and_config()).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors, state.end), (Phase::New, 1, None));
    assert_eq!(methods(&fake), ["inspect", "create", "make_dirs"]);
    test.advance_time_by(4 * SECOND).await;
    assert_eq!(methods(&fake)[3..], ["inspect", "create", "make_dirs", "copy_in", "start"]);
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Started);
}

/// Scenario: A container gone after create: removed before the copies, `make_dirs` finds it missing (`Conflict`), the
/// new inspect does too, so the error backs off and the next poll creates it again.
#[tokio::test(start_paused = true)]
async fn a_container_gone_after_its_create_backs_off_and_the_next_poll_starts_it() {
    let fake = FakeRuntime::new();
    let runtime = Arc::new(Vanishing { fake: fake.clone(), removed: AtomicBool::new(false) });
    let (test, job) = started_with(runtime, out_and_config()).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors, state.end), (Phase::New, 1, None));
    assert_eq!(methods(&fake), ["inspect", "create", "remove", "make_dirs", "inspect"]);
    test.advance_time_by(4 * SECOND).await;
    assert_eq!(methods(&fake)[5..], ["inspect", "create", "make_dirs", "copy_in", "start"]);
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Started);
}

/// Scenario: An unavailable daemon while copying: an error that backs off, and the next poll puts the inputs in.
#[tokio::test(start_paused = true)]
async fn an_unavailable_daemon_while_copying_backs_off_and_the_next_poll_starts_it() {
    let fake = FakeRuntime::new();
    fake.fail_next(Method::MakeDirs, ErrorKind::DaemonUnavailable);
    let (test, job) = started_with(Arc::new(fake.clone()), out_and_config()).await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors, state.end), (Phase::New, 1, None));
    assert_eq!(methods(&fake), ["inspect", "create", "make_dirs"]);
    test.advance_time_by(4 * SECOND).await;
    assert_eq!(methods(&fake)[3..], ["inspect", "create", "make_dirs", "copy_in", "start"]);
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors), (Phase::Started, 0));
    assert!(test.failures().await.is_empty(), "no turn fails");
}

/// `value` with the keys of every object sorted, so that the fixture's text does not follow the order of a map.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&o[k]))).collect::<Map<_, _>>())
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// Compares `contract` with its fixture in `tests/contracts` as text, descriptions included; with `update`, a fixture
/// that differs or is missing is written instead.
fn check_fixture(contract: &Contract, update: bool) -> Result<(), String> {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/contracts/{}.json", contract.type_name()));
    let current = serde_json::to_string_pretty(&sorted(&contract.to_json())).unwrap() + "\n";
    let recorded = std::fs::read_to_string(&file).ok();
    if recorded.as_deref() == Some(current.as_str()) {
        return Ok(());
    }
    if update {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        return std::fs::write(&file, &current).map_err(|e| format!("Cannot write {}: {e}", file.display()));
    }
    let changes = match recorded.as_deref().map(serde_json::from_str::<Value>) {
        None => vec!["the fixture is missing".to_owned()],
        Some(Err(e)) => vec![format!("the fixture is not JSON: {e}")],
        Some(Ok(json)) => match Contract::from_json(&json) {
            Err(e) => vec![format!("the fixture is not a contract: {e}")],
            Ok(old) if old.hash() == contract.hash() => vec!["a description (doc comment) changed".to_owned()],
            Ok(old) => describe_changes(&old, contract),
        },
    };
    Err(format!(
        "The contract of {} differs from {}:\n  - {}\nIf the change is meant, rewrite it with {UPDATE_CONTRACTS_ENV}=true \
         and commit it.",
        contract.type_name(),
        file.display(),
        changes.join("\n  - ")
    ))
}

/// A container job's contract is the host's: pinned with the actor tests, not in `docs/reference/contracts`.
#[test]
fn the_contract_of_a_test_job_matches_its_fixture() {
    let contract = contract_of::<ContainerActor<TestJob>>().unwrap();
    assert_eq!(contract.type_name().as_str(), "test.container");
    let update = std::env::var(UPDATE_CONTRACTS_ENV).is_ok_and(|v| v == "true");
    if let Err(e) = check_fixture(&contract, update) {
        panic!("{e}");
    }
    let json = contract.to_json();
    let messages: Vec<&String> = json["messages"].as_object().unwrap().keys().collect();
    assert_eq!(messages, ["Check", "Poll", "Remove", "Start", "Stop"]);
    let start = &json["messages"]["Start"];
    assert_eq!(start["required"], json!(["to", "messageType"]));
    assert_eq!(
        (
            start["properties"]["messageType"]["minLength"].clone(),
            start["properties"]["messageType"]["maxLength"].clone()
        ),
        (json!(1), json!(200))
    );
    let state = &json["state"];
    assert_eq!(state["required"], json!(["request"]), "every other field has a default");
    assert_eq!(state["properties"]["request"]["$ref"], "#/$defs/Run", "the host's typed request");
    let names = |def: &str| -> Vec<Value> {
        state["$defs"][def]["oneOf"].as_array().unwrap().iter().map(|v| v["const"].clone()).collect()
    };
    assert_eq!(names("Goal"), [json!("run"), json!("stop"), json!("remove")]);
    assert_eq!(names("Phase"), [json!("new"), json!("started"), json!("ended"), json!("removed")]);
    assert_eq!(names("Outcome"), [json!("exited"), json!("failed"), json!("timedOut"), json!("stopped")]);
    let kinds =
        [json!("notStarted"), json!("outOfMemory"), json!("runtimeEnded"), json!("missing"), json!("runtimeErrors")];
    assert_eq!(names("FailureKind"), kinds);
    let end = &state["$defs"]["ContainerEnd"];
    assert_eq!(end["required"], json!(["outcome", "reason"]), "an end saved before kinds has none");
    assert!(end["properties"]["kind"].is_object());
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/contracts/test.container.json");
    assert!(!docs.exists(), "no fixed type name in the catalog");
}

/// The messages pass their samples: what the helpers write and what a client sends.
#[test]
fn the_messages_pass_their_samples() {
    let start = serde_json::to_value(start()).unwrap();
    assert_eq!(start, json!({"type": "Start", "to": "test.watcher/_", "messageType": "Ended"}));
    assert_message_sample::<ContainerMsg>(&start);
    for message in [ContainerMsg::Stop, ContainerMsg::Remove, ContainerMsg::Check, ContainerMsg::Poll] {
        assert_message_sample::<ContainerMsg>(&serde_json::to_value(message).unwrap());
    }
    assert!(cicada::message::decode::<ContainerMsg>("Start", &json!({"to": "test.watcher/_"})).is_err());
    assert!(cicada::message::decode::<ContainerMsg>("Start", &json!({"to": "nope", "messageType": "E"})).is_err());
}

/// The state in each phase, as the actor writes it, and a minimal one pass their samples; an end payload, as the target
/// gets it, passes as a host's message; inconsistent states are refused.
#[tokio::test(start_paused = true)]
async fn the_states_of_each_phase_and_the_end_payload_pass_their_samples() {
    let fake = FakeRuntime::new();
    let mut settings = settings();
    settings.timeout = Some(60 * SECOND);
    let test = harness(&fake, settings).await;
    let job = container(&test, "j1", "alpine:3").await;
    let mut phases = Vec::new();
    let new = test.state_json(&job).await.unwrap();
    assert_eq!(
        new,
        json!({"request": {"image": "alpine:3"}, "goal": null, "phase": "new", "target": null,
        "deadline": null, "end": null, "undelivered": null, "armed": null, "due": null, "errors": 0})
    );
    phases.push(new);
    test.send(&job, start()).await;
    test.run_until_idle().await;
    phases.push(test.state_json(&job).await.unwrap());
    fake.set_logs("app-j1", "done");
    fake.exit_with("app-j1", ContainerStatus::Failed { code: Some(137), reason: OUT_OF_MEMORY.into() });
    test.advance_time_by(2 * SECOND).await;
    phases.push(test.state_json(&job).await.unwrap());
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    phases.push(test.state_json(&job).await.unwrap());
    let written: Vec<&Value> = phases.iter().map(|state| &state["phase"]).collect();
    assert_eq!(written, ["new", "started", "ended", "removed"]);
    for state in &phases {
        assert_state_sample::<State>(state);
    }
    assert_state_sample::<State>(&json!({"request": {"image": "alpine:3"}}));
    let end = &phases[3]["end"];
    assert_eq!(
        end,
        &json!({"outcome": "failed", "code": 137, "kind": "outOfMemory", "reason": "out of memory", "logs": "done\n"})
    );
    // Ends without a kind: one that did not fail, and a failed one saved before kinds.
    for end in [
        json!({"outcome": "exited", "code": 0, "kind": null, "reason": "exited with code 0", "logs": ""}),
        json!({"outcome": "failed", "code": null, "reason": "the container went missing", "logs": ""}),
    ] {
        let state = json!({"request": {"image": "alpine:3"}, "goal": "run", "phase": "ended", "end": end});
        assert_state_sample::<State>(&state);
        assert_eq!(serde_json::from_value::<ContainerEnd>(end).unwrap().kind, None);
    }

    let ended = serde_json::to_value(&ends(&test).await[0]).unwrap();
    assert_eq!(ended, json!({"key": "j1", "end": end}));
    let mut sample = json!({"type": "Ended"});
    sample.as_object_mut().unwrap().extend(ended.as_object().unwrap().clone());
    assert_message_sample::<WatcherMsg>(&sample);
    let stopped = json!({"key": "j1", "end": {"outcome": "stopped", "kind": "missing", "reason": "stopped"}});
    assert!(cicada::message::decode::<WatcherMsg>("Ended", &stopped).is_err(), "a kind where none belongs");

    for invalid in [
        json!({}),
        json!({"request": {"image": 1}}),
        json!({"request": {"image": "a"}, "phase": "paused"}),
        json!({"request": {"image": "a"}, "goal": "pause"}),
        json!({"request": {"image": "a"}, "goal": "stop", "phase": "ended"}),
        json!({"request": {"image": "a"}, "phase": "started"}),
        json!({"request": {"image": "a"}, "armed": "m1"}),
        json!({"request": {"image": "a"}, "goal": "stop", "phase": "ended", "end": {"outcome": "gone", "reason": ""}}),
        json!({"request": {"image": "a"}, "goal": "stop", "phase": "ended",
            "end": {"outcome": "stopped", "kind": "missing", "reason": "stopped"}}),
        json!({"request": {"image": "a"}, "goal": "stop", "phase": "ended",
            "end": {"outcome": "failed", "kind": "lost", "reason": "x"}}),
    ] {
        assert!(serde_json::from_value::<State>(invalid.clone()).is_err(), "{invalid}");
    }
}

/// The job of the turn-limit tests: the test job's spec, with a `MAX_TURN` of `SECS` seconds.
struct Limited<const SECS: u64>;

impl<const SECS: u64> ContainerJob for Limited<SECS> {
    const TYPE: &'static str = "test.limited";
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(SECS));
    type Request = Run;

    fn spec(&self, name: &str, run: &Run) -> Result<ContainerSpec, String> {
        TestJob::plain().spec(name, run)
    }
}

/// The fake, whose first create takes `pull` after the container is made: a slow pull a turn limit can cut off.
struct SlowPull {
    fake: FakeRuntime,
    pull: Duration,
    pulled: AtomicBool,
}

wrapper!(SlowPull {
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError> {
        self.fake.create(spec).await?;
        if !self.pulled.swap(true, Ordering::SeqCst) {
            tokio::time::sleep(self.pull).await;
        }
        Ok(())
    }
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError> {
        self.fake.inspect(name).await
    }
});

/// A node with the default turn limit (15 minutes) that hosts `Limited<SECS>` on `fake`, whose first create takes
/// `pull`, and the target; the container `j1` of `alpine:3` is created and sent its start.
async fn limited<const SECS: u64>(fake: &FakeRuntime, pull: Duration) -> (TestActors, Container<Limited<SECS>>) {
    let runtime = Arc::new(SlowPull { fake: fake.clone(), pull, pulled: AtomicBool::new(false) });
    let test = TestActors::builder()
        .with(move |node| node.host(ContainerActor::new(Limited::<SECS>, runtime, settings())).host(WatcherActor))
        .build()
        .await
        .unwrap();
    test.create(&Watcher, &Ends::default()).await;
    let job = Container::<Limited<SECS>>::named("j1");
    test.create(&job, &ContainerState::new(Run { image: "alpine:3".into() })).await;
    test.send(&job, start()).await;
    (test, job)
}

/// Scenario: A slow pull fits: a create step of 20 minutes, past the node's default of 15, within the job's 35.
#[tokio::test(start_paused = true)]
async fn a_create_step_past_the_nodes_default_fits_the_jobs_turn_limit() {
    let fake = FakeRuntime::new();
    let (test, job) = limited::<{ 35 * 60 }>(&fake, 20 * 60 * SECOND).await;
    test.run_until_idle().await;
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Running);
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Started);
    assert!(test.failures().await.is_empty(), "no turn fails");
    let poll = test.pending(&job).await.into_iter().next().expect("the next poll");
    assert_eq!((poll.failures, poll.last_error), (0, None), "the step ended done");
}

/// Scenario: A limit below the create step: the step fails with `timeout`, its retry runs it again on the container
/// the cut-off step made.
#[tokio::test(start_paused = true)]
async fn a_create_step_past_the_jobs_turn_limit_times_out_and_its_retry_makes_no_second_container() {
    let fake = FakeRuntime::new();
    let (test, job) = limited::<60>(&fake, 120 * SECOND).await;
    test.run_until_idle().await;
    let poll = test.pending(&job).await.into_iter().next().expect("the poll, to retry");
    assert_eq!(poll.failures, 1);
    assert!(poll.last_error.as_deref().is_some_and(|e| e.contains("timeout")), "{:?}", poll.last_error);
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Created, "made, never started");
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.errors), (Phase::New, 0), "not a runtime error");

    test.advance_time_by(2 * SECOND).await;
    assert_eq!(fake.inspect("app-j1").await.unwrap(), ContainerStatus::Running);
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Started);
    assert_eq!(fake.list(&Label::new("made", "actor")).await.unwrap().len(), 1, "no second container");
    assert!(test.failures().await.is_empty(), "the retry ended done");
}

/// Scenario: A zero limit is refused at registration.
#[tokio::test(start_paused = true)]
async fn a_zero_turn_limit_is_refused_at_registration() {
    let runtime = Arc::new(FakeRuntime::new());
    let refused = TestActors::builder()
        .with(move |node| node.host(ContainerActor::new(Limited::<0>, runtime, settings())))
        .build()
        .await
        .err()
        .expect("refused");
    let problems: Vec<String> = refused.problems.iter().map(ToString::to_string).collect();
    assert!(problems.iter().any(|p| p.contains("MAX_TURN must be positive")), "{problems:?}");
}
