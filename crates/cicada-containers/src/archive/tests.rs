//! The archiver on virtual time, with the fake runtime and the memory store; a runtime whose read of one container
//! hangs and a store that fails as scripted wrap them. Rounds run at 0 s, 2 s, 4 s and so on; tests look at odd
//! seconds.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio::sync::{Semaphore, oneshot};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use super::*;
use crate::{
    Call, ContainerSpec, ContainerSummary, CopyFile, FakeRuntime, LogEnd, LogLine, LogPage, LogStream,
    MemoryOutputStore, Method, NetworkSpec, StoredLine, Swept, VolumeSpec,
};

const SECOND: Duration = Duration::from_secs(1);

/// Creates and starts the container `name` with the archiver's label.
async fn labelled(fake: &FakeRuntime, name: &str) {
    let mut spec = ContainerSpec::new(name, "alpine:3");
    spec.labels.insert(PREFIX_LABEL.into(), "ci-".into());
    fake.create(&spec).await.unwrap();
    fake.start(name).await.unwrap();
}

/// The out line `text`, stamped at the Unix second `second`.
fn line(second: i64, text: &str) -> LogLine {
    LogLine::new(cicada::time::from_micros(second * 1_000_000), LogStream::Out, text)
}

/// Adds lines `texts` to the log of `name`, stamped at `second`.
fn push(fake: &FakeRuntime, name: &str, second: i64, texts: &[&str]) {
    for text in texts {
        fake.push_log(name, line(second, text));
    }
}

/// The texts of the stored lines of `name`.
async fn stored(store: &dyn OutputStore, name: &str) -> Vec<String> {
    let lines: Vec<StoredLine> = store.read_logs(name, 0, 10_000).await.unwrap();
    lines.into_iter().map(|stored| stored.line.text).collect()
}

/// How many `logs_since` calls of `name` the fake received.
fn reads(fake: &FakeRuntime, name: &str) -> usize {
    fake.calls().iter().filter(|call| matches!(call, Call::LogsSince { name: n, .. } if n == name)).count()
}

/// How many `list` calls the fake received.
fn lists(fake: &FakeRuntime) -> usize {
    fake.calls().iter().filter(|call| matches!(call, Call::List(_))).count()
}

/// A running archiver of the prefix `ci-`: stopped by the sender, ended when the handle is.
struct Running {
    stop: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

impl Running {
    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.handle.await.unwrap();
    }
}

fn start_on(runtime: Arc<dyn ContainerRuntime>, store: Arc<dyn OutputStore>, settings: ArchiveSettings) -> Running {
    let archiver = LogArchiver::new(runtime, store, "ci-", settings).unwrap();
    let (stop, stopped) = oneshot::channel();
    let handle = tokio::spawn(archiver.run(async move {
        let _ = stopped.await;
    }));
    Running { stop, handle }
}

fn start(fake: &FakeRuntime, store: &MemoryOutputStore) -> Running {
    start_on(Arc::new(fake.clone()), Arc::new(store.clone()), ArchiveSettings::new())
}

/// Scenario: A running container. Pages of two lines, lines that share a second, over several rounds.
#[tokio::test(start_paused = true)]
async fn a_running_containers_lines_arrive_across_rounds_once_and_in_order() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-1").await;
    push(&fake, "ci-1", 100, &["a", "b", "c", "d", "e"]);
    let mut settings = ArchiveSettings::new();
    settings.page_lines = 2;
    let running = start_on(Arc::new(fake.clone()), Arc::new(store.clone()), settings);

    sleep(SECOND).await;
    assert_eq!(stored(&store, "ci-1").await, ["a", "b", "c", "d", "e"]);
    assert_eq!(reads(&fake, "ci-1"), 3, "pages of 2, 2 and 1");
    push(&fake, "ci-1", 100, &["f"]);
    push(&fake, "ci-1", 101, &["g", "h"]);
    sleep(2 * SECOND).await;
    push(&fake, "ci-1", 101, &["i"]);
    sleep(2 * SECOND).await;
    sleep(2 * SECOND).await; // a round with nothing new writes nothing

    assert_eq!(stored(&store, "ci-1").await, ["a", "b", "c", "d", "e", "f", "g", "h", "i"]);
    let all = fake.logs_since("ci-1", None, LOG_PAGE_MAX).await.unwrap();
    assert_eq!(store.log_end("ci-1").await.unwrap(), LogEnd::new(Some(all.next), 9));
    running.stop().await;
}

