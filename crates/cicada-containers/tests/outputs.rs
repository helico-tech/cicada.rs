//! The container actor with an output store (ADR 0066), on virtual time with `TestActors`, the fake runtime and the
//! memory store: at an end the container ran for, the rest of its log and its job's outputs go into the store before
//! the end is recorded and sent with its report. One scenario of the spec per test, and the design's tests.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use cicada::prelude::*;
use cicada::time::MAX_INSTANT;
use cicada_axum::{assert_message_sample, assert_state_sample};
use cicada_containers::{
    ArchiveSettings, Call, Container, ContainerActor, ContainerEnded, ContainerError, ContainerJob, ContainerMsg,
    ContainerProgress, ContainerRuntime, ContainerSettings, ContainerSpec, ContainerState, ContainerStatus,
    ContainerSummary, CopyFile, CopyLimits, CopyOut, ErrorKind, FakeRuntime, Goal, Label, LogArchiver, LogCursor,
    LogEnd, LogPage, MemoryOutputStore, Method, NetworkSpec, Outcome, OutputError, OutputErrorKind, OutputStore,
    Outputs, OutputsReport, PREFIX_LABEL, Phase, SkippedFile, StoredFile, StoredLine, Swept, VolumeSpec,
};
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Notify;

const SECOND: Duration = Duration::from_secs(1);
/// The container of `j1`.
const NAME: &str = "app-j1";

/// The request of a test container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Run {
    /// The image to run.
    image: String,
}

/// The test job: its outputs folder, or none.
struct Job {
    outputs: Option<Outputs>,
}

impl Job {
    /// Outputs at `/work/out` with the default limits.
    fn out() -> Self {
        Self { outputs: Some(Outputs::new("/work/out")) }
    }

    /// Outputs at `/work/out` with at most `max_bytes` bytes.
    fn bytes(max_bytes: u64) -> Self {
        let mut outputs = Outputs::new("/work/out");
        outputs.limits = CopyLimits::new(1000, max_bytes);
        Self { outputs: Some(outputs) }
    }
}

impl ContainerJob for Job {
    const TYPE: &'static str = "test.collect";
    type Request = Run;

    fn spec(&self, name: &str, run: &Run) -> Result<ContainerSpec, String> {
        Ok(ContainerSpec::new(name, run.image.clone()))
    }

    fn outputs(&self, _: &Run) -> Option<Outputs> {
        self.outputs.clone()
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
    /// A container's progress.
    Progress(ContainerProgress),
}

/// What the target got, in order: `progress <seq>` or `end <key>`, and the ends.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Got {
    /// The ends.
    ends: Vec<ContainerEnded>,
    /// What came, in order.
    order: Vec<String>,
}

struct WatcherActor;

impl Actor for WatcherActor {
    type Ref = Watcher;
    type State = Got;

    async fn handle(&self, turn: &mut Turn<Self>, message: WatcherMsg) -> HandlerResult {
        let state = turn.state_mut();
        match message {
            WatcherMsg::Ended(ended) => {
                state.order.push(format!("end {}", ended.key));
                state.ends.push(ended);
            }
            WatcherMsg::Progress(progress) => state.order.push(format!("progress {}", progress.seq)),
        }
        Ok(())
    }
}

type State = ContainerState<Run>;

fn settings() -> ContainerSettings {
    ContainerSettings::new("app-").unwrap()
}

/// A node that hosts `job` on `runtime` with `settings`, with `store` when given, and the target.
async fn harness_on(
    runtime: Arc<dyn ContainerRuntime>,
    store: Option<Arc<dyn OutputStore>>,
    settings: ContainerSettings,
    job: Job,
) -> TestActors {
    let test = TestActors::builder()
        .with(move |node| {
            let actor = ContainerActor::new(job, runtime, settings);
            let actor = match store {
                Some(store) => actor.with_store(store),
                None => actor,
            };
            node.host(actor).host(WatcherActor)
        })
        .build()
        .await
        .unwrap();
    test.create(&Watcher, &Got::default()).await;
    test
}

/// A node with `job` on `fake` and `store`, with the default settings.
async fn harness(fake: &FakeRuntime, store: &MemoryOutputStore, job: Job) -> TestActors {
    harness_on(Arc::new(fake.clone()), Some(Arc::new(store.clone())), settings(), job).await
}

/// The container keyed `key`, created, started and running.
async fn started(test: &TestActors, key: &str) -> Container<Job> {
    let container = Container::<Job>::named(key);
    test.create(&container, &ContainerState::new(Run { image: "alpine:3".into() })).await;
    test.send(&container, ContainerMsg::start(&Watcher, "Ended").unwrap()).await;
    test.run_until_idle().await;
    container
}

/// Writes `contents` to `path` of `app-j1`.
fn write(fake: &FakeRuntime, path: &str, contents: impl Into<Vec<u8>>) {
    fake.put_file(NAME, CopyFile::new(path, contents));
}

/// Exits `app-j1` with code 0 and runs the poll that sees it, and what follows at once.
async fn exit(test: &TestActors, fake: &FakeRuntime) {
    fake.exit_with(NAME, ContainerStatus::Exited { code: 0 });
    test.advance_time_by(2 * SECOND).await;
}

async fn ends(test: &TestActors) -> Vec<ContainerEnded> {
    test.state::<_, Got>(&Watcher).await.ends
}

/// The one end the target got, with its report.
async fn the_end(test: &TestActors) -> (ContainerEnded, OutputsReport) {
    let ends = ends(test).await;
    assert_eq!(ends.len(), 1, "{ends:?}");
    let report = ends[0].end.outputs.clone().expect("an end with a report");
    (ends[0].clone(), report)
}

