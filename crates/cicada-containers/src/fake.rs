//! [`FakeRuntime`]: an in-memory [`ContainerRuntime`] for tests (feature `testing`).

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;

use crate::copy::{Collector, EntryKind};
use crate::logs::{check_max_lines, page};
use crate::paths::{MountTarget, Mounted, deepest_mount, path_parts, place};
use crate::{
    ContainerError, ContainerRuntime, ContainerSpec, ContainerStatus, ContainerSummary, CopyFile, CopyLimits, CopyOut,
    ErrorKind, Label, LogCursor, LogLine, LogPage, LogStream, MountKind, NetworkSpec, Networking, Resource, Skipped,
    Swept, VolumeSpec,
};

/// The exit code of a container the fake stops, as Docker reports one ended by SIGTERM.
const STOPPED_CODE: i64 = 143;

/// An in-memory [`ContainerRuntime`] that follows the trait's repeat rules. Clones share their state.
///
/// It keeps containers, networks and volumes by name with their labels and states, and answers at once: it uses no
/// timer, so it runs under paused time. A test scripts it: [`exit_with`](Self::exit_with) ends a running container,
/// [`set_logs`](Self::set_logs) and [`push_log`](Self::push_log) give it output, [`put_file`](Self::put_file) gives
/// it a file to copy out, [`fail_next`](Self::fail_next) and [`fail_always`](Self::fail_always) make calls of a
/// method fail without effect, and [`calls`](Self::calls) lists every call in order. A started
/// container runs until a test ends it or [`stop`](ContainerRuntime::stop) does (exit code 143).
#[derive(Clone, Debug, Default)]
pub struct FakeRuntime {
    state: Arc<Mutex<State>>,
}

#[derive(Debug, Default)]
struct State {
    containers: BTreeMap<String, FakeContainer>,
    networks: BTreeMap<String, NetworkSpec>,
    volumes: BTreeMap<String, VolumeSpec>,
    calls: Vec<Call>,
    fail_next: Vec<(Method, ErrorKind)>,
    fail_always: Vec<(Method, ErrorKind)>,
}

#[derive(Debug)]
struct FakeContainer {
    spec: ContainerSpec,
    status: ContainerStatus,
    /// Its log lines in log order, which `logs` and `logs_since` both read.
    logs: Vec<LogLine>,
    /// Its files by cleaned path, from `copy_in` and `put_file`.
    files: BTreeMap<String, CopyFile>,
}

/// A method of [`ContainerRuntime`], to script failures of the [`FakeRuntime`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// [`ContainerRuntime::create`].
    Create,
    /// [`ContainerRuntime::copy_in`].
    CopyIn,
    /// [`ContainerRuntime::start`].
    Start,
    /// [`ContainerRuntime::inspect`].
    Inspect,
    /// [`ContainerRuntime::logs`].
    Logs,
    /// [`ContainerRuntime::logs_since`].
    LogsSince,
    /// [`ContainerRuntime::copy_out`].
    CopyOut,
    /// [`ContainerRuntime::stop`].
    Stop,
    /// [`ContainerRuntime::remove`].
    Remove,
    /// [`ContainerRuntime::list`].
    List,
    /// [`ContainerRuntime::ensure_network`].
    EnsureNetwork,
    /// [`ContainerRuntime::ensure_volume`].
    EnsureVolume,
    /// [`ContainerRuntime::sweep`].
    Sweep,
}

/// A call the [`FakeRuntime`] received, with its arguments; failed calls too.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// [`ContainerRuntime::create`] with its spec.
    Create(Box<ContainerSpec>),
    /// [`ContainerRuntime::copy_in`] with its files.
    CopyIn {
        /// The container.
        name: String,
        /// The files.
        files: Vec<CopyFile>,
    },
    /// [`ContainerRuntime::start`].
    Start(String),
    /// [`ContainerRuntime::inspect`].
    Inspect(String),
    /// [`ContainerRuntime::logs`].
    Logs {
        /// The container.
        name: String,
        /// The lines asked for.
        tail_lines: u32,
    },
    /// [`ContainerRuntime::logs_since`].
    LogsSince {
        /// The container.
        name: String,
        /// The cursor given.
        cursor: Option<LogCursor>,
        /// The page size asked for.
        max_lines: u32,
    },
    /// [`ContainerRuntime::copy_out`].
    CopyOut {
        /// The container.
        name: String,
        /// The path asked for, as given.
        path: String,
        /// The limits.
        limits: CopyLimits,
    },
    /// [`ContainerRuntime::stop`].
    Stop {
        /// The container.
        name: String,
        /// The grace period.
        grace: Duration,
    },
    /// [`ContainerRuntime::remove`].
    Remove(String),
    /// [`ContainerRuntime::list`].
    List(Label),
    /// [`ContainerRuntime::ensure_network`].
    EnsureNetwork(NetworkSpec),
    /// [`ContainerRuntime::ensure_volume`].
    EnsureVolume(VolumeSpec),
    /// [`ContainerRuntime::sweep`].
    Sweep(Label),
}

impl Call {
    fn method(&self) -> Method {
        match self {
            Self::Create(_) => Method::Create,
            Self::CopyIn { .. } => Method::CopyIn,
            Self::Start(_) => Method::Start,
            Self::Inspect(_) => Method::Inspect,
            Self::Logs { .. } => Method::Logs,
            Self::LogsSince { .. } => Method::LogsSince,
            Self::CopyOut { .. } => Method::CopyOut,
            Self::Stop { .. } => Method::Stop,
            Self::Remove(_) => Method::Remove,
            Self::List(_) => Method::List,
            Self::EnsureNetwork(_) => Method::EnsureNetwork,
            Self::EnsureVolume(_) => Method::EnsureVolume,
            Self::Sweep(_) => Method::Sweep,
        }
    }
}