/// An ended container is drained once and then skipped; a container without the label is not followed.
#[tokio::test(start_paused = true)]
async fn an_ended_container_is_drained_once_and_then_skipped() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-1").await;
    fake.create(&ContainerSpec::new("ci-old", "alpine:3")).await.unwrap(); // made before the label
    fake.set_logs("ci-old", "old");
    let running = start(&fake, &store);

    sleep(SECOND).await;
    push(&fake, "ci-1", 100, &["last"]);
    fake.exit_with("ci-1", ContainerStatus::Exited { code: 0 });
    sleep(2 * SECOND).await;
    assert_eq!(stored(&store, "ci-1").await, ["last"]);
    let drained = reads(&fake, "ci-1");
    push(&fake, "ci-1", 100, &["never"]); // the fake allows it; Docker does not
    sleep(10 * SECOND).await;

    assert_eq!(reads(&fake, "ci-1"), drained, "skipped after the drain");
    assert_eq!(stored(&store, "ci-1").await, ["last"]);
    assert_eq!(reads(&fake, "ci-old"), 0);
    running.stop().await;
}

/// Scenario: A restart. The new archiver reads on from the stored cursor, and drains an ended container again, which
/// finds nothing new.
#[tokio::test(start_paused = true)]
async fn a_restart_stores_only_the_lines_after_the_stored_cursor() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-run").await;
    labelled(&fake, "ci-end").await;
    push(&fake, "ci-run", 100, &["a", "b"]);
    push(&fake, "ci-end", 100, &["x"]);
    fake.exit_with("ci-end", ContainerStatus::Exited { code: 0 });
    let first = start(&fake, &store);
    sleep(SECOND).await;
    first.stop().await;

    push(&fake, "ci-run", 100, &["c"]);
    let second = start(&fake, &store);
    sleep(SECOND).await;

    assert_eq!(stored(&store, "ci-run").await, ["a", "b", "c"]);
    assert_eq!(stored(&store, "ci-end").await, ["x"]);
    assert_eq!(reads(&fake, "ci-end"), 2, "drained once by each");
    second.stop().await;
}

/// Scenario: A container removed between rounds.
#[tokio::test(start_paused = true)]
async fn a_container_removed_between_rounds_keeps_the_lines_up_to_the_last_read() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-1").await;
    push(&fake, "ci-1", 100, &["a"]);
    let running = start(&fake, &store);

    sleep(SECOND).await;
    push(&fake, "ci-1", 100, &["b"]);
    fake.exit_with("ci-1", ContainerStatus::Exited { code: 0 });
    fake.remove("ci-1").await.unwrap();
    sleep(4 * SECOND).await;

    assert_eq!(stored(&store, "ci-1").await, ["a"]);
    assert_eq!(reads(&fake, "ci-1"), 1, "not read once it is gone");
    running.stop().await;
}

/// A runtime whose `logs_since` of one container waits for a permit of `gate`; every other call goes to the fake.
struct Hanging {
    fake: FakeRuntime,
    name: String,
    gate: Arc<Semaphore>,
    /// Reads of `name` started, and at once now and at most.
    started: AtomicUsize,
    now: AtomicUsize,
    most: AtomicUsize,
}

#[async_trait]
impl ContainerRuntime for Hanging {
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
        if name == self.name {
            self.started.fetch_add(1, Ordering::SeqCst);
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
            self.gate.acquire().await.unwrap().forget();
            self.now.fetch_sub(1, Ordering::SeqCst);
        }
        self.fake.logs_since(name, cursor, max_lines).await
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

/// Scenario: A slow log. Then `stop` ends `run` with the read still hanging.
#[tokio::test(start_paused = true)]
async fn a_hanging_read_holds_up_only_itself_and_is_not_read_twice_at_once() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-a").await;
    labelled(&fake, "ci-b").await;
    let gate = Arc::new(Semaphore::new(0));
    let hanging = Arc::new(Hanging {
        fake: fake.clone(),
        name: "ci-a".into(),
        gate: gate.clone(),
        started: AtomicUsize::new(0),
        now: AtomicUsize::new(0),
        most: AtomicUsize::new(0),
    });
    push(&fake, "ci-a", 100, &["a1"]);
    let running = start_on(hanging.clone(), Arc::new(store.clone()), ArchiveSettings::new());

    for (round, text) in ["b1", "b2", "b3"].into_iter().enumerate() {
        push(&fake, "ci-b", 100, &[text]);
        sleep(if round == 0 { SECOND } else { 2 * SECOND }).await;
    }
    assert_eq!(stored(&store, "ci-b").await, ["b1", "b2", "b3"], "a line each round");
    assert!(stored(&store, "ci-a").await.is_empty());
    assert_eq!(hanging.started.load(Ordering::SeqCst), 1, "not read again while its read hangs");