/// The texts of the stored log of `name`.
async fn stored_log(store: &dyn OutputStore, name: &str) -> Vec<String> {
    store.read_logs(name, 0, 100_000).await.unwrap().into_iter().map(|s| s.line.text).collect()
}

/// The paths of the stored files of `name`.
async fn stored_files(store: &dyn OutputStore, name: &str) -> Vec<String> {
    store.files(name).await.unwrap().into_iter().map(|f| f.path).collect()
}

fn count(fake: &FakeRuntime, of: fn(&Call) -> bool) -> usize {
    fake.calls().iter().filter(|call| of(call)).count()
}

fn copy_outs(fake: &FakeRuntime) -> usize {
    count(fake, |call| matches!(call, Call::CopyOut { .. }))
}

fn log_reads(fake: &FakeRuntime) -> usize {
    count(fake, |call| matches!(call, Call::LogsSince { .. }))
}

fn report(stored: u32, bytes: u64, skipped: &[(&str, &str)], error: Option<&str>) -> OutputsReport {
    OutputsReport {
        stored,
        bytes,
        skipped: skipped.len() as u32,
        first_skipped: skipped.iter().map(|(path, reason)| SkippedFile::new(*path, *reason)).collect(),
        error: error.map(str::to_owned),
    }
}

/// Scenario: Outputs at the end.
#[tokio::test(start_paused = true)]
async fn the_rest_of_the_log_and_the_outputs_go_into_the_store_before_the_end() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::bytes(10)).await;
    let job = started(&test, "j1").await;
    fake.set_logs(NAME, "one\ntwo");
    write(&fake, "/work/out/a.txt", "hello");
    write(&fake, "/work/out/big", vec![b'x'; 20]);
    write(&fake, "/work/elsewhere", "not an output");
    exit(&test, &fake).await;

    let (ended, outputs) = the_end(&test).await;
    assert_eq!((ended.key.as_str(), ended.end.outcome, ended.end.code), ("j1", Outcome::Exited, Some(0)));
    assert_eq!(ended.end.logs, "one\ntwo\n", "the tail, as before");
    assert_eq!(outputs, report(1, 5, &[("/work/out/big", "20 bytes, more than the 5 left")], None));
    assert_eq!(stored_files(&store, NAME).await, ["/work/out/a.txt"]);
    assert_eq!(store.read_file(NAME, "/work/out/a.txt").await.unwrap().unwrap().contents, b"hello");
    assert_eq!(stored_log(&store, NAME).await, ["one", "two"]);

    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.pending, state.errors, state.armed), (Phase::Ended, None, 0, None));
    assert_eq!(state.end.as_ref(), Some(&ended.end), "the saved end is the sent one");
    let Some(Call::CopyOut { path, limits, .. }) = fake.calls().into_iter().find(|c| matches!(c, Call::CopyOut { .. }))
    else {
        panic!("no copy: {:?}", fake.calls())
    };
    assert_eq!((path.as_str(), limits), ("/work/out", CopyLimits::new(1000, 10)));
    assert_state_sample::<State>(&test.state_json(&job).await.unwrap());
    let mut sample = serde_json::to_value(&ended).unwrap();
    assert_eq!(
        sample["end"]["outputs"],
        json!({"stored": 1, "bytes": 5, "skipped": 1,
            "firstSkipped": [{"path": "/work/out/big", "reason": "20 bytes, more than the 5 left"}], "error": null})
    );
    sample.as_object_mut().unwrap().insert("type".into(), json!("Ended"));
    assert_message_sample::<WatcherMsg>(&sample);
    assert!(test.failures().await.is_empty());
}

/// The defaults of an outputs folder.
#[test]
fn an_outputs_folder_has_64_mib_and_1000_files() {
    let outputs = Outputs::new("/work/out");
    assert_eq!((outputs.path.as_str(), outputs.limits), ("/work/out", CopyLimits::new(1000, 64 * 1024 * 1024)));
}

/// Scenario: A file the store refuses.
#[tokio::test(start_paused = true)]
async fn a_file_the_store_refuses_is_skipped_with_its_reason_and_the_others_are_stored() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    started(&test, "j1").await;
    for path in ["/work/out/a", "/work/out/b\u{1}c", "/work/out/d\0e", "/work/out/f"] {
        write(&fake, path, "x");
    }
    exit(&test, &fake).await;

    let (_, outputs) = the_end(&test).await;
    assert_eq!((outputs.stored, outputs.bytes, outputs.skipped, outputs.error), (2, 2, 2, None));
    assert_eq!(stored_files(&store, NAME).await, ["/work/out/a", "/work/out/f"]);
    let skipped: Vec<(&str, &str)> =
        outputs.first_skipped.iter().map(|s| (s.path.as_str(), s.reason.as_str())).collect();
    assert_eq!(
        skipped,
        [
            ("/work/out/b\u{1}c", "invalid: \"/work/out/b\\u{1}c\" has a control character"),
            ("/work/out/d\u{FFFD}e", "invalid: \"/work/out/d\\0e\" has a control character"),
        ],
        "a NUL in a path is stored as U+FFFD"
    );
}

/// More than 10 skipped entries: all are counted, the first 10 listed.
#[tokio::test(start_paused = true)]
async fn more_than_ten_skipped_are_counted_and_ten_are_listed() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let mut outputs = Outputs::new("/work/out");
    outputs.limits = CopyLimits::new(1000, 1).with_max_read(1000); // each skipped file is read too
    let test = harness(&fake, &store, Job { outputs: Some(outputs) }).await;
    started(&test, "j1").await;
    for i in 10..22 {
        write(&fake, &format!("/work/out/f{i}"), "xx");
    }
    exit(&test, &fake).await;

    let (_, outputs) = the_end(&test).await;
    assert_eq!((outputs.stored, outputs.skipped, outputs.first_skipped.len()), (0, 12, 10));
    assert_eq!(outputs.first_skipped[0], SkippedFile::new("/work/out/f10", "2 bytes, more than the 1 left"));
    assert_eq!(outputs.first_skipped[9].path, "/work/out/f19");
}