impl FakeRuntime {
    /// An empty runtime.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends the running container `name` with `status`, `Exited` or `Failed`.
    ///
    /// # Panics
    ///
    /// When the container is not running or `status` is not an end state: the test is wrong.
    pub fn exit_with(&self, name: &str, status: ContainerStatus) {
        assert!(
            matches!(status, ContainerStatus::Exited { .. } | ContainerStatus::Failed { .. }),
            "exit_with({name:?}) needs Exited or Failed, not {status:?}"
        );
        let mut state = self.lock();
        let container = state.containers.get_mut(name).unwrap_or_else(|| panic!("exit_with: no container {name:?}"));
        assert_eq!(container.status, ContainerStatus::Running, "exit_with: container {name:?} is not running");
        container.status = status;
    }

    /// Sets the output of the container `name`, which [`logs`](ContainerRuntime::logs) returns the tail of: one
    /// [`LogStream::Out`] line per line of `text`, stamped at the Unix epoch, in place of every earlier line.
    /// [`logs_since`](ContainerRuntime::logs_since) pages the same lines.
    ///
    /// # Panics
    ///
    /// When there is no container `name`.
    pub fn set_logs(&self, name: &str, text: impl Into<String>) {
        let epoch = cicada::time::from_micros(0);
        let lines = text.into().lines().map(|line| LogLine::new(epoch, LogStream::Out, line)).collect();
        self.change(name, "set_logs", |container| container.logs = lines);
    }

    /// Adds `line` to the end of the log of the container `name`. Times may repeat or go back, as on Docker, where
    /// stdout and stderr are stamped apart; [`logs_since`](ContainerRuntime::logs_since) pages them as Docker does.
    ///
    /// # Panics
    ///
    /// When there is no container `name`.
    pub fn push_log(&self, name: &str, line: LogLine) {
        self.change(name, "push_log", |container| container.logs.push(line));
    }

    /// Adds `file` to the container `name`, in any state, as if its program wrote it there, in place of a file at the
    /// same path; `/out/./a` and `/out/a` are one file. [`copy_out`](ContainerRuntime::copy_out) reads it, unless it
    /// lies on a tmpfs and the container has ended.
    ///
    /// # Panics
    ///
    /// When there is no container `name`, or the path is relative, has a `..` part or names no file.
    pub fn put_file(&self, name: &str, file: CopyFile) {
        let parts = path_parts("put_file into", name, &file.path).unwrap_or_else(|error| panic!("{error}"));
        let file = CopyFile { path: format!("/{}", parts.join("/")), ..file };
        self.change(name, "put_file", |container| {
            container.files.insert(file.path.clone(), file);
        });
    }

    /// Makes the next call of `method` fail with `kind` and without effect. Several queue up, one per call.
    pub fn fail_next(&self, method: Method, kind: ErrorKind) {
        self.lock().fail_next.push((method, kind));
    }

    /// Makes every call of `method` fail with `kind` and without effect, until [`clear_failures`](Self::clear_failures).
    pub fn fail_always(&self, method: Method, kind: ErrorKind) {
        self.lock().fail_always.push((method, kind));
    }

    /// Drops every scripted failure, of [`fail_next`](Self::fail_next) and [`fail_always`](Self::fail_always).
    pub fn clear_failures(&self) {
        let mut state = self.lock();
        state.fail_next.clear();
        state.fail_always.clear();
    }

    /// Every call so far, in order, failed ones too.
    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    /// Runs `change` on the container `name` for the script method `what`, which panics without one.
    fn change(&self, name: &str, what: &str, change: impl FnOnce(&mut FakeContainer)) {
        let mut state = self.lock();
        change(state.containers.get_mut(name).unwrap_or_else(|| panic!("{what}: no container {name:?}")));
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records `call` and returns the state, or the scripted failure of its method.
    fn enter(&self, call: Call) -> Result<MutexGuard<'_, State>, ContainerError> {
        let mut state = self.lock();
        let method = call.method();
        state.calls.push(call);
        if let Some(at) = state.fail_next.iter().position(|(m, _)| *m == method) {
            let (_, kind) = state.fail_next.remove(at);
            return Err(ContainerError::new(kind, format!("scripted failure of {method:?}")));
        }
        if let Some((_, kind)) = state.fail_always.iter().find(|(m, _)| *m == method) {
            return Err(ContainerError::new(*kind, format!("scripted failure of {method:?}")));
        }
        Ok(state)
    }
}

fn conflict(message: String) -> ContainerError {
    ContainerError::new(ErrorKind::Conflict, message)
}

/// The volumes, bind mounts and tmpfs mounts of `spec`.
fn mount_targets(spec: &ContainerSpec) -> Vec<MountTarget> {
    spec.mounts
        .iter()
        .map(|m| MountTarget::new(&m.target, Mounted::Mount { writable: !m.read_only }))
        .chain(spec.tmpfs.iter().map(|t| MountTarget::new(&t.target, Mounted::Tmpfs)))
        .collect()
}

fn has(labels: &BTreeMap<String, String>, label: &Label) -> bool {
    labels.get(&label.key) == Some(&label.value)
}

#[async_trait]
impl ContainerRuntime for FakeRuntime {
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError> {
        let mut state = self.enter(Call::Create(Box::new(spec.clone())))?;
        state.containers.entry(spec.name.clone()).or_insert_with(|| FakeContainer {
            spec: spec.clone(),
            status: ContainerStatus::Created,
            logs: Vec::new(),
            files: BTreeMap::new(),
        });
        Ok(())
    }

    async fn copy_in(&self, name: &str, files: &[CopyFile]) -> Result<(), ContainerError> {
        let mut state = self.enter(Call::CopyIn { name: name.into(), files: files.to_vec() })?;
        let container = state.containers.get_mut(name).ok_or_else(|| conflict(format!("no container {name}")))?;
        if container.status != ContainerStatus::Created {
            return Err(conflict(format!("container {name} is {:?}, not created", container.status)));
        }
        let spec = &container.spec;
        let mounts = mount_targets(spec);
        let mut placed = Vec::with_capacity(files.len());
        for file in files {
            let (parts, _) = place(name, &file.path, spec.read_only_root, &mounts)?;
            let path = format!("/{}", parts.join("/"));
            placed.push(CopyFile { path, ..file.clone() });
        }
        for file in placed {
            container.files.insert(file.path.clone(), file);
        }
        Ok(())
    }

