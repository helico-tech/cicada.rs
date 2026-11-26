//! The warnings of the container actor when its output store keeps failing, in a test binary of their own: tracing
//! caches whether a log line is wanted per call site, so another test that logs the same line without a subscriber at
//! the same time could hide it here.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use cicada::prelude::*;
use cicada_containers::{
    Container, ContainerActor, ContainerJob, ContainerMsg, ContainerSettings, ContainerSpec, ContainerState,
    ContainerStatus, CopyFile, FakeRuntime, LogCursor, LogEnd, LogPage, MemoryOutputStore, OutputError,
    OutputErrorKind, OutputStore, Outputs, Phase, StoredFile, StoredLine,
};
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

const NAME: &str = "app-j1";

/// The request of a test container.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Run {}

/// The test job, with outputs at `/work/out`.
struct Job;

impl ContainerJob for Job {
    const TYPE: &'static str = "test.warnings";
    type Request = Run;

    fn spec(&self, name: &str, _: &Run) -> Result<ContainerSpec, String> {
        Ok(ContainerSpec::new(name, "alpine:3"))
    }

    fn outputs(&self, _: &Run) -> Option<Outputs> {
        Some(Outputs::new("/work/out"))
    }
}

/// The target of the ends, which keeps nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Watcher;

impl ActorRef for Watcher {
    const TYPE: &'static str = "test.watcher";
    type Message = Ignored;
}

/// Any message of the target.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Ignored {
    /// A container's end.
    Ended(serde_json::Value),
}

struct WatcherActor;

impl Actor for WatcherActor {
    type Ref = Watcher;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: Ignored) -> HandlerResult {
        Ok(())
    }
}

/// The memory store, whose `put_file` and `remove` always fail with `Unavailable`.
#[derive(Default)]
struct Failing(MemoryOutputStore);

fn unavailable() -> OutputError {
    OutputError::new(OutputErrorKind::Unavailable, "the store is down")
}

#[async_trait]
impl OutputStore for Failing {
    async fn log_end(&self, container: &str) -> Result<LogEnd, OutputError> {
        self.0.log_end(container).await
    }
    async fn append_logs(
        &self,
        container: &str,
        after: Option<&LogCursor>,
        page: &LogPage,
    ) -> Result<u64, OutputError> {
        self.0.append_logs(container, after, page).await
    }
    async fn read_logs(&self, container: &str, from: u64, limit: u32) -> Result<Vec<StoredLine>, OutputError> {
        self.0.read_logs(container, from, limit).await
    }
    async fn put_file(&self, _: &str, _: &CopyFile) -> Result<(), OutputError> {
        Err(unavailable())
    }
    async fn files(&self, container: &str) -> Result<Vec<StoredFile>, OutputError> {
        self.0.files(container).await
    }
    async fn read_file(&self, container: &str, path: &str) -> Result<Option<CopyFile>, OutputError> {
        self.0.read_file(container, path).await
    }
    async fn remove(&self, _: &str) -> Result<(), OutputError> {
        Err(unavailable())
    }
}

/// What the subscriber wrote.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Log {
    /// The warning lines that hold `text`.
    fn warnings(&self, text: &str) -> usize {
        let log = String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned();
        log.lines().filter(|line| line.contains("WARN") && line.contains(text)).count()
    }
}

/// Scenario: a store that keeps failing. While collecting, the errors in a row reach `max_errors` and the end goes out
/// with one warning; while deleting, the deletion keeps trying past `max_errors` with one warning.
#[tokio::test(start_paused = true)]
async fn a_store_that_keeps_failing_logs_one_warning() {
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || writer.clone()).finish();
    let _logging = tracing::subscriber::set_default(subscriber);

    let fake = FakeRuntime::new();
    let mut settings = ContainerSettings::new("app-").unwrap();
    settings.max_errors = 3;
    settings.output_retention = Some(Duration::ZERO);
    let (runtime, store) = (Arc::new(fake.clone()), Arc::new(Failing::default()));
    let test = TestActors::builder()
        .with(move |node| node.host(ContainerActor::new(Job, runtime, settings).with_store(store)).host(WatcherActor))
        .build()
        .await
        .unwrap();
    test.create(&Watcher, &NoState::default()).await;
    let job = Container::<Job>::named("j1");
    test.create(&job, &ContainerState::new(Run {})).await;
    test.send(&job, ContainerMsg::start(&Watcher, "Ended").unwrap()).await;
    test.run_until_idle().await;
    fake.put_file(NAME, CopyFile::new("/work/out/a", "x"));
    fake.exit_with(NAME, ContainerStatus::Exited { code: 0 });
    test.advance_time_by(Duration::from_secs(600)).await;
    let state: ContainerState<Run> = test.state(&job).await;
    let error = state.end.and_then(|end| end.outputs).and_then(|outputs| outputs.error);
    assert!(error.as_deref().is_some_and(|e| e.starts_with("(no outputs: failed 3 times in a row: ")), "{error:?}");
    assert_eq!(log.warnings("the outputs were not collected; the end goes out without them"), 1, "after 3 errors");

    test.send(&job, ContainerMsg::Remove).await;
    test.advance_time_by(Duration::from_secs(3600)).await;
    let state: ContainerState<Run> = test.state(&job).await;
    assert_eq!(state.phase, Phase::Removed);
    assert!(state.errors > 3 && state.outputs_until.is_some(), "it keeps trying: {} errors", state.errors);
    assert_eq!(log.warnings("the stored outputs were not deleted; the deletion keeps trying"), 1, "once, at 3");
}