/// Scenario: No store. And a store without an outputs folder stores the log only, with counts of 0.
#[tokio::test(start_paused = true)]
async fn without_a_store_the_end_has_no_outputs_and_without_a_folder_the_counts_are_zero() {
    let fake = FakeRuntime::new();
    let test = harness_on(Arc::new(fake.clone()), None, settings(), Job::out()).await;
    let job = started(&test, "j1").await;
    fake.set_logs(NAME, "one");
    write(&fake, "/work/out/a", "x");
    fake.exit_with(NAME, ContainerStatus::Exited { code: 0 });
    assert_eq!(test.advance_time_by(2 * SECOND).await, 2, "the poll that records and sends, and the target's turn");
    let ended = ends(&test).await;
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].end.outputs, None);
    assert!(serde_json::to_value(&ended[0]).unwrap()["end"].get("outputs").is_none(), "absent on the wire");
    assert_eq!((log_reads(&fake), copy_outs(&fake)), (0, 0));
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Ended);

    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job { outputs: None }).await;
    started(&test, "j1").await;
    fake.set_logs(NAME, "one");
    write(&fake, "/work/out/a", "x");
    exit(&test, &fake).await;
    let (_, outputs) = the_end(&test).await;
    assert_eq!(outputs, OutputsReport::default());
    assert_eq!(stored_log(&store, NAME).await, ["one"]);
    assert!(stored_files(&store, NAME).await.is_empty());
    assert_eq!(copy_outs(&fake), 0);
}

/// Ends the container did not run for never collect: a refused spec, a missing container, too many errors.
#[tokio::test(start_paused = true)]
async fn ends_the_container_did_not_run_for_carry_no_outputs() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    let refused = Container::<Job>::named("a/b");
    test.create(&refused, &ContainerState::new(Run { image: "alpine:3".into() })).await;
    test.send(&refused, ContainerMsg::start(&Watcher, "Ended").unwrap()).await;
    test.run_until_idle().await;
    let missing = started(&test, "j2").await;
    fake.remove("app-j2").await.unwrap();
    test.advance_time_by(2 * SECOND).await;
    let stopped = Container::<Job>::named("j3");
    test.create(&stopped, &ContainerState::new(Run { image: "alpine:3".into() })).await;
    test.send(&stopped, ContainerMsg::Stop).await;
    test.send(&stopped, ContainerMsg::start(&Watcher, "Ended").unwrap()).await;
    test.run_until_idle().await;

    let ends = ends(&test).await;
    assert_eq!(ends.len(), 3, "{ends:?}");
    assert!(ends.iter().all(|ended| ended.end.outputs.is_none()), "{ends:?}");
    assert_eq!((log_reads(&fake), copy_outs(&fake)), (0, 0));
    assert_eq!(test.state::<_, State>(&missing).await.phase, Phase::Ended);
}

/// Scenario: Start and Remove during the collection. And a state in `Collecting` passes its sample.
#[tokio::test(start_paused = true)]
async fn a_start_and_a_remove_during_the_collection_get_the_end_once_after_it() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    let job = started(&test, "j1").await;
    write(&fake, "/work/out/a", "x");
    fake.fail_always(Method::CopyOut, ErrorKind::DaemonUnavailable);
    exit(&test, &fake).await;

    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.end.is_none(), state.errors), (Phase::Collecting, true, 1));
    assert_eq!(state.pending.as_ref().map(|end| end.outcome), Some(Outcome::Exited));
    let collecting = test.state_json(&job).await.unwrap();
    assert_eq!(collecting["pending"]["outcome"], "exited");
    assert!(collecting["pending"].get("outputs").is_none());
    assert_state_sample::<State>(&collecting);
    assert!(ends(&test).await.is_empty(), "no end before the collection finished");

    // A second Start names a type the target does not take: the end goes to the first one. The Stop and the Remove
    // arm one poll at once, the Remove's in place of the Stop's, and the copy still fails.
    test.send(&job, ContainerMsg::start(&Watcher, "Elsewhere").unwrap()).await;
    test.send(&job, ContainerMsg::Stop).await;
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.goal, state.errors), (Phase::Collecting, Some(Goal::Remove), 2));
    assert!(ends(&test).await.is_empty());
    assert_eq!(fake.inspect(NAME).await.unwrap(), ContainerStatus::Exited { code: 0 }, "not removed yet");

    fake.clear_failures();
    test.advance_time_by(8 * SECOND).await;

    let (ended, outputs) = the_end(&test).await;
    assert_eq!((ended.end.outcome, outputs.stored), (Outcome::Exited, 1));
    let state: State = test.state(&job).await;
    assert_eq!(
        (state.phase, state.undelivered, state.target.unwrap().message_type),
        (Phase::Removed, None, "Ended".into())
    );
    assert_eq!(fake.inspect(NAME).await.unwrap(), ContainerStatus::Missing);
    let calls = fake.calls();
    let last_copy = calls.iter().rposition(|c| matches!(c, Call::CopyOut { .. })).unwrap();
    let remove = calls.iter().position(|c| matches!(c, Call::Remove(_))).unwrap();
    assert!(last_copy < remove, "removed after the collection: {calls:?}");
    assert_eq!(count(&fake, |c| matches!(c, Call::Stop { .. })), 0, "a Stop during the collection stops nothing");
    assert_eq!(stored_files(&store, NAME).await, ["/work/out/a"], "the store outlives the container");
}

