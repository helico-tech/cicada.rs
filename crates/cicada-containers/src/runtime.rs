//! The [`ContainerRuntime`] trait and what it reports.

use std::time::Duration;

use async_trait::async_trait;

use crate::{ContainerError, ContainerSpec, CopyFile, Label, NetworkSpec, VolumeSpec};

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
    /// missing, running or ended, and with `InvalidSpec` for a relative path or, with
    /// [`read_only_root`](ContainerSpec::read_only_root), a path on no volume or bind mount.
    async fn copy_in(&self, name: &str, files: &[CopyFile]) -> Result<(), ContainerError>;

    /// Starts the created container `name`. For a running, exited or failed one it is `Ok` and does nothing, so a
    /// container runs at most once. Fails with `Conflict` for a missing one.
    async fn start(&self, name: &str) -> Result<(), ContainerError>;

    /// The status of the container `name`; [`ContainerStatus::Missing`] when there is none, not an error.
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError>;

    /// The last `tail_lines` lines of the container's stdout and stderr together, as lossy UTF-8. Fails with
    /// `Conflict` for a missing container.
    async fn logs(&self, name: &str, tail_lines: u32) -> Result<String, ContainerError>;

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
    /// sweep finishes the work. `Ok` with nothing removed when nothing carries the label.
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

/// What a [`sweep`](ContainerRuntime::sweep) removed, per kind, and what it skipped.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// The names of the removed containers.
    pub containers: Vec<String>,
    /// The names of the removed networks.
    pub networks: Vec<String>,
    /// The names of the removed volumes.
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
