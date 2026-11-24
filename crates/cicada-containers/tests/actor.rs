//! The container actor on virtual time (ADR 0057): `TestActors` and the fake runtime, one scenario of the spec per
//! test, with a target actor that keeps every end it is sent. And its wire format: a test job's contract, whose type
//! name its host chooses, pinned in `tests/contracts`, with samples of every message, the state in each phase and an
//! end payload, taken by the schemas and the decoders (ADR 0026).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::time::{self, Timestamp};
use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_containers::{
    Call, Container, ContainerActor, ContainerEnded, ContainerJob, ContainerMsg, ContainerRuntime, ContainerSettings,
    ContainerSpec, ContainerState, ContainerStatus, ErrorKind, FakeRuntime, Goal, Label, Method, Outcome, Phase,
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

/// The test job: builds a spec with a secret and a label from the request.
struct TestJob;

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
        Ok(spec)
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
    let runtime: Arc<dyn ContainerRuntime> = Arc::new(fake.clone());
    let test = TestActors::builder()
        .with(move |node| node.host(ContainerActor::new(TestJob, runtime, settings)).host(WatcherActor))
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
            Call::Start(_) => "start",
            Call::Inspect(_) => "inspect",
            Call::Logs { .. } => "logs",
            Call::Stop { .. } => "stop",
            Call::Remove(_) => "remove",
            _ => "other",
        })
        .collect()
}

/// Scenario: The spec is built, not stored. Scenario: An exit is reported with its logs.
#[tokio::test(start_paused = true)]
async fn the_spec_is_built_at_the_create_and_an_exit_is_reported_with_its_logs() {
    let fake = FakeRuntime::new();
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "alpine:3").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;

    let Call::Create(spec) = &fake.calls()[0] else { panic!("{:?}", fake.calls()) };
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
    assert_eq!((end.outcome, end.code, end.logs.as_str()), (Outcome::Exited, Some(3), "done\n"));
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
    assert_eq!(end.outcome, Outcome::Failed);
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
    assert_eq!((end.outcome, end.code, end.logs.as_str()), (Outcome::Stopped, None, ""));
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

/// Scenario: A missing image.
#[tokio::test(start_paused = true)]
async fn a_missing_image_ends_failed_at_the_first_poll() {
    let fake = FakeRuntime::new();
    fake.fail_next(Method::Create, ErrorKind::ImageNotFound);
    let test = harness(&fake, settings()).await;
    let job = container(&test, "j1", "nope:1").await;
    test.send(&job, start()).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!(end.outcome, Outcome::Failed);
    assert!(end.reason.starts_with("image not found"), "{}", end.reason);
    assert_eq!((state.errors, state.armed), (0, None), "no backoff");
    test.advance_time_by(120 * SECOND).await;
    assert_eq!(methods(&fake), ["create"], "no further create");
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
    assert_eq!((end.outcome, end.reason.as_str()), (Outcome::Failed, "no image"));
    assert!(fake.calls().is_empty());
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

    assert_eq!(methods(&fake), ["create", "start", "inspect", "logs", "remove"]);
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
    assert_eq!(methods(&fake), ["create", "start", "stop", "inspect", "logs", "remove"]);
    let state: State = test.state(&job).await;
    let end = state.end.unwrap();
    assert_eq!((end.outcome, end.code, end.logs.as_str()), (Outcome::Stopped, Some(143), "working\n"));
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
    assert_eq!((end.outcome, end.code), (Outcome::TimedOut, Some(143)));
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
    assert_eq!(end.outcome, Outcome::Failed);
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
    assert_eq!(end.outcome, Outcome::Failed);
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
    assert_eq!(methods(&fake), ["create", "start"], "a poll that is not armed does nothing");
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
    fake.exit_with("app-j1", ContainerStatus::Failed { code: Some(137), reason: "OOMKilled".into() });
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
    assert_eq!(end, &json!({"outcome": "failed", "code": 137, "reason": "OOMKilled", "logs": "done\n"}));

    let ended = serde_json::to_value(&ends(&test).await[0]).unwrap();
    assert_eq!(ended, json!({"key": "j1", "end": end}));
    let mut sample = json!({"type": "Ended"});
    sample.as_object_mut().unwrap().extend(ended.as_object().unwrap().clone());
    assert_message_sample::<WatcherMsg>(&sample);

    for invalid in [
        json!({}),
        json!({"request": {"image": 1}}),
        json!({"request": {"image": "a"}, "phase": "paused"}),
        json!({"request": {"image": "a"}, "goal": "pause"}),
        json!({"request": {"image": "a"}, "goal": "stop", "phase": "ended"}),
        json!({"request": {"image": "a"}, "phase": "started"}),
        json!({"request": {"image": "a"}, "armed": "m1"}),
        json!({"request": {"image": "a"}, "goal": "stop", "phase": "ended", "end": {"outcome": "gone", "reason": ""}}),
    ] {
        assert!(serde_json::from_value::<State>(invalid.clone()).is_err(), "{invalid}");
    }
}