/// Scenario: An end saved by 0.22.
#[tokio::test(start_paused = true)]
async fn an_end_saved_by_022_collects_nothing_and_sends_no_second_end() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    fake.create(&ContainerSpec::new(NAME, "alpine:3")).await.unwrap();
    fake.start(NAME).await.unwrap();
    fake.exit_with(NAME, ContainerStatus::Exited { code: 0 });
    let job = Container::<Job>::named("j1");
    let saved = json!({"request": {"image": "alpine:3"}, "goal": "run", "phase": "ended",
        "target": {"to": "test.watcher/_", "messageType": "Ended"}, "deadline": null,
        "end": {"outcome": "exited", "code": 0, "kind": null, "reason": "exited with code 0", "logs": ""},
        "undelivered": null, "armed": null, "due": null, "errors": 0, "progressSeq": 0, "progressHash": null});
    test.create(&job, &saved).await;
    test.send(&job, ContainerMsg::Check).await;
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;

    assert!(ends(&test).await.is_empty());
    assert_eq!((log_reads(&fake), copy_outs(&fake)), (0, 0));
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.end.unwrap().outputs), (Phase::Removed, None));
}

/// A long log is stored across turns, at most 20 pages of 500 lines in each.
#[tokio::test(start_paused = true)]
async fn a_long_log_is_drained_across_several_turns() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    let job = started(&test, "j1").await;
    let lines: Vec<String> = (0..25_000).map(|i| format!("l{i}")).collect();
    fake.set_logs(NAME, lines.join("\n"));
    fake.exit_with(NAME, ContainerStatus::Exited { code: 0 });
    tokio::time::sleep(2 * SECOND).await;

    // The poll that sees the exit, then one turn per 20 pages.
    let mut reads = Vec::new();
    for _ in 0..4 {
        assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
        reads.push(log_reads(&fake));
    }
    assert_eq!(reads, [0, 20, 40, 51], "10 000, 10 000, then 5 000 lines and a short page");
    let state: State = test.state(&job).await;
    assert_eq!(state.phase, Phase::Ended);
    assert_eq!(stored_log(&store, NAME).await, lines);
    let max = fake.calls().into_iter().filter_map(|c| match c {
        Call::LogsSince { max_lines, .. } => Some(max_lines),
        _ => None,
    });
    assert!(max.into_iter().all(|m| m == 500));
}

/// An output store whose first `append_logs` stores the same page first, as a log archiver running next to the actor
/// would, so the actor's append fails with `Conflict`; and whose `put_file` fails with `Unavailable` a number of times.
#[derive(Default)]
struct Racing {
    store: MemoryOutputStore,
    races: AtomicUsize,
    unavailable: AtomicUsize,
}

#[async_trait]
impl OutputStore for Racing {
    async fn log_end(&self, container: &str) -> Result<LogEnd, OutputError> {
        self.store.log_end(container).await
    }

    async fn append_logs(
        &self,
        container: &str,
        after: Option<&LogCursor>,
        page: &LogPage,
    ) -> Result<u64, OutputError> {
        if self.races.fetch_sub(1, Ordering::SeqCst) > 0 {
            self.store.append_logs(container, after, page).await?;
        }
        self.store.append_logs(container, after, page).await
    }

    async fn read_logs(&self, container: &str, from: u64, limit: u32) -> Result<Vec<StoredLine>, OutputError> {
        self.store.read_logs(container, from, limit).await
    }

    async fn put_file(&self, container: &str, file: &CopyFile) -> Result<(), OutputError> {
        if self.unavailable.load(Ordering::SeqCst) > 0 {
            self.unavailable.fetch_sub(1, Ordering::SeqCst);
            return Err(OutputError::new(OutputErrorKind::Unavailable, "the store is down"));
        }
        self.store.put_file(container, file).await
    }

    async fn files(&self, container: &str) -> Result<Vec<StoredFile>, OutputError> {
        self.store.files(container).await
    }

    async fn read_file(&self, container: &str, path: &str) -> Result<Option<CopyFile>, OutputError> {
        self.store.read_file(container, path).await
    }

    async fn remove(&self, container: &str) -> Result<(), OutputError> {
        self.store.remove(container).await
    }
}

/// A log archiver next to the actor: what it stored is not stored again, also when it races the end's append.
#[tokio::test(start_paused = true)]
async fn a_running_archiver_and_the_end_store_each_line_once() {
    let fake = FakeRuntime::new();
    let racing = Arc::new(Racing { races: AtomicUsize::new(1), ..Racing::default() });
    let store = racing.store.clone();
    let test = harness_on(Arc::new(fake.clone()), Some(racing.clone()), settings(), Job::out()).await;
    started(&test, "j1").await;
    fake.set_logs(NAME, "a\nb");
    let archiver =
        LogArchiver::new(Arc::new(fake.clone()), Arc::new(store.clone()), "app-", ArchiveSettings::new()).unwrap();
    archiver.run(tokio::time::sleep(SECOND)).await;
    assert_eq!(stored_log(&store, NAME).await, ["a", "b"], "the archiver followed the running container");
    assert!(fake.list(&Label::new(PREFIX_LABEL, "app-")).await.unwrap().iter().any(|c| c.name == NAME));

    fake.set_logs(NAME, "a\nb\nc\nd");
    exit(&test, &fake).await;
    assert_eq!(stored_log(&store, NAME).await, ["a", "b", "c", "d"], "each line once");
    let (_, outputs) = the_end(&test).await;
    assert_eq!(outputs.error, None);
}

