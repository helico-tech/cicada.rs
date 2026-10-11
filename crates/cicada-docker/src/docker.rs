use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use axum::body::Bytes;
use bollard::Docker;
use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerCreateBody, ContainerStateStatusEnum, HostConfig, PortBinding as DockerPortBinding,
    RestartPolicy as DockerRestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, LogsOptions, RemoveContainerOptions, StopContainerOptions,
};
use cicada::time::Timestamp;
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};

use crate::engine::{ContainerInfo, ContainerStatus, Engine, EngineError, LogOptions};
use crate::spec::{ContainerSpec, RestartPolicy};

/// An [`Engine`] on a Docker daemon, through [bollard](https://docs.rs/bollard).
///
/// ```no_run
/// # fn f() -> Result<(), bollard::errors::Error> {
/// use cicada_docker::DockerEngine;
///
/// let engine = DockerEngine::connect()?; // DOCKER_HOST, else the local socket
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct DockerEngine {
    docker: Docker,
}

impl DockerEngine {
    /// Connects with Docker's defaults: `DOCKER_HOST` when set, else the local socket (or named pipe on Windows).
    /// The connection is lazy; a daemon that is down shows up at the first call.
    pub fn connect() -> Result<Self, BollardError> {
        Ok(Self::new(Docker::connect_with_defaults()?))
    }

    /// An engine on a configured bollard client.
    pub fn new(docker: Docker) -> Self {
        Self { docker }
    }

    /// The bollard client.
    pub fn client(&self) -> &Docker {
        &self.docker
    }
}

#[async_trait]
impl Engine for DockerEngine {
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, EngineError> {
        let response = match self.docker.inspect_container(name, None).await {
            Ok(response) => response,
            Err(e) if status_of(&e) == Some(404) => return Ok(None),
            Err(e) => return Err(map_error(e)),
        };
        let state = response.state.unwrap_or_default();
        Ok(Some(ContainerInfo {
            id: response.id.unwrap_or_default(),
            status: match state.status {
                Some(ContainerStateStatusEnum::CREATED) => ContainerStatus::Created,
                Some(ContainerStateStatusEnum::RUNNING) => ContainerStatus::Running,
                Some(ContainerStateStatusEnum::PAUSED) => ContainerStatus::Paused,
                Some(ContainerStateStatusEnum::RESTARTING) => ContainerStatus::Restarting,
                Some(ContainerStateStatusEnum::REMOVING) => ContainerStatus::Removing,
                Some(ContainerStateStatusEnum::EXITED) => ContainerStatus::Exited,
                Some(ContainerStateStatusEnum::DEAD) => ContainerStatus::Dead,
                Some(ContainerStateStatusEnum::STOPPING) => ContainerStatus::Stopping,
                _ => ContainerStatus::Unknown,
            },
            labels: response.config.and_then(|c| c.labels).unwrap_or_default().into_iter().collect(),
            exit_code: state.exit_code,
            error: state.error.filter(|e| !e.is_empty()),
            oom_killed: state.oom_killed.unwrap_or(false),
            started_at: state.started_at.as_deref().and_then(docker_time),
            finished_at: state.finished_at.as_deref().and_then(docker_time),
        }))
    }

    async fn image_present(&self, image: &str) -> Result<bool, EngineError> {
        match self.docker.inspect_image(image).await {
            Ok(_) => Ok(true),
            Err(e) if status_of(&e) == Some(404) => Ok(false),
            Err(e) => Err(map_error(e)),
        }
    }

    async fn pull(&self, image: &str) -> Result<(), EngineError> {
        // Without a tag Docker pulls every tag of the repository; pull `latest`, as `docker pull` does.
        let (from_image, tag) = match image.contains('@') || image.rsplit('/').next().unwrap_or(image).contains(':') {
            true => (image.to_owned(), None),
            false => (image.to_owned(), Some("latest".to_owned())),
        };
        let options = CreateImageOptions { from_image: Some(from_image), tag, ..Default::default() };
        let mut progress = self.docker.create_image(Some(options), None, None);
        while let Some(step) = progress.next().await {
            let step = step.map_err(map_error)?;
            if let Some(error) = step.error_detail.and_then(|d| d.message) {
                return Err(EngineError::Other(format!("pull of {image} failed: {error}")));
            }
        }
        Ok(())
    }

