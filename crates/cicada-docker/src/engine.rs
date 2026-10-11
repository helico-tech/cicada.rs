use std::collections::BTreeMap;

use async_trait::async_trait;
use axum::body::Bytes;
use cicada::time::Timestamp;
use futures::stream::BoxStream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::spec::ContainerSpec;

/// The container operations a [`crate::ContainerActor`] needs, by container name. [`crate::DockerEngine`] implements it
/// on a Docker daemon; tests implement it with a fake.
///
/// Every operation may be repeated by a turn that runs again (G4), so each must be harmless when repeated in the state
/// it leaves: `stop` of a container that is not running and `remove` of an absent one succeed.
#[async_trait]
pub trait Engine: Send + Sync + 'static {
    /// The container `name`, or `None` when there is none.
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, EngineError>;

    /// Whether the daemon has `image`.
    async fn image_present(&self, image: &str) -> Result<bool, EngineError>;

    /// Pulls `image`, returning when the pull completes.
    async fn pull(&self, image: &str) -> Result<(), EngineError>;

    /// Creates (does not start) container `name` from `spec`, with `labels` added to the spec's; returns its id. Fails
    /// with [`EngineError::Conflict`] when the name is taken.
    async fn create(
        &self,
        name: &str,
        spec: &ContainerSpec,
        labels: &BTreeMap<String, String>,
    ) -> Result<String, EngineError>;

    /// Starts container `name`; a running container is left as it is.
    async fn start(&self, name: &str) -> Result<(), EngineError>;

    /// Stops container `name`, killing it after its stop timeout; a container that is not running is left as it is.
    async fn stop(&self, name: &str) -> Result<(), EngineError>;

    /// Removes container `name`, stopping it first; an absent container is not an error.
    async fn remove(&self, name: &str) -> Result<(), EngineError>;

    /// The output of container `name` (stdout and stderr, interleaved) as raw chunks.
    fn logs(&self, name: &str, options: LogOptions) -> BoxStream<'static, Result<Bytes, EngineError>>;
}

/// A container as the engine reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInfo {
    /// Docker's container id.
    pub id: String,
    /// Its status.
    pub status: ContainerStatus,
    /// Its labels.
    pub labels: BTreeMap<String, String>,
    /// Exit code of the last run; meaningful once it has exited.
    pub exit_code: Option<i64>,
    /// Docker's error text for the last run, if any.
    pub error: Option<String>,
    /// Whether the last run was killed for running out of memory.
    pub oom_killed: bool,
    /// When the last run started.
    pub started_at: Option<Timestamp>,
    /// When the last run ended.
    pub finished_at: Option<Timestamp>,
}

/// Status of a container; [`Self::Missing`] when the handle's container does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContainerStatus {
    /// Created, never started.
    Created,
    /// Running.
    Running,
    /// Paused.
    Paused,
    /// Being restarted by its restart policy.
    Restarting,
    /// Being removed.
    Removing,
    /// Exited; see the exit code.
    Exited,
    /// Could not be stopped or removed cleanly.
    Dead,
    /// Being stopped.
    Stopping,
    /// Docker reported no status.
    Unknown,
    /// There is no container.
    Missing,
}

impl ContainerStatus {
    /// Whether the container is running or about to (`running`, `restarting`).
    pub fn is_up(self) -> bool {
        matches!(self, Self::Running | Self::Restarting)
    }
}

/// What [`Engine::logs`] returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogOptions {
    /// Only the last `tail` lines; all when `None`.
    pub tail: Option<u32>,
    /// Keep the stream open and return new output as it comes.
    pub follow: bool,
    /// Only output since this time.
    pub since: Option<Timestamp>,
    /// Prefix every line with its time.
    pub timestamps: bool,
}

/// An engine failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// The container or image does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The name is taken, or the operation conflicts with the container's status.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Any other failure, including an unreachable daemon.
    #[error("{0}")]
    Other(String),
}