    gate.add_permits(1);
    sleep(SECOND / 2).await;
    assert_eq!(stored(&store, "ci-a").await, ["a1"]);
    push(&fake, "ci-a", 100, &["a2"]);
    sleep(SECOND).await; // the round at 6 s reads it again, and hangs
    assert_eq!(hanging.started.load(Ordering::SeqCst), 2);
    assert_eq!(hanging.most.load(Ordering::SeqCst), 1, "never two reads at once");

    running.stop().await;
    gate.add_permits(10);
    sleep(SECOND).await;
    assert_eq!(hanging.now.load(Ordering::SeqCst), 1, "the hanging read is dropped, not finished");
    assert_eq!(stored(&store, "ci-a").await, ["a1"]);
}

/// Two archivers on one store, whose calls yield so that their reads interleave.
#[tokio::test(start_paused = true)]
async fn two_archivers_on_one_store_store_every_line_once() {
    let fake = FakeRuntime::new();
    let memory = MemoryOutputStore::new();
    let store = Arc::new(Scripted::new(&memory));
    labelled(&fake, "ci-1").await;
    labelled(&fake, "ci-2").await;
    let mut settings = ArchiveSettings::new();
    settings.page_lines = 2;
    let one = start_on(Arc::new(fake.clone()), store.clone(), settings.clone());
    let two = start_on(Arc::new(fake.clone()), store.clone(), settings);

    let mut expected = Vec::new();
    for second in 0..6 {
        let texts: Vec<String> = (0..3).map(|i| format!("{second}.{i}")).collect();
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        push(&fake, "ci-1", 100 + second, &texts);
        push(&fake, "ci-2", 100 + second, &texts);
        expected.extend(texts.iter().map(|t| t.to_string()));
        sleep(SECOND).await;
    }
    sleep(4 * SECOND).await;

    assert_eq!(stored(&memory, "ci-1").await, expected);
    assert_eq!(stored(&memory, "ci-2").await, expected);
    assert!(store.conflicts.load(Ordering::SeqCst) > 0, "the archivers raced");
    one.stop().await;
    two.stop().await;
}

#[test]
fn page_lines_outside_1_to_the_page_maximum_are_refused() {
    let new = |page_lines| {
        let mut settings = ArchiveSettings::new();
        settings.page_lines = page_lines;
        LogArchiver::new(Arc::new(FakeRuntime::new()), Arc::new(MemoryOutputStore::new()), "ci-", settings)
    };
    for refused in [0, LOG_PAGE_MAX + 1] {
        let error = new(refused).err().unwrap();
        assert_eq!(error.to_string(), format!("page_lines is {refused}, not 1 to 1000"));
    }
    assert!(new(1).is_ok() && new(LOG_PAGE_MAX).is_ok());
}

#[test]
fn a_zero_interval_and_no_parallel_reads_are_refused() {
    let new =
        |settings| LogArchiver::new(Arc::new(FakeRuntime::new()), Arc::new(MemoryOutputStore::new()), "ci-", settings);
    let zero_interval = ArchiveSettings { interval: Duration::ZERO, ..ArchiveSettings::new() };
    assert_eq!(new(zero_interval).err().unwrap().to_string(), "interval is zero");
    let no_reads = ArchiveSettings { max_parallel: 0, ..ArchiveSettings::new() };
    assert_eq!(new(no_reads).err().unwrap().to_string(), "max_parallel is 0");
}

#[test]
fn the_backoff_doubles_from_the_fifth_failure_up_to_a_minute() {
    let settings = ArchiveSettings::new();
    let waits: Vec<u64> = (0..12).map(|n| settings.backoff(n).as_secs()).collect();
    assert_eq!(waits, [2, 2, 2, 2, 2, 2, 4, 8, 16, 32, 60, 60]);
    assert_eq!(settings.backoff(u32::MAX), MAX_BACKOFF);
}

/// An `OutputStore` over a memory store whose calls yield first, so that writers interleave, and fail as scripted.
struct Scripted {
    memory: MemoryOutputStore,
    /// The next failures of `log_end` (false) and `append_logs` (true) of a container, in order.
    fail_next: Mutex<Vec<(String, bool, OutputErrorKind)>>,
    log_ends: AtomicUsize,
    conflicts: AtomicUsize,
}

impl Scripted {
    fn new(memory: &MemoryOutputStore) -> Self {
        Self {
            memory: memory.clone(),
            fail_next: Mutex::new(Vec::new()),
            log_ends: AtomicUsize::new(0),
            conflicts: AtomicUsize::new(0),
        }
    }

