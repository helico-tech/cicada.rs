//! The [`ContainerRuntime`] trait and what it reports.

use std::time::Duration;

use async_trait::async_trait;

use crate::{
    ContainerError, ContainerSpec, CopyFile, CopyLimits, CopyOut, ErrorKind, Label, LogCursor, LogPage, NetworkSpec,
    VolumeSpec,
};

/// A container runtime: Docker, or the fake in tests. Use it as `Arc<dyn ContainerRuntime>` to pick one at run time.
///
/// Every call is safe to repeat: called again with the same arguments, it gives the same end state. Containers,
/// networks and volumes are found by name. Every method fails only with a [`ContainerError`], whose kind says whether
/// a retry can help.
#[async_trait]
pub trait ContainerRuntime: Send + Sync + 'static {
    /// Creates the container `spec.name`, pulling its image when the backend has to. An existing container of that
    /// name is kept as it is, without comparing it to `spec`, and the call is `Ok`: the name is its identity.
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError>;

    /// Writes `files` into the container `name`, which must be `Created`: meant for a container before
    /// [`start`](Self::start). Repeating it writes the same files again. Fails with `Conflict` for a container that is
    /// missing, running or ended. Fails with `InvalidSpec`, before any write, for a path that is relative, has a `..`
    /// part or names no file once empty and `.` parts are dropped (`/`); one equal to a mount target, as a file cannot
    /// replace a mount point; one at or below a tmpfs with no deeper volume or bind mount, as the tmpfs mounted at
    /// start would hide it; one on a read-only mount; and, with [`read_only_root`](ContainerSpec::read_only_root), one
    /// on no volume or bind mount. `/work/./a` and `/work/a` are one file.
    async fn copy_in(&self, name: &str, files: &[CopyFile]) -> Result<(), ContainerError>;

    /// Starts the created container `name`. For a running, exited or failed one it is `Ok` and does nothing, so a
    /// container runs at most once. Fails with `Conflict` for a missing one.
    async fn start(&self, name: &str) -> Result<(), ContainerError>;

    /// The status of the container `name`; [`ContainerStatus::Missing`] when there is none, not an error.
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError>;

    /// The last `tail_lines` lines of the container's stdout and stderr together, as lossy UTF-8. Fails with
    /// `Conflict` for a missing container.
    async fn logs(&self, name: &str, tail_lines: u32) -> Result<String, ContainerError>;

    /// Up to `max_lines` log lines of the container `name` after `cursor` (from the first line for `None`), each
    /// with its time, its stream and its text as lossy UTF-8 without the newline, and the cursor to call again with.
    /// For a host that follows a run's output and keeps it outside the container; call it outside actor turns, as a
    /// page is too big for actor state.
    ///
    /// Paging to the end gives every line once, in log order, and the same cursor gives the same lines first. A page
    /// with fewer than `max_lines` lines has read everything there is now: call again later with
    /// [`next`](LogPage::next), which an empty page leaves at the cursor given (the start for `None`). `max_lines`
    /// outside 1 to [`LOG_PAGE_MAX`](crate::LOG_PAGE_MAX) fails with `InvalidSpec` before any read. A missing container fails with
    /// `Conflict`, also when it is removed during the read; a created one gives an empty page, a running or ended
    /// one its lines. A container whose log cannot be read, such as Docker's log driver `none`, fails with `Other`,
    /// not `DaemonUnavailable`, so a follower stops retrying.
    ///
    /// Limits, from how Docker keeps logs: a line stamped before the cursor's second but written after it (a race of
    /// microseconds at a second boundary between stdout and stderr) is missed. Each read makes Docker scan the log
    /// from the cursor's second, so poll gently, every few seconds, not in a tight loop. Log rotation that removes
    /// part of that second shifts the cursor's count. A line longer than 16 KiB comes back as several lines, as
    /// Docker stores it in parts. Docker takes the second as a 32-bit number: a cursor after January 2038 fails with
    /// `Other` rather than read from the wrong time.
    ///
    /// The default fails with `Other`, so a runtime written before this method compiles unchanged; the fake and
    /// the Docker backend implement it.
    async fn logs_since(
        &self,
        name: &str,
        cursor: Option<LogCursor>,
        max_lines: u32,
    ) -> Result<LogPage, ContainerError> {
        let _ = (cursor, max_lines);
        Err(ContainerError::new(
            ErrorKind::Other,
            format!("logs of {name}: logs_since is not supported by this runtime"),
        ))
    }

    /// The regular files at or below `path` of the container `name`, created, running or ended (until
    /// [`remove`](Self::remove)): every file below a folder, or the one file `path` names. Each kept file is whole,
    /// with its absolute path, contents and permission bits, so it can go to [`copy_in`](Self::copy_in) of another
    /// container. For a host that collects a run's artifacts after it ended, or reads a small progress file while it
    /// runs; call it outside actor turns, as the files are too big for actor state.
    ///
    /// **Limits.** A file goes into [`files`](CopyOut::files) when its size fits what is left of
    /// [`max_bytes`](CopyLimits::max_bytes); else it is in [`skipped`](CopyOut::skipped) with its size, and the copy
    /// goes on, so one huge file costs only itself. A file is whole or skipped, never cut. Once `files` and `skipped`
    /// together hold [`max_files`](CopyLimits::max_files) entries, the next entry that would go into either stops the
    /// copy, and one last skipped entry for `path` says the rest was not read; with exactly `max_files` entries there
    /// is no such entry. So memory is at most `max_bytes`, plus `max_files + 1` paths, whatever the folder holds.
    ///
    /// **Links.** Only regular files are copied, and no link is followed: a symbolic link, inside the folder or out
    /// of it, a hard link, a device, a pipe and a socket are skipped with a reason, and so is an entry whose name is
    /// not under `path`. A folder is not listed: the paths of its files show it.
    ///
    /// Fails with `InvalidSpec`, before any call, for a `path` that is relative, has a `..` part or names no file once
    /// empty and `.` parts are dropped (`/`), and for a zero limit. Files come back under the cleaned path. Fails with
    /// `Conflict` for a missing container, also one removed during the call. A missing `path` gives an empty result.
    ///
    /// **What the container shows.** Files written to a tmpfs are gone once the container ended (Docker then shows
    /// the image's files under that path, if any): artifacts belong on the container's own file system or on a
    /// volume. A file being written while it is read can come back half written: a program that updates a progress
    /// file should write it to a temporary name in the same folder and rename it into place.
    ///
    /// Further limits: dropping the returned future does not stop the read, which goes on to the end of what the
    /// runtime sends and then drops it. A file that grows, shrinks or vanishes while Docker builds its archive can
    /// come back short, or be missing, without an error. The order of the entries, and so which files fit the limits
    /// first, may differ between Docker engine versions.
    ///
    /// The default fails with `Other`, so a runtime written before this method compiles unchanged; the fake and
    /// the Docker backend implement it.
    async fn copy_out(&self, name: &str, path: &str, limits: CopyLimits) -> Result<CopyOut, ContainerError> {
        let _ = (path, limits);
        Err(ContainerError::new(
            ErrorKind::Other,
            format!("copy out of {name}: copy_out is not supported by this runtime"),
        ))
    }

    /// Stops the running container `name`, killing it after `grace`; it then reports `Exited` with the code the
    /// runtime gave the stopped process (Docker: 143 after SIGTERM, 137 after the grace period). A created container
    /// stays `Created`, an ended one keeps its end, and a missing one is `Ok`.
    async fn stop(&self, name: &str, grace: Duration) -> Result<(), ContainerError>;

    /// Removes the container `name`, forced: a running one is killed first. A missing one is `Ok`.
    async fn remove(&self, name: &str) -> Result<(), ContainerError>;

    /// The containers that carry `label`, with their status, in name order.
    async fn list(&self, label: &Label) -> Result<Vec<ContainerSummary>, ContainerError>;

    /// Creates the network `spec.name`, or finds it. Only the `internal` flag of an existing network is compared, as it
    /// decides isolation: another flag fails with `Conflict`.
    async fn ensure_network(&self, spec: &NetworkSpec) -> Result<(), ContainerError>;

    /// Creates the volume `spec.name`, or finds it.
    async fn ensure_volume(&self, spec: &VolumeSpec) -> Result<(), ContainerError>;

    /// Removes every container, then every network, then every volume that carries `label`, and says what it removed
    /// and what it skipped. Best effort: a network or volume still in use, or anything the runtime refuses to remove,
    /// is skipped with its reason and the sweep goes on; only `DaemonUnavailable` ends it with an error, and the next
    /// sweep finishes the work. `Ok` with nothing removed when nothing carries the label. The result names only what
    /// this sweep removed or skipped: something already gone, or a container another caller is removing, is in no list.
    async fn sweep(&self, label: &Label) -> Result<Swept, ContainerError>;
}