    async fn start(&self, name: &str) -> Result<(), ContainerError> {
        let mut state = self.enter(Call::Start(name.into()))?;
        let container = state.containers.get_mut(name).ok_or_else(|| conflict(format!("no container {name}")))?;
        if container.status == ContainerStatus::Created {
            container.status = ContainerStatus::Running;
        }
        Ok(())
    }

    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError> {
        let state = self.enter(Call::Inspect(name.into()))?;
        Ok(state.containers.get(name).map_or(ContainerStatus::Missing, |container| container.status.clone()))
    }

    async fn logs(&self, name: &str, tail_lines: u32) -> Result<String, ContainerError> {
        let state = self.enter(Call::Logs { name: name.into(), tail_lines })?;
        let container = state.containers.get(name).ok_or_else(|| conflict(format!("no container {name}")))?;
        let start = container.logs.len().saturating_sub(tail_lines as usize);
        Ok(container.logs[start..].iter().map(|line| format!("{}\n", line.text)).collect())
    }

    async fn logs_since(
        &self,
        name: &str,
        cursor: Option<LogCursor>,
        max_lines: u32,
    ) -> Result<LogPage, ContainerError> {
        let state = self.enter(Call::LogsSince { name: name.into(), cursor, max_lines })?;
        check_max_lines(name, max_lines)?;
        let container = state.containers.get(name).ok_or_else(|| conflict(format!("no container {name}")))?;
        // Every line: the pager drops those before the cursor's second, as it does for Docker.
        Ok(page(container.logs.iter().cloned(), cursor, max_lines))
    }