    async fn create(
        &self,
        name: &str,
        spec: &ContainerSpec,
        labels: &BTreeMap<String, String>,
    ) -> Result<String, EngineError> {
        let mut all_labels: HashMap<String, String> = spec.labels.clone().into_iter().collect();
        all_labels.extend(labels.clone());
        let mut port_bindings = HashMap::new();
        for port in &spec.ports {
            let host_port = port.host_port.map(|p| p.to_string());
            port_bindings
                .entry(format!("{}/{}", port.container_port, port.protocol.as_str()))
                .or_insert_with(|| Some(Vec::new()))
                .get_or_insert_with(Vec::new)
                .push(DockerPortBinding { host_ip: None, host_port });
        }
        let body = ContainerCreateBody {
            image: Some(spec.image.clone()),
            cmd: spec.cmd.clone(),
            entrypoint: spec.entrypoint.clone(),
            env: Some(spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
            labels: Some(all_labels),
            exposed_ports: Some(port_bindings.keys().cloned().collect()),
            stop_timeout: spec.stop_timeout_seconds.map(i64::from),
            host_config: Some(HostConfig {
                port_bindings: Some(port_bindings),
                binds: (!spec.binds.is_empty()).then(|| spec.binds.clone()),
                network_mode: spec.network.clone(),
                restart_policy: Some(restart_policy(spec.restart)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let options = CreateContainerOptions { name: Some(name.to_owned()), ..Default::default() };
        let created = self.docker.create_container(Some(options), body).await.map_err(map_error)?;
        Ok(created.id)
    }

    async fn start(&self, name: &str) -> Result<(), EngineError> {
        match self.docker.start_container(name, None).await {
            Err(e) if status_of(&e) == Some(304) => Ok(()),
            result => result.map_err(map_error),
        }
    }

    async fn stop(&self, name: &str) -> Result<(), EngineError> {
        // No `t`: Docker uses the container's own stop timeout (the spec's `stopTimeoutSeconds`).
        match self.docker.stop_container(name, None::<StopContainerOptions>).await {
            Err(e) if status_of(&e) == Some(304) => Ok(()),
            result => result.map_err(map_error),
        }
    }

    async fn remove(&self, name: &str) -> Result<(), EngineError> {
        let options = RemoveContainerOptions { force: true, ..Default::default() };
        match self.docker.remove_container(name, Some(options)).await {
            Err(e) if status_of(&e) == Some(404) => Ok(()),
            result => result.map_err(map_error),
        }
    }

    fn logs(&self, name: &str, options: LogOptions) -> BoxStream<'static, Result<Bytes, EngineError>> {
        let options = LogsOptions {
            follow: options.follow,
            stdout: true,
            stderr: true,
            since: options.since.map_or(0, |t| i32::try_from(t.timestamp()).unwrap_or(i32::MAX)),
            until: 0,
            timestamps: options.timestamps,
            tail: options.tail.map_or_else(|| "all".to_owned(), |n| n.to_string()),
        };
        self.docker.logs(name, Some(options)).map_ok(|chunk| chunk.into_bytes()).map_err(map_error).boxed()
    }
}

fn restart_policy(policy: RestartPolicy) -> DockerRestartPolicy {
    let (name, maximum_retry_count) = match policy {
        RestartPolicy::No => (RestartPolicyNameEnum::NO, None),
        RestartPolicy::OnFailure { max_retries } => (RestartPolicyNameEnum::ON_FAILURE, max_retries.map(i64::from)),
        RestartPolicy::UnlessStopped => (RestartPolicyNameEnum::UNLESS_STOPPED, None),
        RestartPolicy::Always => (RestartPolicyNameEnum::ALWAYS, None),
    };
    DockerRestartPolicy { name: Some(name), maximum_retry_count }
}

/// A Docker time, or `None` for Docker's "never" (`0001-01-01T00:00:00Z`).
fn docker_time(text: &str) -> Option<Timestamp> {
    let time = chrono::DateTime::parse_from_rfc3339(text).ok()?.to_utc();
    (time.timestamp() > 0).then_some(time)
}

fn status_of(error: &BollardError) -> Option<u16> {
    match error {
        BollardError::DockerResponseServerError { status_code, .. } => Some(*status_code),
        _ => None,
    }
}

fn map_error(error: BollardError) -> EngineError {
    match &error {
        BollardError::DockerResponseServerError { status_code: 404, message } => EngineError::NotFound(message.clone()),
        BollardError::DockerResponseServerError { status_code: 409, message } => EngineError::Conflict(message.clone()),
        _ => EngineError::Other(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_never_is_none() {
        assert_eq!(docker_time("0001-01-01T00:00:00Z"), None);
        assert_eq!(
            docker_time("2026-10-11T09:30:00.123456789Z").unwrap().to_rfc3339(),
            "2026-10-11T09:30:00.123456789+00:00"
        );
    }
}
