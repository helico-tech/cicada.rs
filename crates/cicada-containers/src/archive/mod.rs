//! The [`LogArchiver`]: follows the logs of a job's containers and keeps them in an [`OutputStore`], outside actor
//! turns.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::{Id, JoinSet};
use tokio::time::{Instant, sleep_until};

use crate::{
    ContainerError, ContainerRuntime, ContainerStatus, ErrorKind, LOG_PAGE_MAX, Label, LogCursor, OutputError,
    OutputErrorKind, OutputStore, PREFIX_LABEL,
};

/// Failed reads of a container, or failed lists, in a row before the wait between tries grows.
const FAILURES_BEFORE_BACKOFF: u32 = 5;
/// The longest wait between tries after failures.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// The settings of a [`LogArchiver`], checked by [`LogArchiver::new`].
///
/// ```
/// use std::time::Duration;
/// use cicada_containers::ArchiveSettings;
///
/// let mut settings = ArchiveSettings::new();
/// assert_eq!((settings.interval, settings.page_lines, settings.max_parallel), (Duration::from_secs(2), 500, 16));
/// settings.interval = Duration::from_secs(5);
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSettings {
    /// The time between rounds, and the base of the wait after failures; 2 s. Keep it positive: each round makes
    /// Docker scan every followed log from its cursor's second.
    pub interval: Duration,
    /// The `max_lines` of each [`logs_since`](ContainerRuntime::logs_since) call; 500, and 1 to [`LOG_PAGE_MAX`].
    pub page_lines: u32,
    /// The most log reads at once; 16. Keep it positive.
    pub max_parallel: usize,
}

impl ArchiveSettings {
    /// The defaults.
    pub fn new() -> Self {
        Self { interval: Duration::from_secs(2), page_lines: 500, max_parallel: 16 }
    }

    /// The wait before the next try after `failures` failures in a row: the interval, and from the fifth on the
    /// interval times `2^(failures − 5)`, at most 60 s.
    fn backoff(&self, failures: u32) -> Duration {
        let Some(over) = failures.checked_sub(FAILURES_BEFORE_BACKOFF) else { return self.interval };
        1u32.checked_shl(over)
            .and_then(|factor| self.interval.checked_mul(factor))
            .map_or(MAX_BACKOFF, |wait| wait.min(MAX_BACKOFF))
    }
}

impl Default for ArchiveSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// [`ArchiveSettings`] out of their bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidArchive(String);

/// Follows the log of every container of a container job and keeps it in an [`OutputStore`], so it survives the
/// container's removal. It runs outside actor turns, as a page is too big for actor state: a host runs
/// [`run`](Self::run) as an app task or spawns it itself.
///
/// **Which containers.** The containers [`list`](ContainerRuntime::list) gives for the label [`PREFIX_LABEL`] with the
/// job's prefix, which the container actor puts on every container it creates. Only existing containers are followed,
/// so nothing is written for a removed one.
///
/// **Rounds that never wait.** Each [`interval`](ArchiveSettings::interval), a round lists the containers and starts a
/// read for each one that is neither drained nor still being read, up to
/// [`max_parallel`](ArchiveSettings::max_parallel) reads at once, without waiting for them: a slow log holds up only
/// itself. A read takes the stored cursor from [`log_end`](OutputStore::log_end) and appends each page of
/// [`logs_since`](ContainerRuntime::logs_since) with the guarded [`append_logs`](OutputStore::append_logs), up to a
/// short page. A running container is read again in later rounds; an ended one is drained once and then skipped. So
/// every line is stored once and in order, also after a restart and with two archivers on one store, as the second
/// writer of a page fails with `Conflict` and reads the cursor again. Each archiver repeats every log scan of its
/// daemon: run one per Docker daemon.
///
/// **Errors.** Nothing ends `run` but `stop`. A failed `list` is tried again next round. A container whose log is
/// `Unsupported` is drained, and logged once; one that is gone (`Conflict`) is dropped; every other error, of the
/// runtime or the store, is tried again in a later round, with a wait that doubles from the fifth failure in a row,
/// at most 60 s. A store's `Conflict` reads the cursor again next round, and its `Invalid` skips the container.
///
/// **Limits.** A container made before the label existed is not followed. Lines a container writes after the last
/// read, when it is removed before the next round, are not stored.
///
/// ```
/// use std::sync::Arc;
/// use std::time::Duration;
/// use cicada_containers::{
///     ArchiveSettings, ContainerRuntime, ContainerSpec, FakeRuntime, LogArchiver, MemoryOutputStore, OutputStore,
///     PREFIX_LABEL,
/// };
///
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let runtime = FakeRuntime::new();
/// let mut spec = ContainerSpec::new("ci-1", "alpine:3");
/// spec.labels.insert(PREFIX_LABEL.into(), "ci-".into()); // as the container actor does
/// runtime.create(&spec).await?;
/// runtime.set_logs("ci-1", "hello\nworld");
///
/// let store = MemoryOutputStore::new();
/// let archiver = LogArchiver::new(Arc::new(runtime), Arc::new(store.clone()), "ci-", ArchiveSettings::new())?;
/// archiver.run(tokio::time::sleep(Duration::from_secs(1))).await;
/// assert_eq!(store.read_logs("ci-1", 0, 10).await?[1].line.text, "world");
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct LogArchiver {
    runtime: Arc<dyn ContainerRuntime>,
    store: Arc<dyn OutputStore>,
    label: Label,
    settings: ArchiveSettings,
}