    async fn copy_out(&self, name: &str, path: &str, limits: CopyLimits) -> Result<CopyOut, ContainerError> {
        let state = self.enter(Call::CopyOut { name: name.into(), path: path.into(), limits })?;
        let mut collector = Collector::new(name, path, limits)?;
        let container = state.containers.get(name).ok_or_else(|| conflict(format!("no container {name}")))?;
        let ended = matches!(container.status, ContainerStatus::Exited { .. } | ContainerStatus::Failed { .. });
        let mounts = mount_targets(&container.spec);
        let root = collector.root().to_owned();
        let root: Vec<&str> = root.split('/').skip(1).collect();
        let on_tmpfs = |parts: &[&str]| deepest_mount(parts, &mounts).is_some_and(|m| m.mounted == Mounted::Tmpfs);
        let mut entries: Vec<(Vec<&str>, &CopyFile)> = container
            .files
            .values()
            .map(|file| (file.path.split('/').skip(1).collect::<Vec<_>>(), file))
            .filter(|(parts, _)| parts.starts_with(&root) && !(ended && on_tmpfs(parts)))
            .collect();
        // Docker's archive walks a folder part by part: `/out/a/c` before `/out/a-b`.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (parts, file) in entries {
            // Named as in Docker's archive: relative to the folder that holds `path`.
            let entry = parts[root.len() - 1..].join("/");
            let size = file.contents.len() as u64;
            let Ok(go_on) = collector.offer(&entry, EntryKind::File, size, file.mode, |buf| {
                buf.copy_from_slice(&file.contents);
                Ok::<_, Infallible>(())
            });
            if !go_on {
                break;
            }
        }
        Ok(collector.finish())
    }

    async fn stop(&self, name: &str, grace: Duration) -> Result<(), ContainerError> {
        let mut state = self.enter(Call::Stop { name: name.into(), grace })?;
        if let Some(container) = state.containers.get_mut(name)
            && container.status == ContainerStatus::Running
        {
            container.status = ContainerStatus::Exited { code: STOPPED_CODE };
        }
        Ok(())
    }

    async fn remove(&self, name: &str) -> Result<(), ContainerError> {
        let mut state = self.enter(Call::Remove(name.into()))?;
        state.containers.remove(name);
        Ok(())
    }

    async fn list(&self, label: &Label) -> Result<Vec<ContainerSummary>, ContainerError> {
        let state = self.enter(Call::List(label.clone()))?;
        Ok(state
            .containers
            .iter()
            .filter(|(_, container)| has(&container.spec.labels, label))
            .map(|(name, container)| ContainerSummary::new(name.clone(), container.status.clone()))
            .collect())
    }

    async fn ensure_network(&self, spec: &NetworkSpec) -> Result<(), ContainerError> {
        let mut state = self.enter(Call::EnsureNetwork(spec.clone()))?;
        match state.networks.get(&spec.name) {
            Some(existing) if existing.internal != spec.internal => Err(conflict(format!(
                "network {} exists with internal = {}, not {}",
                spec.name, existing.internal, spec.internal
            ))),
            Some(_) => Ok(()),
            None => {
                state.networks.insert(spec.name.clone(), spec.clone());
                Ok(())
            }
        }
    }

    async fn ensure_volume(&self, spec: &VolumeSpec) -> Result<(), ContainerError> {
        let mut state = self.enter(Call::EnsureVolume(spec.clone()))?;
        state.volumes.entry(spec.name.clone()).or_insert_with(|| spec.clone());
        Ok(())
    }

    async fn sweep(&self, label: &Label) -> Result<Swept, ContainerError> {
        let mut state = self.enter(Call::Sweep(label.clone()))?;
        let mut swept = Swept::default();
        state.containers.retain(|name, container| {
            let labelled = has(&container.spec.labels, label);
            if labelled {
                swept.containers.push(name.clone());
            }
            !labelled
        });
        let state = &mut *state;
        let user_of_network = |network: &str| {
            state.containers.iter().find_map(|(name, container)| match &container.spec.network {
                Networking::Attach(attached) if attached.iter().any(|a| a.name == network) => Some(name.clone()),
                _ => None,
            })
        };
        let mut networks = Vec::new();
        for (name, network) in &state.networks {
            if !has(&network.labels, label) {
                continue;
            }
            match user_of_network(name) {
                Some(user) => swept.skipped.push(Skipped::new(Resource::Network, name, format!("in use by {user}"))),
                None => networks.push(name.clone()),
            }
        }
        let user_of_volume = |volume: &str| {
            state.containers.iter().find_map(|(name, container)| {
                let mounted = container.spec.mounts.iter().any(|m| m.kind == MountKind::Volume && m.source == volume);
                mounted.then(|| name.clone())
            })
        };
        let mut volumes = Vec::new();
        for (name, volume) in &state.volumes {
            if !has(&volume.labels, label) {
                continue;
            }
            match user_of_volume(name) {
                Some(user) => swept.skipped.push(Skipped::new(Resource::Volume, name, format!("in use by {user}"))),
                None => volumes.push(name.clone()),
            }
        }
        for name in &networks {
            state.networks.remove(name);
        }
        for name in &volumes {
            state.volumes.remove(name);
        }
        swept.networks = networks;
        swept.volumes = volumes;
        Ok(swept)
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use super::*;
    use crate::{Mount, NetworkAttachment, Tmpfs};

    /// Runs a future of the fake, which never waits: it is ready at its first poll.
    fn now<T>(future: impl Future<Output = T>) -> T {
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("the fake waited"),
        }
    }

    fn labelled(name: &str, run: &str) -> ContainerSpec {
        let mut spec = ContainerSpec::new(name, "alpine:3");
        spec.labels.insert("run".into(), run.into());
        spec
    }

    fn started(fake: &FakeRuntime, name: &str) {
        now(fake.create(&ContainerSpec::new(name, "alpine:3"))).unwrap();
        now(fake.start(name)).unwrap();
    }

    fn kind<T: std::fmt::Debug>(result: Result<T, ContainerError>) -> ErrorKind {
        result.unwrap_err().kind
    }

    #[test]
    fn a_host_calls_every_method_through_a_trait_object() {
        let runtime: Arc<dyn ContainerRuntime> = Arc::new(FakeRuntime::new());
        let run = Label::new("run", "r1");
        now(runtime.ensure_network(&NetworkSpec::new("net"))).unwrap();
        now(runtime.ensure_volume(&VolumeSpec::new("vol"))).unwrap();
        now(runtime.create(&labelled("job-1", "r1"))).unwrap();
        now(runtime.copy_in("job-1", &[CopyFile::new("/etc/job", "x")])).unwrap();
        now(runtime.start("job-1")).unwrap();
        assert_eq!(now(runtime.inspect("job-1")).unwrap(), ContainerStatus::Running);
        assert_eq!(now(runtime.logs("job-1", 10)).unwrap(), "");
        assert_eq!(now(runtime.list(&run)).unwrap(), [ContainerSummary::new("job-1", ContainerStatus::Running)]);
        now(runtime.stop("job-1", Duration::from_secs(1))).unwrap();
        now(runtime.remove("job-1")).unwrap();
        assert_eq!(now(runtime.sweep(&run)).unwrap(), Swept::default());
    }

    #[test]
    fn a_repeated_create_keeps_the_first_container() {
        let fake = FakeRuntime::new();
        let first = ContainerSpec::new("job-1", "alpine:3");
        let second = ContainerSpec::new("job-1", "busybox:1");
        now(fake.create(&first)).unwrap();
        now(fake.start("job-1")).unwrap();
        now(fake.create(&second)).unwrap();
        let state = fake.lock();
        assert_eq!(state.containers.len(), 1);
        assert_eq!(state.containers["job-1"].spec, first);
        assert_eq!(state.containers["job-1"].status, ContainerStatus::Running, "not replaced");
    }

    #[test]
    fn a_repeated_start_does_not_run_an_ended_container_again() {
        let fake = FakeRuntime::new();
        started(&fake, "exited");
        fake.exit_with("exited", ContainerStatus::Exited { code: 0 });
        now(fake.start("exited")).unwrap();
        assert_eq!(now(fake.inspect("exited")).unwrap(), ContainerStatus::Exited { code: 0 });

        let failed = ContainerStatus::Failed { code: Some(137), reason: "out of memory".into() };
        started(&fake, "failed");
        fake.exit_with("failed", failed.clone());
        now(fake.start("failed")).unwrap();
        assert_eq!(now(fake.inspect("failed")).unwrap(), failed);

        started(&fake, "running");
        now(fake.start("running")).unwrap();
        assert_eq!(now(fake.inspect("running")).unwrap(), ContainerStatus::Running);
    }

    #[test]
    fn stopping_a_running_container_exits_it_and_a_created_one_stays() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        now(fake.stop("job-1", Duration::from_secs(10))).unwrap();
        assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Exited { code: 143 });
        now(fake.stop("job-1", Duration::from_secs(10))).unwrap();
        assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Exited { code: 143 }, "keeps its end");

        now(fake.create(&ContainerSpec::new("job-2", "alpine:3"))).unwrap();
        now(fake.stop("job-2", Duration::ZERO)).unwrap();
        assert_eq!(now(fake.inspect("job-2")).unwrap(), ContainerStatus::Created);
    }

    #[test]
    fn removing_and_stopping_what_is_gone_is_ok() {
        let fake = FakeRuntime::new();
        now(fake.stop("gone", Duration::ZERO)).unwrap();
        now(fake.remove("gone")).unwrap();
        started(&fake, "job-1");
        now(fake.remove("job-1")).unwrap();
        now(fake.remove("job-1")).unwrap();
        assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Missing);
    }

    #[test]
    fn calls_on_a_missing_container_conflict_and_inspect_reports_it_missing() {
        let fake = FakeRuntime::new();
        let error = now(fake.start("gone")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Conflict);
        assert!(!error.is_retryable());
        assert_eq!(kind(now(fake.copy_in("gone", &[]))), ErrorKind::Conflict);
        assert_eq!(kind(now(fake.logs("gone", 5))), ErrorKind::Conflict);
        assert_eq!(now(fake.inspect("gone")).unwrap(), ContainerStatus::Missing);
    }

    #[test]
    fn copy_in_writes_only_into_a_created_container() {
        let fake = FakeRuntime::new();
        now(fake.create(&ContainerSpec::new("job-1", "alpine:3"))).unwrap();
        let files = [CopyFile::new("/work/input.json", "{}")];
        now(fake.copy_in("job-1", &files)).unwrap();
        now(fake.copy_in("job-1", &files)).unwrap();
        assert_eq!(fake.lock().containers["job-1"].files.len(), 1, "the same file again");

        now(fake.start("job-1")).unwrap();
        let late = [CopyFile::new("/work/late", "x")];
        assert_eq!(kind(now(fake.copy_in("job-1", &late))), ErrorKind::Conflict);
        fake.exit_with("job-1", ContainerStatus::Exited { code: 0 });
        assert_eq!(kind(now(fake.copy_in("job-1", &late))), ErrorKind::Conflict);
        assert!(!fake.lock().containers["job-1"].files.contains_key("/work/late"), "no file written");
        assert_eq!(fake.calls()[1], Call::CopyIn { name: "job-1".into(), files: files.to_vec() });
    }

    #[test]
    fn copy_in_refuses_a_relative_path_and_the_read_only_root() {
        let fake = FakeRuntime::new();
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        spec.read_only_root = true;
        spec.mounts.push(Mount::new(MountKind::Volume, "work", "/work"));
        now(fake.create(&spec)).unwrap();
        assert_eq!(kind(now(fake.copy_in("job-1", &[CopyFile::new("work/a", "x")]))), ErrorKind::InvalidSpec);
        let root = [CopyFile::new("/work/a", "x"), CopyFile::new("/workspace/b", "x")];
        assert_eq!(kind(now(fake.copy_in("job-1", &root))), ErrorKind::InvalidSpec);
        assert!(fake.lock().containers["job-1"].files.is_empty(), "all or nothing");
        now(fake.copy_in("job-1", &[CopyFile::new("/work/a", "x")])).unwrap();
    }

    #[test]
    fn copy_in_refuses_a_parent_part_and_a_read_only_mount() {
        let fake = FakeRuntime::new();
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        let mut input = Mount::new(MountKind::Bind, "/srv/in", "/in");
        input.read_only = true;
        spec.mounts = vec![input, Mount::new(MountKind::Volume, "out", "/in/out")];
        now(fake.create(&spec)).unwrap();
        let parent = [CopyFile::new("/work/../etc/passwd", "x")];
        assert_eq!(kind(now(fake.copy_in("job-1", &parent))), ErrorKind::InvalidSpec);
        assert_eq!(kind(now(fake.copy_in("job-1", &[CopyFile::new("/in/a", "x")]))), ErrorKind::InvalidSpec);
        assert!(fake.lock().containers["job-1"].files.is_empty());
        now(fake.copy_in("job-1", &[CopyFile::new("/in/out/a", "x"), CopyFile::new("/etc/b", "x")])).unwrap();
    }

    #[test]
    fn copy_in_refuses_a_mount_target_on_any_root() {
        for read_only_root in [false, true] {
            let fake = FakeRuntime::new();
            let mut spec = ContainerSpec::new("job-1", "alpine:3");
            spec.read_only_root = read_only_root;
            spec.mounts.push(Mount::new(MountKind::Volume, "work", "/work"));
            now(fake.create(&spec)).unwrap();
            for exact in ["/work", "/work/"] {
                let files = [CopyFile::new("/work/a", "x"), CopyFile::new(exact, "x")];
                assert_eq!(
                    kind(now(fake.copy_in("job-1", &files))),
                    ErrorKind::InvalidSpec,
                    "{exact} {read_only_root}"
                );
            }
            assert!(fake.lock().containers["job-1"].files.is_empty(), "nothing written");
        }
    }

    #[test]
    fn copy_in_refuses_a_tmpfs_path_and_the_root_path_on_a_writable_root() {
        let fake = FakeRuntime::new();
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        spec.tmpfs.push(Tmpfs::new("/scratch"));
        now(fake.create(&spec)).unwrap();
        for bad in ["/scratch/c", "/scratch", "/", "/./"] {
            let files = [CopyFile::new("/etc/a", "x"), CopyFile::new(bad, "x")];
            assert_eq!(kind(now(fake.copy_in("job-1", &files))), ErrorKind::InvalidSpec, "{bad}");
        }
        assert!(fake.lock().containers["job-1"].files.is_empty(), "nothing written");
    }

    #[test]
    fn copy_in_keeps_one_file_under_two_spellings() {
        let fake = FakeRuntime::new();
        now(fake.create(&ContainerSpec::new("job-1", "alpine:3"))).unwrap();
        now(fake.copy_in("job-1", &[CopyFile::new("/work/./a", "first")])).unwrap();
        now(fake.copy_in("job-1", &[CopyFile::new("/work/a", "second")])).unwrap();
        let files = &fake.lock().containers["job-1"].files;
        assert_eq!(files.keys().collect::<Vec<_>>(), ["/work/a"]);
        assert_eq!((files["/work/a"].path.as_str(), files["/work/a"].contents.as_slice()), ("/work/a", &b"second"[..]));
    }

    #[test]
    fn logs_return_the_tail() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        fake.set_logs("job-1", "one\ntwo\nthree\n");
        assert_eq!(now(fake.logs("job-1", 2)).unwrap(), "two\nthree\n");
        assert_eq!(now(fake.logs("job-1", 10)).unwrap(), "one\ntwo\nthree\n");
        assert_eq!(now(fake.logs("job-1", 0)).unwrap(), "");
    }

    fn texts(page: &LogPage) -> Vec<(LogStream, &str)> {
        page.lines.iter().map(|line| (line.stream, line.text.as_str())).collect()
    }

    #[test]
    fn scripted_log_lines_are_paged_and_read_by_logs() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        let time = cicada::time::from_micros(1_764_000_000_000_000);
        fake.push_log("job-1", LogLine::new(time, LogStream::Out, "one"));
        fake.push_log("job-1", LogLine::new(time, LogStream::Err, "two"));
        fake.push_log("job-1", LogLine::new(time, LogStream::Out, "three"));
        let first = now(fake.logs_since("job-1", None, 2)).unwrap();
        assert_eq!(texts(&first), [(LogStream::Out, "one"), (LogStream::Err, "two")]);
        assert_eq!(first.lines[0].time, time);
        let second = now(fake.logs_since("job-1", Some(first.next), 2)).unwrap();
        assert_eq!(texts(&second), [(LogStream::Out, "three")]);
        let empty = now(fake.logs_since("job-1", Some(second.next), 2)).unwrap();
        assert_eq!(empty, LogPage::new(Vec::new(), second.next), "an empty page keeps the cursor");
        assert_eq!(now(fake.logs("job-1", 10)).unwrap(), "one\ntwo\nthree\n");
        // A cursor kept as JSON reads the same page.
        let stored: LogCursor = serde_json::from_str(&serde_json::to_string(&first.next).unwrap()).unwrap();
        assert_eq!(now(fake.logs_since("job-1", Some(stored), 2)).unwrap(), second);
    }

    #[test]
    fn set_logs_gives_out_lines_at_the_epoch_to_both_calls() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        fake.set_logs("job-1", "one\ntwo\n");
        let page = now(fake.logs_since("job-1", None, 10)).unwrap();
        assert_eq!(texts(&page), [(LogStream::Out, "one"), (LogStream::Out, "two")]);
        assert!(page.lines.iter().all(|line| line.time.timestamp() == 0));
        assert_eq!(now(fake.logs("job-1", 1)).unwrap(), "two\n");
        // A pushed line follows the set ones; set_logs replaces them all.
        fake.push_log("job-1", LogLine::new(cicada::time::from_micros(5_000_000), LogStream::Err, "three"));
        assert_eq!(now(fake.logs("job-1", 10)).unwrap(), "one\ntwo\nthree\n");
        fake.set_logs("job-1", "four");
        assert_eq!(texts(&now(fake.logs_since("job-1", None, 10)).unwrap()), [(LogStream::Out, "four")]);
    }

    #[test]
    fn logs_since_of_a_missing_or_created_container_and_a_bad_page_size() {
        let fake = FakeRuntime::new();
        assert_eq!(kind(now(fake.logs_since("gone", None, 5))), ErrorKind::Conflict);
        now(fake.create(&ContainerSpec::new("job-1", "alpine:3"))).unwrap();
        assert_eq!(now(fake.logs_since("job-1", None, 5)).unwrap(), LogPage::new(Vec::new(), "0:0".parse().unwrap()));
        for bad in [0, 1001] {
            assert_eq!(kind(now(fake.logs_since("job-1", None, bad))), ErrorKind::InvalidSpec, "{bad}");
        }
        assert!(now(fake.logs_since("job-1", None, crate::LOG_PAGE_MAX)).is_ok());
    }

    #[test]
    fn a_scripted_failure_of_logs_since_is_recorded() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        fake.set_logs("job-1", "one");
        fake.fail_next(Method::LogsSince, ErrorKind::DaemonUnavailable);
        let cursor: LogCursor = "0:0".parse().unwrap();
        assert_eq!(kind(now(fake.logs_since("job-1", Some(cursor), 3))), ErrorKind::DaemonUnavailable);
        assert_eq!(now(fake.logs_since("job-1", Some(cursor), 3)).unwrap().lines.len(), 1);
        let call = Call::LogsSince { name: "job-1".into(), cursor: Some(cursor), max_lines: 3 };
        assert_eq!(fake.calls().iter().filter(|c| **c == call).count(), 2);
    }

    #[test]
    fn a_runtime_without_logs_since_or_copy_out_compiles_and_fails_with_other() {
        /// A host's runtime written before `logs_since` and `copy_out`.
        struct Older;

        #[async_trait]
        impl ContainerRuntime for Older {
            async fn create(&self, _: &ContainerSpec) -> Result<(), ContainerError> {
                Ok(())
            }
            async fn copy_in(&self, _: &str, _: &[CopyFile]) -> Result<(), ContainerError> {
                Ok(())
            }
            async fn start(&self, _: &str) -> Result<(), ContainerError> {
                Ok(())
            }
            async fn inspect(&self, _: &str) -> Result<ContainerStatus, ContainerError> {
                Ok(ContainerStatus::Missing)
            }
            async fn logs(&self, _: &str, _: u32) -> Result<String, ContainerError> {
                Ok(String::new())
            }
            async fn stop(&self, _: &str, _: Duration) -> Result<(), ContainerError> {
                Ok(())
            }
            async fn remove(&self, _: &str) -> Result<(), ContainerError> {
                Ok(())
            }
            async fn list(&self, _: &Label) -> Result<Vec<ContainerSummary>, ContainerError> {
                Ok(Vec::new())
            }
            async fn ensure_network(&self, _: &NetworkSpec) -> Result<(), ContainerError> {
                Ok(())
            }
            async fn ensure_volume(&self, _: &VolumeSpec) -> Result<(), ContainerError> {
                Ok(())
            }
            async fn sweep(&self, _: &Label) -> Result<Swept, ContainerError> {
                Ok(Swept::default())
            }
        }

        let runtime: Arc<dyn ContainerRuntime> = Arc::new(Older);
        let error = now(runtime.logs_since("job-1", None, 10)).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Other, "{error}");
        assert!(error.message.contains("not supported"), "{error}");
        let error = now(runtime.copy_out("job-1", "/out", CopyLimits::new(10, 100))).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Other, "{error}");
        assert!(error.message.contains("not supported"), "{error}");
    }

    const ROOMY: CopyLimits = CopyLimits { max_files: 100, max_bytes: 1 << 20 };

    fn with_mode(path: &str, contents: &str, mode: u32) -> CopyFile {
        CopyFile { mode, ..CopyFile::new(path, contents) }
    }

    fn skipped(out: &CopyOut) -> Vec<(&str, &str)> {
        out.skipped.iter().map(|s| (s.path.as_str(), s.reason.as_str())).collect()
    }

    #[test]
    fn copy_out_returns_what_copy_in_wrote_and_what_the_program_wrote() {
        let fake = FakeRuntime::new();
        now(fake.create(&ContainerSpec::new("job-1", "alpine:3"))).unwrap();
        now(fake.copy_in("job-1", &[CopyFile::new("/in/config.json", "{}")])).unwrap();
        let out = now(fake.copy_out("job-1", "/in", ROOMY)).unwrap();
        assert_eq!(out, CopyOut::new(vec![CopyFile::new("/in/config.json", "{}")], Vec::new()), "from a created one");

        now(fake.start("job-1")).unwrap();
        fake.put_file("job-1", with_mode("/out/./sub/b", "bee", 0o755));
        fake.put_file("job-1", CopyFile::new("/out/a", "ay"));
        fake.exit_with("job-1", ContainerStatus::Exited { code: 0 });
        let out = now(fake.copy_out("job-1", "/out/", ROOMY)).unwrap();
        assert_eq!(out.files, [CopyFile::new("/out/a", "ay"), with_mode("/out/sub/b", "bee", 0o755)]);
        assert!(out.skipped.is_empty());
        let one = now(fake.copy_out("job-1", "/out/sub/b", ROOMY)).unwrap();
        assert_eq!(one.files, [with_mode("/out/sub/b", "bee", 0o755)], "one file by its path");
    }

    #[test]
    fn copy_out_walks_a_folder_part_by_part() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        for path in ["/out/a-b", "/out/a/c", "/out/b", "/out/a/b/d", "/outer/x", "/ou"] {
            fake.put_file("job-1", CopyFile::new(path, "x"));
        }
        let out = now(fake.copy_out("job-1", "/out", ROOMY)).unwrap();
        let paths: Vec<&str> = out.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["/out/a/b/d", "/out/a/c", "/out/a-b", "/out/b"]);
    }

    #[test]
    fn copy_out_skips_a_file_over_the_bytes_left_and_stops_at_the_file_limit() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        for (path, size) in [("/out/a", 10), ("/out/b", 5000), ("/out/c", 20)] {
            fake.put_file("job-1", CopyFile::new(path, vec![b'x'; size]));
        }
        let out = now(fake.copy_out("job-1", "/out", CopyLimits::new(10, 100))).unwrap();
        assert_eq!(out.files.iter().map(|f| f.contents.len()).collect::<Vec<_>>(), [10, 20]);
        assert_eq!(skipped(&out), [("/out/b", "5000 bytes, more than the 90 left")]);

        let out = now(fake.copy_out("job-1", "/out", CopyLimits::new(2, 100))).unwrap();
        assert_eq!(out.files.len(), 1);
        assert_eq!(
            skipped(&out),
            [("/out/b", "5000 bytes, more than the 90 left"), ("/out", "more than 2 files; the rest was not read")]
        );
        let out = now(fake.copy_out("job-1", "/out", CopyLimits::new(3, 100))).unwrap();
        assert_eq!((out.files.len(), out.skipped.len()), (2, 1), "exactly at the limit: no last entry");
    }

    #[test]
    fn copy_out_of_what_is_not_there() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        fake.put_file("job-1", CopyFile::new("/out/a", "x"));
        assert_eq!(now(fake.copy_out("job-1", "/missing", ROOMY)).unwrap(), CopyOut::default());
        assert_eq!(now(fake.copy_out("job-1", "/out/a/b", ROOMY)).unwrap(), CopyOut::default(), "below a file");
        assert_eq!(kind(now(fake.copy_out("gone", "/out", ROOMY))), ErrorKind::Conflict);
        for bad in ["out", "/", "/out/../etc"] {
            assert_eq!(kind(now(fake.copy_out("job-1", bad, ROOMY))), ErrorKind::InvalidSpec, "{bad}");
        }
        for zero in [CopyLimits::new(0, 100), CopyLimits::new(10, 0)] {
            assert_eq!(kind(now(fake.copy_out("job-1", "/out", zero))), ErrorKind::InvalidSpec, "{zero:?}");
            assert_eq!(kind(now(fake.copy_out("gone", "/out", zero))), ErrorKind::InvalidSpec, "before the lookup");
        }
    }

    #[test]
    fn a_tmpfs_is_empty_once_the_container_ended() {
        let fake = FakeRuntime::new();
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        spec.tmpfs.push(Tmpfs::new("/scratch"));
        spec.mounts.push(Mount::new(MountKind::Volume, "keep", "/scratch/keep"));
        now(fake.create(&spec)).unwrap();
        now(fake.start("job-1")).unwrap();
        fake.put_file("job-1", CopyFile::new("/scratch/a", "x"));
        fake.put_file("job-1", CopyFile::new("/scratch/keep/b", "y"));
        let running = now(fake.copy_out("job-1", "/scratch", ROOMY)).unwrap();
        assert_eq!(running.files.len(), 2, "while it runs");
        fake.exit_with("job-1", ContainerStatus::Exited { code: 0 });
        let ended = now(fake.copy_out("job-1", "/scratch", ROOMY)).unwrap();
        assert_eq!(ended.files, [CopyFile::new("/scratch/keep/b", "y")], "a volume below the tmpfs stays");
        assert_eq!(now(fake.copy_out("job-1", "/scratch/a", ROOMY)).unwrap(), CopyOut::default());
    }

    #[test]
    fn a_scripted_failure_of_copy_out_is_recorded() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        fake.put_file("job-1", CopyFile::new("/out/a", "x"));
        fake.fail_next(Method::CopyOut, ErrorKind::DaemonUnavailable);
        assert_eq!(kind(now(fake.copy_out("job-1", "/out", ROOMY))), ErrorKind::DaemonUnavailable);
        assert_eq!(now(fake.copy_out("job-1", "/out", ROOMY)).unwrap().files.len(), 1);
        let call = Call::CopyOut { name: "job-1".into(), path: "/out".into(), limits: ROOMY };
        assert_eq!(fake.calls().iter().filter(|c| **c == call).count(), 2);
    }

    #[test]
    #[should_panic(expected = "put_file: no container")]
    fn put_file_panics_for_a_missing_container() {
        FakeRuntime::new().put_file("gone", CopyFile::new("/out/a", "x"));
    }

    #[test]
    fn a_repeated_ensure_keeps_one_network_and_volume() {
        let fake = FakeRuntime::new();
        let mut network = NetworkSpec::new("net");
        network.internal = true;
        now(fake.ensure_network(&network)).unwrap();
        now(fake.ensure_network(&network)).unwrap();
        now(fake.ensure_volume(&VolumeSpec::new("vol"))).unwrap();
        now(fake.ensure_volume(&VolumeSpec::new("vol"))).unwrap();
        let state = fake.lock();
        assert_eq!((state.networks.len(), state.volumes.len()), (1, 1));
    }

    #[test]
    fn an_existing_network_with_another_internal_flag_conflicts() {
        let fake = FakeRuntime::new();
        let mut internal = NetworkSpec::new("net");
        internal.internal = true;
        now(fake.ensure_network(&internal)).unwrap();
        let mut labelled = NetworkSpec::new("net");
        labelled.internal = true;
        labelled.labels.insert("other".into(), "label".into());
        now(fake.ensure_network(&labelled)).unwrap();
        assert_eq!(kind(now(fake.ensure_network(&NetworkSpec::new("net")))), ErrorKind::Conflict);
        assert!(fake.lock().networks["net"].internal);
    }

    #[test]
    fn a_sweep_removes_by_label() {
        let fake = FakeRuntime::new();
        for run in ["r1", "r2"] {
            let mut network = NetworkSpec::new(format!("net-{run}"));
            network.labels.insert("run".into(), run.into());
            now(fake.ensure_network(&network)).unwrap();
            let mut volume = VolumeSpec::new(format!("vol-{run}"));
            volume.labels.insert("run".into(), run.into());
            now(fake.ensure_volume(&volume)).unwrap();
            let mut spec = labelled(&format!("job-{run}"), run);
            spec.network = Networking::Attach(vec![NetworkAttachment::new(format!("net-{run}"))]);
            spec.mounts.push(Mount::new(MountKind::Volume, format!("vol-{run}"), "/work"));
            now(fake.create(&spec)).unwrap();
            now(fake.start(&format!("job-{run}"))).unwrap();
        }
        let swept = now(fake.sweep(&Label::new("run", "r1"))).unwrap();
        assert_eq!(swept.containers, ["job-r1"]);
        assert_eq!(swept.networks, ["net-r1"]);
        assert_eq!(swept.volumes, ["vol-r1"]);
        assert!(swept.skipped.is_empty());
        let state = fake.lock();
        assert_eq!(state.containers.keys().collect::<Vec<_>>(), ["job-r2"]);
        assert_eq!(state.networks.keys().collect::<Vec<_>>(), ["net-r2"]);
        assert_eq!(state.volumes.keys().collect::<Vec<_>>(), ["vol-r2"]);
        drop(state);
        assert_eq!(now(fake.sweep(&Label::new("run", "r1"))).unwrap(), Swept::default(), "nothing left");
    }

    #[test]
    fn a_sweep_skips_a_network_and_a_volume_in_use() {
        let fake = FakeRuntime::new();
        let mut network = NetworkSpec::new("net-r1");
        network.labels.insert("run".into(), "r1".into());
        now(fake.ensure_network(&network)).unwrap();
        let mut volume = VolumeSpec::new("vol-r1");
        volume.labels.insert("run".into(), "r1".into());
        now(fake.ensure_volume(&volume)).unwrap();
        let mut other = ContainerSpec::new("other", "alpine:3");
        other.network = Networking::Attach(vec![NetworkAttachment::new("net-r1")]);
        other.mounts.push(Mount::new(MountKind::Volume, "vol-r1", "/data"));
        now(fake.create(&other)).unwrap();

        let swept = now(fake.sweep(&Label::new("run", "r1"))).unwrap();
        assert!(swept.networks.is_empty() && swept.volumes.is_empty());
        assert_eq!(
            swept.skipped,
            [
                Skipped::new(Resource::Network, "net-r1", "in use by other"),
                Skipped::new(Resource::Volume, "vol-r1", "in use by other"),
            ]
        );
        let state = fake.lock();
        assert!(state.networks.contains_key("net-r1") && state.volumes.contains_key("vol-r1"));
    }

    #[test]
    fn scripted_exit() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        fake.exit_with("job-1", ContainerStatus::Exited { code: 3 });
        assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Exited { code: 3 });
        let methods: Vec<Method> = fake.calls().iter().map(Call::method).collect();
        assert_eq!(methods, [Method::Create, Method::Start, Method::Inspect]);
    }

    #[test]
    fn scripted_failure() {
        let fake = FakeRuntime::new();
        let spec = ContainerSpec::new("job-1", "alpine:3");
        fake.fail_next(Method::Create, ErrorKind::DaemonUnavailable);
        let error = now(fake.create(&spec)).unwrap_err();
        assert_eq!(error.kind, ErrorKind::DaemonUnavailable);
        assert!(error.is_retryable());
        assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Missing);
        now(fake.create(&spec)).unwrap();
        assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Created);
    }

    #[test]
    fn a_failure_lasts_until_it_is_cleared_and_has_no_effect() {
        let fake = FakeRuntime::new();
        started(&fake, "job-1");
        fake.fail_always(Method::Stop, ErrorKind::Other);
        for _ in 0..2 {
            assert_eq!(kind(now(fake.stop("job-1", Duration::ZERO))), ErrorKind::Other);
            assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Running);
        }
        fake.clear_failures();
        now(fake.stop("job-1", Duration::ZERO)).unwrap();
        assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Exited { code: 143 });
        assert_eq!(fake.calls().iter().filter(|call| call.method() == Method::Stop).count(), 3, "failed ones too");
    }

    #[test]
    fn clones_share_their_state() {
        let fake = FakeRuntime::new();
        let clone = fake.clone();
        started(&clone, "job-1");
        assert_eq!(now(fake.inspect("job-1")).unwrap(), ContainerStatus::Running);
        assert_eq!(fake.calls(), clone.calls());
    }
}
