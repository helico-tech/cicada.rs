//! The container actor on virtual time (ADR 0057): `TestActors` and the fake runtime, one scenario of the spec per
//! test, with a target actor that keeps every end it is sent.

use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, Timestamp};
use cicada_containers::{
    Call, Container, ContainerActor, ContainerEnded, ContainerJob, ContainerMsg, ContainerRuntime, ContainerSettings,
    ContainerSpec, ContainerState, ContainerStatus, ErrorKind, FakeRuntime, Goal, Label, Method, Outcome, Phase,
};
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

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
