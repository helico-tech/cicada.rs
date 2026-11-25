//! The [`ContainerRuntime`] trait and what it reports.

use std::time::Duration;

use async_trait::async_trait;

use crate::{
    ContainerError, ContainerSpec, CopyDir, CopyFile, CopyLimits, CopyOut, ErrorKind, Label, LogCursor, LogPage,
    NetworkSpec, VolumeSpec,
};

/// The reason of [`ContainerStatus::Failed`] for a container the runtime killed for lack of memory, exactly; the
/// container actor reports such an end with the kind [`OutOfMemory`](crate::FailureKind::OutOfMemory).
pub const OUT_OF_MEMORY: &str = "out of memory";

/// A container runtime: Docker, or the fake in tests. Use it as `Arc<dyn ContainerRuntime>` to pick one at run time.
///
/// Every call is safe to repeat: called again with the same arguments, it gives the same end state. Containers,
/// networks and volumes are found by name. Every method fails only with a [`ContainerError`], whose kind says whether
/// a retry can help. [`inspect`](Self::inspect) reports a container killed for lack of memory as
/// [`ContainerStatus::Failed`] with the reason [`OUT_OF_MEMORY`], which the container actor matches exactly: another
/// runtime must use it too.
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

    /// Makes the missing folders of `dirs` in the container `name`, which must be `Created`, so that a program that
    /// does not run as root can write there: each missing named folder with its [`mode`](CopyDir::mode), and each
    /// missing parent with `0o755`. For a host that prepares output folders before [`start`](Self::start).
    ///
    /// **Only what is missing is made.** An existing folder is left exactly as it is, whoever owns it and whatever its
    /// mode, also when `dirs` names it with another mode; so is an existing parent. A folder named in `dirs` gets its
    /// own mode also when it is a parent of another named folder, in any order. The same path named twice with one
    /// mode is made once. A repeat makes nothing, as everything exists then: the same end state.
    ///
    /// **Links are followed to a folder.** A symbolic link at a named path or a parent, such as `/var/run -> /run`,
    /// counts as the folder it points to; a folder made below it is made at the link's target, and the link stays.
    /// The path rule below applies to that target too. The walk of one named folder follows at most 8 links, counting
    /// every link on its way; one more is `Conflict`. Docker resolves a chain of links itself, so a chain counts as one
    /// link there, and a link loop fails with `Other` from Docker.
    ///
    /// Fails, before any write, with `Conflict` for a file or a link to anything but a folder (a missing target too)
    /// at a named path or a parent, more than 8 links in the walk, and a container that is missing, running or ended.
    /// Fails with `InvalidSpec` for a mode above `0o7777`, one path named twice with two modes, and every path
    /// [`copy_in`](Self::copy_in) refuses: relative, with a `..` part, `/`, a mount target, at or below a tmpfs, on a
    /// read-only mount, and with [`read_only_root`](ContainerSpec::read_only_root) on no volume or bind mount.
    ///
    /// Limits: the mode of a volume's own root folder cannot be set, as the path rule refuses a mount target: make a
    /// folder inside the volume, or set the mode in the image. A folder that another API caller makes between the
    /// check and the write is not seen; the container has no process yet, so nothing else can race.
    ///
    /// The default fails with `Unsupported`, so a runtime written before this method compiles unchanged; the fake and
    /// the Docker backend implement it.
    async fn make_dirs(&self, name: &str, dirs: &[CopyDir]) -> Result<(), ContainerError> {
        let _ = dirs;
        Err(ContainerError::new(
            ErrorKind::Unsupported,
            format!("make folders in {name}: make_dirs is not supported by this runtime"),
        ))
    }

    /// Starts the created container `name`. For a running, exited or failed one it is `Ok` and does nothing, so a
    /// container runs at most once. Fails with `Conflict` for a missing one.
    async fn start(&self, name: &str) -> Result<(), ContainerError>;

    /// The status of the container `name`; [`ContainerStatus::Missing`] when there is none, not an error.
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError>;

    /// The last `tail_lines` lines of the container's stdout and stderr together, as lossy UTF-8. Fails with
    /// `Conflict` for a missing container, and with `Unsupported` for one whose log the runtime can never read, such
    /// as Docker's log driver `none`.
    async fn logs(&self, name: &str, tail_lines: u32) -> Result<String, ContainerError>;

    /// Up to `max_lines` log lines of the container `name` after `cursor` (from the first line for `None`), each
    /// with its time, its stream and its text as lossy UTF-8 without the newline, and the cursor to call again with.
    /// For a host that follows a run's output and keeps it outside the container, such as the container actor with a
    /// store ([`ContainerActor::with_store`](crate::ContainerActor::with_store)). A page goes to a store, not into
    /// actor state; called in a turn, a long read needs the job's [`MAX_TURN`](crate::ContainerJob::MAX_TURN).
    ///
    /// Paging to the end gives every line once, in log order, and the same cursor gives the same lines first. A page
    /// with fewer than `max_lines` lines has read everything there is now: call again later with
    /// [`next`](LogPage::next), which an empty page leaves at the cursor given (the start for `None`). `max_lines`
    /// outside 1 to [`LOG_PAGE_MAX`](crate::LOG_PAGE_MAX) fails with `InvalidSpec` before any read. A missing container fails with
    /// `Conflict`, also when it is removed during the read; a created one gives an empty page, a running or ended
    /// one its lines. A container whose log the runtime can never read, such as Docker's log driver `none`, fails with
    /// `Unsupported`, so a follower stops; it may retry the other kinds, counting errors in a row.
    ///
    /// Limits, from how Docker keeps logs: a line stamped before the cursor's second but written after it (a race of
    /// microseconds at a second boundary between stdout and stderr) is missed. Each read makes Docker scan the log
    /// from the cursor's second, so poll gently, every few seconds, not in a tight loop. Log rotation that removes
    /// part of that second shifts the cursor's count. A line longer than 16 KiB comes back as several lines, as
    /// Docker stores it in parts. Docker takes the second as a 32-bit number: a cursor after January 2038 fails with
    /// `Other` rather than read from the wrong time.
    ///
    /// The default fails with `Unsupported`, so a runtime written before this method compiles unchanged; the fake and
    /// the Docker backend implement it.
    async fn logs_since(
        &self,
        name: &str,
        cursor: Option<LogCursor>,
        max_lines: u32,
    ) -> Result<LogPage, ContainerError> {
        let _ = (cursor, max_lines);
        Err(ContainerError::new(
            ErrorKind::Unsupported,
            format!("logs of {name}: logs_since is not supported by this runtime"),
        ))
    }

    /// The regular files at or below `path` of the container `name`, created, running or ended (until
    /// [`remove`](Self::remove)): every file below a folder, or the one file `path` names. Each kept file is whole,
    /// with its absolute path, contents and permission bits, so it can go to [`copy_in`](Self::copy_in) of another
    /// container. For a host that collects a run's artifacts after it ended, as the container actor with a store does,
    /// or reads a small progress file while it runs. The files go to a store, not into actor state; called in a turn,
    /// a long copy needs the job's [`MAX_TURN`](crate::ContainerJob::MAX_TURN).
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
    /// The default fails with `Unsupported`, so a runtime written before this method compiles unchanged; the fake and
    /// the Docker backend implement it.
    async fn copy_out(&self, name: &str, path: &str, limits: CopyLimits) -> Result<CopyOut, ContainerError> {
        let _ = (path, limits);
        Err(ContainerError::new(
            ErrorKind::Unsupported,
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

    /// The processor kind and operating system of the runtime, which also shows that it answers. It changes nothing
    /// and is safe to repeat. A runtime that cannot be reached fails with `DaemonUnavailable`.
    ///
    /// Docker answers with the fields `Arch` and `Os` of its `GET /version`, as they are: Go's names, the spelling of
    /// image platforms (`amd64`, `arm64`, `arm`; `linux`), so they compare with an image's platform as they are. Docker
    /// gives no variant: `arm` does not say `v6` or `v7`. A missing or empty field is `Other`.
    ///
    /// The call has no time limit of its own: Docker's client gives up after 120 seconds. A host that wants a faster
    /// answer wraps the call in `tokio::time::timeout`. A Docker that answers but is not healthy fails with `Other`,
    /// as for every call: a 5xx, such as a Docker Desktop engine that is starting, and a Docker below the API floor
    /// (Docker 25). So a liveness check treats every error but `Unsupported` as "not here this round".
    ///
    /// The default fails with `Unsupported`, so a runtime written before this method compiles unchanged; the fake and
    /// the Docker backend implement it.
    async fn info(&self) -> Result<RuntimeInfo, ContainerError> {
        Err(ContainerError::new(ErrorKind::Unsupported, "runtime info: info is not supported by this runtime"))
    }
}

/// What [`info`](ContainerRuntime::info) reports: the runtime's processor kind and operating system, in the spelling
/// of image platforms.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInfo {
    /// The processor kind, such as `amd64` or `arm64`; no variant.
    pub arch: String,
    /// The operating system, such as `linux`.
    pub os: String,
}

impl RuntimeInfo {
    /// A runtime on the processor kind `arch` and the operating system `os`.
    pub fn new(arch: impl Into<String>, os: impl Into<String>) -> Self {
        Self { arch: arch.into(), os: os.into() }
    }
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
        /// Why: exactly [`OUT_OF_MEMORY`] for a container killed for lack of memory.
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