/// A copy that fails with `DaemonUnavailable` counts, backs off and succeeds on the next poll.
#[tokio::test(start_paused = true)]
async fn a_daemon_error_of_the_copy_retries_and_then_succeeds() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    let job = started(&test, "j1").await;
    write(&fake, "/work/out/a", "abc");
    fake.fail_next(Method::CopyOut, ErrorKind::DaemonUnavailable);
    exit(&test, &fake).await;
    assert_eq!(test.state::<_, State>(&job).await.errors, 1);
    test.advance_time_by(3 * SECOND).await;
    assert!(ends(&test).await.is_empty(), "the backoff after one error is 4 s");
    test.advance_time_by(SECOND).await;
    let (_, outputs) = the_end(&test).await;
    assert_eq!(outputs, report(1, 3, &[], None));
    assert_eq!(test.state::<_, State>(&job).await.errors, 0);
}

/// Scenario: A store that comes back.
#[tokio::test(start_paused = true)]
async fn a_store_that_comes_back_gets_the_files_and_the_end_has_no_error() {
    let fake = FakeRuntime::new();
    let racing = Arc::new(Racing { unavailable: AtomicUsize::new(2), ..Racing::default() });
    let store = racing.store.clone();
    let test = harness_on(Arc::new(fake.clone()), Some(racing), settings(), Job::out()).await;
    started(&test, "j1").await;
    write(&fake, "/work/out/a", "abc");
    exit(&test, &fake).await;
    test.advance_time_by(4 * SECOND).await;
    assert!(ends(&test).await.is_empty(), "two polls failed");
    test.advance_time_by(8 * SECOND).await;
    let (_, outputs) = the_end(&test).await;
    assert_eq!(outputs, report(1, 3, &[], None));
    assert_eq!(stored_files(&store, NAME).await, ["/work/out/a"]);
    assert_eq!(copy_outs(&fake), 3);
}

/// Scenario: Errors while outputs are collected.
#[tokio::test(start_paused = true)]
async fn too_many_errors_while_collecting_deliver_the_end_with_its_outcome_and_the_error() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let mut settings = settings();
    settings.max_errors = 3;
    let test = harness_on(Arc::new(fake.clone()), Some(Arc::new(store.clone())), settings, Job::out()).await;
    started(&test, "j1").await;
    fake.set_logs(NAME, "kept");
    fake.fail_always(Method::CopyOut, ErrorKind::DaemonUnavailable);
    exit(&test, &fake).await;
    test.advance_time_by(4 * SECOND + 8 * SECOND).await;

    let (ended, outputs) = the_end(&test).await;
    assert_eq!((ended.end.outcome, ended.end.code, ended.end.kind), (Outcome::Exited, Some(0), None));
    let error = outputs.error.clone().unwrap();
    assert!(error.starts_with("(no outputs: failed 3 times in a row: daemon unavailable: "), "{error}");
    assert_eq!((outputs.stored, outputs.skipped), (0, 0));
    assert_eq!(copy_outs(&fake), 3);
    assert_eq!(stored_log(&store, NAME).await, ["kept"], "the log stored before the errors stays");
}

/// Scenario: A bad outputs path. And a runtime without `copy_out` ends the collection at once; one without
/// `logs_since` skips the log.
#[tokio::test(start_paused = true)]
async fn a_bad_path_and_a_missing_copy_end_the_collection_at_once_and_a_missing_log_is_skipped() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job { outputs: Some(Outputs::new("work/out")) }).await;
    let job = started(&test, "j1").await;
    exit(&test, &fake).await;
    let (_, outputs) = the_end(&test).await;
    let expected = "(no outputs: invalid spec: copy out of app-j1: relative path work/out)";
    assert_eq!(outputs, report(0, 0, &[], Some(expected)));
    assert_eq!((copy_outs(&fake), test.state::<_, State>(&job).await.errors), (1, 0));

    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    started(&test, "j1").await;
    fake.fail_always(Method::CopyOut, ErrorKind::Unsupported);
    exit(&test, &fake).await;
    let (_, outputs) = the_end(&test).await;
    assert!(outputs.error.unwrap().starts_with("(no outputs: unsupported: "));
    assert_eq!(copy_outs(&fake), 1);

    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    started(&test, "j1").await;
    fake.set_logs(NAME, "never stored");
    write(&fake, "/work/out/a", "x");
    fake.fail_always(Method::LogsSince, ErrorKind::Unsupported);
    exit(&test, &fake).await;
    let (_, outputs) = the_end(&test).await;
    assert_eq!(outputs, report(1, 1, &[], None));
    assert!(stored_log(&store, NAME).await.is_empty());
}

/// Scenario: A container removed during the collection.
#[tokio::test(start_paused = true)]
async fn a_container_removed_during_the_collection_delivers_what_is_stored() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    started(&test, "j1").await;
    fake.set_logs(NAME, "kept");
    write(&fake, "/work/out/a", "x");
    fake.fail_next(Method::CopyOut, ErrorKind::DaemonUnavailable);
    exit(&test, &fake).await;
    fake.remove(NAME).await.unwrap();
    test.advance_time_by(4 * SECOND).await;

    let (ended, outputs) = the_end(&test).await;
    assert_eq!(ended.end.outcome, Outcome::Exited);
    assert_eq!(outputs, report(0, 0, &[], Some("(no outputs: conflict: no container app-j1)")));
    assert_eq!(stored_log(&store, NAME).await, ["kept"]);
}

/// The last progress goes in the turn that finishes the collection, before the end.
#[tokio::test(start_paused = true)]
async fn with_a_store_the_last_progress_is_read_when_the_collection_finishes() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let test = harness(&fake, &store, Job::out()).await;
    let job = Container::<Job>::named("j1");
    test.create(&job, &ContainerState::new(Run { image: "alpine:3".into() })).await;
    test.send(&job, ContainerMsg::start_with_progress(&Watcher, "Ended", "Progress").unwrap()).await;
    test.run_until_idle().await;
    write(&fake, "/tmp/cicada-progress", "done");
    write(&fake, "/work/out/a", "x");
    exit(&test, &fake).await;

    assert_eq!(test.state::<_, Got>(&Watcher).await.order, ["progress 1", "end j1"]);
    let paths: Vec<String> = fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::CopyOut { path, .. } => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(paths, ["/work/out", "/tmp/cicada-progress"], "the outputs, then the progress, once");
}