impl LogArchiver {
    /// An archiver of the containers of `runtime` whose names start with `prefix`, the container actor's prefix, into
    /// `store`. Fails when [`page_lines`](ArchiveSettings::page_lines) is not 1 to [`LOG_PAGE_MAX`].
    pub fn new(
        runtime: Arc<dyn ContainerRuntime>,
        store: Arc<dyn OutputStore>,
        prefix: impl Into<String>,
        settings: ArchiveSettings,
    ) -> Result<Self, InvalidArchive> {
        if !(1..=LOG_PAGE_MAX).contains(&settings.page_lines) {
            return Err(InvalidArchive(format!("page_lines is {}, not 1 to {LOG_PAGE_MAX}", settings.page_lines)));
        }
        Ok(Self { runtime, store, label: Label::new(PREFIX_LABEL, prefix.into()), settings })
    }

    /// Archives in rounds until `stop` completes, then returns; the reads still going are dropped, which loses
    /// nothing, as each page is stored with its cursor in one write. No error ends it.
    pub async fn run(self, stop: impl Future<Output = ()>) {
        let mut stop = pin!(stop);
        let mut rounds = Rounds::default();
        let mut reads = JoinSet::new();
        let mut next_round = Instant::now();
        loop {
            tokio::select! {
                () = &mut stop => return,
                Some(done) = reads.join_next_with_id() => rounds.finish(&self.settings, done),
                () = sleep_until(next_round) => {
                    tokio::select! {
                        () = &mut stop => return,
                        wait = self.round(&mut rounds, &mut reads) => next_round = Instant::now() + wait,
                    }
                }
            }
        }
    }

    /// Lists the containers and starts the reads that are due; gives the wait before the next round.
    async fn round(&self, rounds: &mut Rounds, reads: &mut JoinSet<Outcome>) -> Duration {
        let listed = match self.runtime.list(&self.label).await {
            Ok(listed) => listed,
            Err(e) => {
                rounds.list_failures += 1;
                tracing::warn!(label = %self.label.value, failures = rounds.list_failures, "container logs: list failed: {e}");
                return self.settings.backoff(rounds.list_failures);
            }
        };
        rounds.list_failures = 0;
        let names: HashSet<&str> = listed.iter().map(|c| c.name.as_str()).collect();
        rounds.done.retain(|name| names.contains(name.as_str()));
        rounds.failures.retain(|name, _| names.contains(name.as_str()));
        // From after the last container started, so that more containers than `max_parallel` take turns.
        let split = rounds.last.as_ref().map_or(0, |last| listed.partition_point(|c| c.name <= *last));
        let now = Instant::now();
        for container in listed[split..].iter().chain(&listed[..split]) {
            if reads.len() >= self.settings.max_parallel {
                break;
            }
            let name = &container.name;
            let waiting = rounds.failures.get(name).is_some_and(|failure| failure.not_before > now);
            if rounds.done.contains(name) || rounds.reading.values().any(|r| r == name) || waiting {
                continue;
            }
            let ended = matches!(container.status, ContainerStatus::Exited { .. } | ContainerStatus::Failed { .. });
            let read = read(self.runtime.clone(), self.store.clone(), name.clone(), self.settings.page_lines);
            let id = reads.spawn(async move { read.await.map(|()| ended) }).id();
            rounds.reading.insert(id, name.clone());
            rounds.last = Some(name.clone());
        }
        self.settings.interval
    }
}