    fn fail_next(&self, container: &str, append: bool, kind: OutputErrorKind) {
        self.fail_next.lock().unwrap().push((container.into(), append, kind));
    }

    async fn enter(&self, container: &str, append: bool) -> Result<(), OutputError> {
        tokio::task::yield_now().await;
        let mut failures = self.fail_next.lock().unwrap();
        match failures.iter().position(|(c, a, _)| c == container && *a == append) {
            Some(at) => Err(OutputError::new(failures.remove(at).2, "scripted")),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl OutputStore for Scripted {
    async fn log_end(&self, container: &str) -> Result<LogEnd, OutputError> {
        self.log_ends.fetch_add(1, Ordering::SeqCst);
        self.enter(container, false).await?;
        self.memory.log_end(container).await
    }
    async fn append_logs(
        &self,
        container: &str,
        after: Option<&LogCursor>,
        page: &LogPage,
    ) -> Result<u64, OutputError> {
        self.enter(container, true).await?;
        let appended = self.memory.append_logs(container, after, page).await;
        if appended.as_ref().is_err_and(|e| e.kind == OutputErrorKind::Conflict) {
            self.conflicts.fetch_add(1, Ordering::SeqCst);
        }
        appended
    }
    async fn read_logs(&self, container: &str, from: u64, limit: u32) -> Result<Vec<StoredLine>, OutputError> {
        self.memory.read_logs(container, from, limit).await
    }
    async fn put_file(&self, container: &str, file: &CopyFile) -> Result<(), OutputError> {
        self.memory.put_file(container, file).await
    }
    async fn files(&self, container: &str) -> Result<Vec<crate::StoredFile>, OutputError> {
        self.memory.files(container).await
    }
    async fn read_file(&self, container: &str, path: &str) -> Result<Option<CopyFile>, OutputError> {
        self.memory.read_file(container, path).await
    }
    async fn remove(&self, container: &str) -> Result<(), OutputError> {
        self.memory.remove(container).await
    }
}

/// Scenario: A daemon that comes back. And an `InvalidSpec`, retried the same way.
#[tokio::test(start_paused = true)]
async fn a_failed_read_is_tried_again_next_round() {
    for (kind, failures) in [(ErrorKind::DaemonUnavailable, 2), (ErrorKind::InvalidSpec, 1), (ErrorKind::Other, 1)] {
        let fake = FakeRuntime::new();
        let store = MemoryOutputStore::new();
        labelled(&fake, "ci-1").await;
        push(&fake, "ci-1", 100, &["a"]);
        (0..failures).for_each(|_| fake.fail_next(Method::LogsSince, kind));
        let running = start(&fake, &store);

        sleep(SECOND + 2 * SECOND * (failures as u32 - 1)).await;
        assert!(stored(&store, "ci-1").await.is_empty(), "{kind:?}");
        sleep(2 * SECOND).await;
        assert_eq!(stored(&store, "ci-1").await, ["a"], "{kind:?}");
        running.stop().await;
    }
}

/// Five failed reads in a row, then a wait that doubles: reads at 0, 2, 4, 6 and 8 s, then 10, 14, 22 and 38 s.
#[tokio::test(start_paused = true)]
async fn failed_reads_wait_longer_from_the_fifth_in_a_row() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-1").await;
    labelled(&fake, "ci-2").await;
    push(&fake, "ci-1", 100, &["a"]);
    fake.fail_always(Method::LogsSince, ErrorKind::DaemonUnavailable);
    let running = start(&fake, &store);

    sleep(23 * SECOND).await;
    assert_eq!(reads(&fake, "ci-1"), 8);
    assert_eq!(lists(&fake), 12, "the rounds go on");
    fake.clear_failures();
    sleep(14 * SECOND).await;
    assert!(stored(&store, "ci-1").await.is_empty(), "still waiting at 37 s");
    sleep(2 * SECOND).await;
    assert_eq!(stored(&store, "ci-1").await, ["a"]);
    assert_eq!(reads(&fake, "ci-1"), 9);
    running.stop().await;
}

/// Scenario: A failed list. Five in a row make the rounds wait longer, as for reads.
#[tokio::test(start_paused = true)]
async fn a_failed_list_is_tried_again_and_waits_longer_from_the_fifth() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-1").await;
    push(&fake, "ci-1", 100, &["a"]);
    fake.fail_next(Method::List, ErrorKind::DaemonUnavailable);
    let running = start(&fake, &store);

    sleep(SECOND).await;
    assert!(stored(&store, "ci-1").await.is_empty());
    sleep(2 * SECOND).await;
    assert_eq!(stored(&store, "ci-1").await, ["a"], "the next round lists again");

    fake.fail_always(Method::List, ErrorKind::Other);
    sleep(22 * SECOND).await; // lists at 4, 6, 8, 10 and 12 s, then 14 and 18 s, and the next at 26 s
    assert_eq!(lists(&fake), 2 + 7);
    fake.clear_failures();
    push(&fake, "ci-1", 100, &["b"]);
    sleep(2 * SECOND).await;
    assert_eq!(stored(&store, "ci-1").await, ["a", "b"]);
    assert_eq!(lists(&fake), 2 + 8);
    running.stop().await;
}

/// Scenario: A log that can never be read. Logged once and drained, while another container goes on.
#[tokio::test(start_paused = true)]
async fn an_unsupported_log_is_drained_and_not_read_again() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-1").await;
    labelled(&fake, "ci-2").await;
    push(&fake, "ci-1", 100, &["a"]);
    fake.fail_next(Method::LogsSince, ErrorKind::Unsupported); // ci-1, read first
    let running = start(&fake, &store);