/// The status of a container. Two end states tell a program's own exit from the runtime's failure.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerStatus {
    /// There is no container of that name.
    Missing,
    /// Created and not started.
    Created,
    /// Its process runs.
    Running,
    /// Its process exited with `code`; a container ended by [`stop`](ContainerRuntime::stop) is here too.
    Exited {
        /// The exit code.
        code: i64,
    },
    /// The runtime ended it: it did not start, ran out of memory or was lost by the runtime.
    Failed {
        /// The exit code, when the runtime has one.
        code: Option<i64>,
        /// Why.
        reason: String,
    },
}

/// A container found by [`list`](ContainerRuntime::list).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerSummary {
    /// Its name.
    pub name: String,
    /// Its status.
    pub status: ContainerStatus,
}

impl ContainerSummary {
    /// The container `name` with `status`.
    pub fn new(name: impl Into<String>, status: ContainerStatus) -> Self {
        Self { name: name.into(), status }
    }
}

/// What a [`sweep`](ContainerRuntime::sweep) removed, per kind, and what it skipped. Something another caller removed
/// first, or a container another caller is removing, is in no list.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// The names of the containers this sweep removed.
    pub containers: Vec<String>,
    /// The names of the networks this sweep removed.
    pub networks: Vec<String>,
    /// The names of the volumes this sweep removed.
    pub volumes: Vec<String>,
    /// What it left, with the reason.
    pub skipped: Vec<Skipped>,
}

/// Something a [`sweep`](ContainerRuntime::sweep) left in place.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// Its kind.
    pub resource: Resource,
    /// Its name.
    pub name: String,
    /// Why it is left, such as the container that still uses a network.
    pub reason: String,
}

impl Skipped {
    /// The `resource` called `name`, left for `reason`.
    pub fn new(resource: Resource, name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self { resource, name: name.into(), reason: reason.into() }
    }
}

/// A kind of thing the runtime keeps.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// A container.
    Container,
    /// A network.
    Network,
    /// A volume.
    Volume,
}