/// What a read gives: `Ok(ended)` once it read to a short page, with whether the container had ended before it.
type Outcome = Result<bool, ReadError>;

/// Why a read stopped early.
enum ReadError {
    Runtime(ContainerError),
    Store(OutputError),
}

/// The archiver's memory between rounds.
#[derive(Default)]
struct Rounds {
    /// The containers being read, by the id of their read.
    reading: HashMap<Id, String>,
    /// The containers not to read again: drained after their end, with an `Unsupported` log, or `Invalid` for the
    /// store. Pruned to the listed names each round.
    done: HashSet<String>,
    /// Failed reads in a row, by container. Pruned to the listed names each round.
    failures: HashMap<String, Failures>,
    /// Failed lists in a row.
    list_failures: u32,
    /// The last container a round started a read for.
    last: Option<String>,
}

/// Failed reads in a row of one container, and when it may be read again.
struct Failures {
    count: u32,
    not_before: Instant,
}

impl Rounds {
    /// Takes in the outcome of a finished read.
    fn finish(&mut self, settings: &ArchiveSettings, done: Result<(Id, Outcome), tokio::task::JoinError>) {
        let (id, outcome) = match done {
            Ok((id, outcome)) => (id, outcome),
            Err(e) => {
                let id = e.id();
                let Some(name) = self.reading.remove(&id) else { return };
                tracing::warn!(container = %name, "container logs: the read panicked: {e}");
                self.failed(settings, name);
                return;
            }
        };
        let Some(name) = self.reading.remove(&id) else { return };
        match outcome {
            Ok(ended) => {
                self.failures.remove(&name);
                if ended {
                    self.done.insert(name);
                }
            }
            Err(ReadError::Runtime(e)) => match e.kind {
                ErrorKind::Unsupported => {
                    tracing::warn!(container = %name, "container logs: not archived, the log cannot be read: {e}");
                    self.failures.remove(&name);
                    self.done.insert(name);
                }
                // Gone: the next list no longer has it.
                ErrorKind::Conflict => {
                    self.failures.remove(&name);
                }
                _ => {
                    tracing::warn!(container = %name, "container logs: the read failed: {e}");
                    self.failed(settings, name);
                }
            },
            Err(ReadError::Store(e)) => match e.kind {
                // Another writer moved the cursor: the next round reads it again.
                OutputErrorKind::Conflict => {}
                OutputErrorKind::Invalid => {
                    tracing::warn!(container = %name, "container logs: not archived, the store refuses it: {e}");
                    self.failures.remove(&name);
                    self.done.insert(name);
                }
                _ => {
                    tracing::warn!(container = %name, "container logs: the store failed: {e}");
                    self.failed(settings, name);
                }
            },
        }
    }

    /// Counts a failed read of `name`, and from the fifth in a row sets when it may be read again.
    fn failed(&mut self, settings: &ArchiveSettings, name: String) {
        let failures = self.failures.entry(name).or_insert(Failures { count: 0, not_before: Instant::now() });
        failures.count += 1;
        if failures.count >= FAILURES_BEFORE_BACKOFF {
            failures.not_before = Instant::now() + settings.backoff(failures.count);
        }
    }
}

/// Reads the log of `name` from the stored cursor to a short page, appending each page that holds lines.
async fn read(
    runtime: Arc<dyn ContainerRuntime>,
    store: Arc<dyn OutputStore>,
    name: String,
    page_lines: u32,
) -> Result<(), ReadError> {
    let mut cursor: Option<LogCursor> = store.log_end(&name).await.map_err(ReadError::Store)?.cursor;
    loop {
        let page = runtime.logs_since(&name, cursor, page_lines).await.map_err(ReadError::Runtime)?;
        // An empty page leaves the cursor where it was: nothing to write.
        if !page.lines.is_empty() {
            store.append_logs(&name, cursor.as_ref(), &page).await.map_err(ReadError::Store)?;
        }
        cursor = Some(page.next);
        if page.lines.len() < page_lines as usize {
            return Ok(());
        }
    }
}