/// The fake, whose `copy_out` of one container waits for a gate; counts the copies of that container begun.
struct Gated {
    fake: FakeRuntime,
    slow: &'static str,
    gate: Arc<Notify>,
    begun: Arc<AtomicUsize>,
}

#[async_trait]
impl ContainerRuntime for Gated {
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError> {
        self.fake.create(spec).await
    }
    async fn copy_in(&self, name: &str, files: &[CopyFile]) -> Result<(), ContainerError> {
        self.fake.copy_in(name, files).await
    }
    async fn start(&self, name: &str) -> Result<(), ContainerError> {
        self.fake.start(name).await
    }
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError> {
        self.fake.inspect(name).await
    }
    async fn logs(&self, name: &str, tail_lines: u32) -> Result<String, ContainerError> {
        self.fake.logs(name, tail_lines).await
    }
    async fn logs_since(
        &self,
        name: &str,
        cursor: Option<LogCursor>,
        max_lines: u32,
    ) -> Result<LogPage, ContainerError> {
        self.fake.logs_since(name, cursor, max_lines).await
    }
    async fn copy_out(&self, name: &str, path: &str, limits: CopyLimits) -> Result<CopyOut, ContainerError> {
        if name == self.slow {
            self.begun.fetch_add(1, Ordering::SeqCst);
            self.gate.notified().await;
        }
        self.fake.copy_out(name, path, limits).await
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
}

/// Scenario: A slow copy.
#[tokio::test(start_paused = true)]
async fn a_slow_copy_holds_up_only_its_own_container() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let gate = Arc::new(Notify::new());
    let runtime = Gated { fake: fake.clone(), slow: "app-slow", gate: gate.clone(), begun: Arc::default() };
    let test = harness_on(Arc::new(runtime), Some(Arc::new(store.clone())), settings(), Job::out()).await;
    let slow = started(&test, "slow").await;
    let fast = started(&test, "fast").await;
    for name in ["app-slow", "app-fast"] {
        fake.put_file(name, CopyFile::new("/work/out/a", name));
        fake.exit_with(name, ContainerStatus::Exited { code: 0 });
    }
    tokio::time::sleep(2 * SECOND).await;
    assert_eq!(test.system().run_due_turns(10).await.unwrap(), 2, "both see their exits");

    // Both collect at once, as a node runs the turns of different actors in parallel.
    let system = test.system().clone();
    let collecting = tokio::spawn(async move { system.run_due_turns(10).await.unwrap() });
    let mut fast_state: State = test.state(&fast).await;
    for _ in 0..100 {
        if fast_state.phase == Phase::Ended {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        fast_state = test.state(&fast).await;
    }
    assert_eq!(fast_state.phase, Phase::Ended, "the fast end is recorded and sent while the slow copy waits");
    assert_eq!(test.state::<_, State>(&slow).await.phase, Phase::Collecting);
    gate.notify_one();
    assert_eq!(collecting.await.unwrap(), 2);
    test.run_until_idle().await;
    assert_eq!(test.state::<_, Got>(&Watcher).await.order, ["end fast", "end slow"]);
    assert_eq!(stored_files(&store, "app-slow").await, ["/work/out/a"]);
}

/// The test job with a `MAX_TURN` of 60 s and outputs at `/work/out`.
struct Limited;

impl ContainerJob for Limited {
    const TYPE: &'static str = "test.limited";
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(60));
    type Request = Run;

    fn spec(&self, name: &str, run: &Run) -> Result<ContainerSpec, String> {
        Ok(ContainerSpec::new(name, run.image.clone()))
    }

    fn outputs(&self, _: &Run) -> Option<Outputs> {
        Some(Outputs::new("/work/out"))
    }
}

/// Scenario: A copy longer than the turn limit. Each collecting try times out in the copy; after three, the next
/// sends the end with the error and what the store holds, and makes no fourth copy.
#[tokio::test(start_paused = true)]
async fn a_copy_that_always_times_out_ends_after_three_tries() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    // A gate never opened: every copy of `app-j1` waits past the turn limit.
    let begun = Arc::new(AtomicUsize::new(0));
    let runtime =
        Arc::new(Gated { fake: fake.clone(), slow: NAME, gate: Arc::new(Notify::new()), begun: begun.clone() });
    let output_store: Arc<dyn OutputStore> = Arc::new(store.clone());
    let test = TestActors::builder()
        .with(move |node| {
            node.host(ContainerActor::new(Limited, runtime, settings()).with_store(output_store)).host(WatcherActor)
        })
        .build()
        .await
        .unwrap();
    test.create(&Watcher, &Got::default()).await;
    let job = Container::<Limited>::named("j1");
    test.create(&job, &ContainerState::new(Run { image: "alpine:3".into() })).await;
    test.send(&job, ContainerMsg::start(&Watcher, "Ended").unwrap()).await;
    test.run_until_idle().await;
    // A file an earlier try stored before its turn was cut off.
    store.put_file(NAME, &CopyFile::new("/work/out/a", "abc")).await.unwrap();
    exit(&test, &fake).await;
    test.advance_time_by(10 * 60 * SECOND).await;

    let (ended, outputs) = the_end(&test).await;
    assert_eq!(ended.end.outcome, Outcome::Exited, "the end keeps its outcome");
    let error = outputs.error.clone().unwrap();
    assert!(error.starts_with("(no outputs: the collection failed 3 times; last: "), "{error}");
    assert!(error.contains("timeout"), "{error}");
    assert_eq!((outputs.stored, outputs.bytes, outputs.skipped), (1, 3, 0), "what the store holds");
    assert_eq!(begun.load(Ordering::SeqCst), 3, "no fourth copy");
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.pending, state.errors), (Phase::Ended, None, 0));
}