    sleep(SECOND).await;
    push(&fake, "ci-2", 100, &["b"]);
    sleep(10 * SECOND).await;

    assert_eq!(reads(&fake, "ci-1"), 1);
    assert!(stored(&store, "ci-1").await.is_empty());
    assert_eq!(stored(&store, "ci-2").await, ["b"]);
    running.stop().await;
}

/// A container gone during its read (`Conflict`) is dropped without counting a failure: six in a row, and the read
/// of the seventh round, at 12 s, is not held back.
#[tokio::test(start_paused = true)]
async fn a_container_gone_during_its_read_is_dropped_without_a_wait() {
    let fake = FakeRuntime::new();
    let store = MemoryOutputStore::new();
    labelled(&fake, "ci-1").await;
    push(&fake, "ci-1", 100, &["a"]);
    (0..6).for_each(|_| fake.fail_next(Method::LogsSince, ErrorKind::Conflict));
    let running = start(&fake, &store);

    sleep(11 * SECOND).await;
    assert!(stored(&store, "ci-1").await.is_empty());
    sleep(2 * SECOND).await;
    assert_eq!(stored(&store, "ci-1").await, ["a"]);
    running.stop().await;
}

/// The store's `Unavailable` and `Other` are tried again next round, from `log_end` or `append_logs`; its `Conflict`
/// reads the cursor again next round.
#[tokio::test(start_paused = true)]
async fn a_failed_store_call_is_tried_again_next_round() {
    let cases = [
        (false, OutputErrorKind::Unavailable),
        (true, OutputErrorKind::Unavailable),
        (true, OutputErrorKind::Other),
        (true, OutputErrorKind::Conflict),
    ];
    for (append, kind) in cases {
        let fake = FakeRuntime::new();
        let memory = MemoryOutputStore::new();
        let store = Arc::new(Scripted::new(&memory));
        labelled(&fake, "ci-1").await;
        push(&fake, "ci-1", 100, &["a"]);
        store.fail_next("ci-1", append, kind);
        let running = start_on(Arc::new(fake.clone()), store.clone(), ArchiveSettings::new());

        sleep(SECOND).await;
        assert!(stored(&memory, "ci-1").await.is_empty(), "{append} {kind:?}");
        sleep(2 * SECOND).await;
        assert_eq!(stored(&memory, "ci-1").await, ["a"], "{append} {kind:?}");
        running.stop().await;
    }
}

/// The store's `Invalid` skips the container, logged once, while another container goes on.
#[tokio::test(start_paused = true)]
async fn a_container_the_store_refuses_is_skipped() {
    let fake = FakeRuntime::new();
    let memory = MemoryOutputStore::new();
    let store = Arc::new(Scripted::new(&memory));
    labelled(&fake, "ci-1").await;
    labelled(&fake, "ci-2").await;
    push(&fake, "ci-1", 100, &["a"]);
    store.fail_next("ci-1", false, OutputErrorKind::Invalid);
    let running = start_on(Arc::new(fake.clone()), store.clone(), ArchiveSettings::new());

    sleep(SECOND).await;
    push(&fake, "ci-2", 100, &["b"]);
    sleep(10 * SECOND).await;

    assert!(stored(&memory, "ci-1").await.is_empty());
    assert_eq!(reads(&fake, "ci-1"), 0);
    assert_eq!(store.log_ends.load(Ordering::SeqCst), 1 + 6, "ci-1 once, ci-2 each round");
    assert_eq!(stored(&memory, "ci-2").await, ["b"]);
    running.stop().await;
}