/// An output store whose `remove` fails with the given kinds first, in order, and counts its calls.
#[derive(Default)]
struct Flaky {
    store: MemoryOutputStore,
    remove_errors: Mutex<VecDeque<OutputErrorKind>>,
    removes: AtomicUsize,
}

impl Flaky {
    fn failing(kinds: &[OutputErrorKind]) -> Arc<Self> {
        Arc::new(Self { remove_errors: Mutex::new(kinds.iter().copied().collect()), ..Self::default() })
    }
}

#[async_trait]
impl OutputStore for Flaky {
    async fn log_end(&self, container: &str) -> Result<LogEnd, OutputError> {
        self.store.log_end(container).await
    }

    async fn append_logs(
        &self,
        container: &str,
        after: Option<&LogCursor>,
        page: &LogPage,
    ) -> Result<u64, OutputError> {
        self.store.append_logs(container, after, page).await
    }

    async fn read_logs(&self, container: &str, from: u64, limit: u32) -> Result<Vec<StoredLine>, OutputError> {
        self.store.read_logs(container, from, limit).await
    }

    async fn put_file(&self, container: &str, file: &CopyFile) -> Result<(), OutputError> {
        self.store.put_file(container, file).await
    }

    async fn files(&self, container: &str) -> Result<Vec<StoredFile>, OutputError> {
        self.store.files(container).await
    }

    async fn read_file(&self, container: &str, path: &str) -> Result<Option<CopyFile>, OutputError> {
        self.store.read_file(container, path).await
    }

    async fn remove(&self, container: &str) -> Result<(), OutputError> {
        self.removes.fetch_add(1, Ordering::SeqCst);
        if let Some(kind) = self.remove_errors.lock().unwrap().pop_front() {
            return Err(OutputError::new(kind, "the store said no"));
        }
        self.store.remove(container).await
    }
}

const DAY: Duration = Duration::from_secs(24 * 3600);

/// The settings with an output retention of `retention`.
fn retained(retention: Duration) -> ContainerSettings {
    let mut settings = settings();
    settings.output_retention = Some(retention);
    settings
}

/// A node with `store` and `settings`, and the container `j1` with stored outputs, removed.
async fn removed(
    fake: &FakeRuntime,
    store: Arc<dyn OutputStore>,
    settings: ContainerSettings,
) -> (TestActors, Container<Job>) {
    let test = harness_on(Arc::new(fake.clone()), Some(store), settings, Job::out()).await;
    let job = started(&test, "j1").await;
    fake.set_logs(NAME, "one");
    write(fake, "/work/out/a", "abc");
    exit(&test, fake).await;
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, State>(&job).await.phase, Phase::Removed);
    (test, job)
}

/// True while `store` still holds the stored outputs of `app-j1`.
async fn kept(store: &dyn OutputStore) -> bool {
    let (files, log) = (stored_files(store, NAME).await, stored_log(store, NAME).await);
    assert_eq!(files.is_empty(), log.is_empty(), "the files and the log go together: {files:?} {log:?}");
    !files.is_empty()
}

/// Scenario: A week after removal. The actor's own record stays.
#[tokio::test(start_paused = true)]
async fn stored_outputs_are_deleted_a_retention_after_the_removal_and_not_before() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let (test, job) = removed(&fake, Arc::new(store.clone()), retained(7 * DAY)).await;
    let state: State = test.state(&job).await;
    let until = cicada::time::add(test.now(), 7 * DAY).unwrap();
    assert_eq!((state.outputs_until, state.due), (Some(until), Some(until)));
    assert_state_sample::<State>(&test.state_json(&job).await.unwrap());
    let calls = fake.calls();
    assert!(calls.iter().any(|c| matches!(c, Call::Remove(_))), "the container is removed first: {calls:?}");

    test.advance_time_by(6 * DAY).await;
    assert!(kept(&store).await, "still there after 6 days");
    test.advance_time_by(DAY).await;
    assert!(!kept(&store).await, "gone after 7");
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.outputs_until, state.armed, state.errors), (Phase::Removed, None, None, 0));
    assert_eq!(state.end.unwrap().outcome, Outcome::Exited, "the actor's record stays");
    assert_eq!(ends(&test).await.len(), 1);
    assert!(test.failures().await.is_empty());
}

/// Scenario: No retention. And with a retention but no store, nothing waits.
#[tokio::test(start_paused = true)]
async fn without_a_retention_or_a_store_nothing_is_deleted() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let (test, job) = removed(&fake, Arc::new(store.clone()), settings()).await;
    test.advance_time_by(365 * DAY).await;
    assert!(kept(&store).await);
    let state: State = test.state(&job).await;
    assert_eq!((state.outputs_until, state.armed), (None, None));

    let fake = FakeRuntime::new();
    let test = harness_on(Arc::new(fake.clone()), None, retained(DAY), Job::out()).await;
    let job = started(&test, "j1").await;
    exit(&test, &fake).await;
    test.send(&job, ContainerMsg::Remove).await;
    test.run_until_idle().await;
    let state: State = test.state(&job).await;
    assert_eq!((state.phase, state.outputs_until, state.armed), (Phase::Removed, None, None));
}

/// A retention of zero deletes in the poll right after the removal; one past the largest instant is capped there.
#[tokio::test(start_paused = true)]
async fn a_zero_retention_deletes_at_once_and_a_huge_one_is_capped() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let (test, job) = removed(&fake, Arc::new(store.clone()), retained(Duration::ZERO)).await;
    assert!(!kept(&store).await);
    assert_eq!(test.state::<_, State>(&job).await.outputs_until, None);

    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let (test, job) = removed(&fake, Arc::new(store.clone()), retained(Duration::MAX)).await;
    assert_eq!(test.state::<_, State>(&job).await.outputs_until, Some(MAX_INSTANT));
    assert!(kept(&store).await);
}

/// Scenario: A store that is down at the due time. `Other` keeps trying too, with the backoff and without limit.
#[tokio::test(start_paused = true)]
async fn a_store_down_at_the_due_time_is_tried_again_with_the_backoff_and_then_deletes() {
    let fake = FakeRuntime::new();
    let flaky = Flaky::failing(&[OutputErrorKind::Unavailable, OutputErrorKind::Other, OutputErrorKind::Unavailable]);
    let mut settings = retained(DAY);
    settings.max_errors = 2;
    let (test, job) = removed(&fake, flaky.clone(), settings).await;
    test.advance_time_by(DAY).await;
    assert_eq!(flaky.removes.load(Ordering::SeqCst), 1);
    let state: State = test.state(&job).await;
    assert_eq!((state.errors, state.outputs_until.is_some()), (1, true));
    // The backoff: 4, 8 and 16 s; past `max_errors` it keeps trying.
    test.advance_time_by(4 * SECOND + 8 * SECOND + 15 * SECOND).await;
    assert_eq!(flaky.removes.load(Ordering::SeqCst), 3);
    assert!(kept(&flaky.store).await);
    test.advance_time_by(SECOND).await;
    assert_eq!(flaky.removes.load(Ordering::SeqCst), 4, "deleted at the fourth try");
    assert!(!kept(&flaky.store).await);
    let state: State = test.state(&job).await;
    assert_eq!((state.errors, state.outputs_until, state.armed), (0, None, None));
    assert!(test.failures().await.is_empty());
}

/// Scenario: A store that refuses the name.
#[tokio::test(start_paused = true)]
async fn a_store_that_refuses_the_name_clears_the_due_time_without_retrying() {
    let fake = FakeRuntime::new();
    let flaky = Flaky::failing(&[OutputErrorKind::Invalid]);
    let (test, job) = removed(&fake, flaky.clone(), retained(DAY)).await;
    test.advance_time_by(30 * DAY).await;
    assert_eq!(flaky.removes.load(Ordering::SeqCst), 1, "not retried");
    let state: State = test.state(&job).await;
    assert_eq!((state.outputs_until, state.armed, state.errors), (None, None, 0));
}

/// Scenario: A lost retention poll. `Check` before the due time arms a fresh poll due then, not sooner; after it, at
/// once.
#[tokio::test(start_paused = true)]
async fn a_lost_retention_poll_comes_back_on_check_and_deletes_at_the_saved_time() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let (test, job) = removed(&fake, Arc::new(store.clone()), retained(7 * DAY)).await;
    let until = test.state::<_, State>(&job).await.outputs_until;
    let address = job.address().unwrap();
    assert_eq!(test.system().purge_mailbox(&address).await.unwrap(), Some(1));
    test.send(&job, ContainerMsg::Check).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, State>(&job).await.due, until, "due at the saved time");
    test.advance_time_by(7 * DAY - SECOND).await;
    assert!(kept(&store).await);
    test.advance_time_by(SECOND).await;
    assert!(!kept(&store).await);

    // Lost again and found only after the due time: the Check deletes at once.
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let (test, job) = removed(&fake, Arc::new(store.clone()), retained(DAY)).await;
    test.system().purge_mailbox(&job.address().unwrap()).await.unwrap();
    test.advance_time_by(2 * DAY).await;
    assert!(kept(&store).await, "the poll is lost");
    test.send(&job, ContainerMsg::Check).await;
    test.run_until_idle().await;
    assert!(!kept(&store).await);
    assert_eq!(test.state::<_, State>(&job).await.outputs_until, None);
    test.send(&job, ContainerMsg::Check).await;
    assert_eq!(test.run_until_idle().await, 1, "no work left: the Check arms nothing");
}

/// A restart between the removal and the due time: a new node with the state as saved, whose retention is unset by
/// now, still deletes at the saved time. And a node without a store at the due time clears it.
#[tokio::test(start_paused = true)]
async fn a_restart_with_the_retention_unset_still_deletes_at_the_saved_time() {
    let (fake, store) = (FakeRuntime::new(), MemoryOutputStore::new());
    let (test, job) = removed(&fake, Arc::new(store.clone()), retained(7 * DAY)).await;
    let mut saved = test.state_json(&job).await.unwrap();
    let until = test.state::<_, State>(&job).await.outputs_until;
    // The poll is not carried over to the new node.
    saved["armed"] = json!(null);
    saved["due"] = json!(null);

    let at = test.now();
    // Without a store first: the outputs stay for the node with one.
    for with_store in [false, true] {
        let restart = |store: Option<Arc<dyn OutputStore>>| {
            let fake = fake.clone();
            TestActors::builder().start_at(at).with(move |node| {
                let actor = ContainerActor::new(Job::out(), Arc::new(fake), settings());
                let actor = match store {
                    Some(store) => actor.with_store(store),
                    None => actor,
                };
                node.host(actor).host(WatcherActor)
            })
        };
        let node = restart(with_store.then(|| Arc::new(store.clone()) as Arc<dyn OutputStore>)).build().await.unwrap();
        node.create(&job, &saved).await;
        node.send(&job, ContainerMsg::Check).await;
        node.run_until_idle().await;
        assert_eq!(node.state::<_, State>(&job).await.due, until);
        node.advance_time_by(7 * DAY).await;
        assert_eq!(node.state::<_, State>(&job).await.outputs_until, None);
        assert_eq!(kept(&store).await, !with_store, "with a store: deleted; without: kept and cleared");
    }
}
