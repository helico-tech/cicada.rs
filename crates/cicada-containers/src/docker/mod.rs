//! [`DockerRuntime`]: the [`ContainerRuntime`] over the Docker Engine API (feature `docker`).

mod map;

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use bollard::errors::Error as DockerError;
use bollard::models::{ContainerInspectResponse, NetworkCreateRequest, VolumeCreateRequest};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, InspectContainerOptions, InspectNetworkOptions,
    ListContainersOptionsBuilder, ListNetworksOptionsBuilder, ListVolumesOptionsBuilder, LogsOptionsBuilder,
    RemoveContainerOptionsBuilder, RemoveVolumeOptions, StartContainerOptions, StopContainerOptionsBuilder,
    UploadToContainerOptionsBuilder,
};
use bollard::{ClientVersion, Docker};
use futures::StreamExt;
use tokio::sync::OnceCell;

use self::map::{fail, kind_of, message_of, status_code};
use crate::{
    ContainerError, ContainerRuntime, ContainerSpec, ContainerStatus, ContainerSummary, CopyFile, ErrorKind, Label,
    NetworkSpec, Resource, Skipped, Swept, VolumeSpec,
};

/// The oldest Docker Engine API the backend speaks: Docker 25, which joins several networks in one create.
const API_FLOOR: ClientVersion = ClientVersion { major_version: 1, minor_version: 44 };

/// How much longer than the grace period a stop may take before the request gives up.
const STOP_SLACK: Duration = Duration::from_secs(60);

/// A [`ContainerRuntime`] over the Docker Engine API, by [bollard](https://docs.rs/bollard), without TLS.
///
/// [`connect`](Self::connect) reads `DOCKER_HOST` (`unix://`, `tcp://` or `http://`) and uses the local socket when it
/// is not set; Docker CLI contexts are not read. It needs Docker 25 or later (API 1.44) and the host's Tokio runtime.
///
/// - Containers are found by their exact name: Docker would also take an ID prefix, so every call inspects first,
///   treats another name as missing and acts on the full ID it found.
/// - [`create`](ContainerRuntime::create) pulls a missing image without a registry login, so a private image must be
///   local. An existing container is kept, even when its image is gone.
/// - [`stop`](ContainerRuntime::stop) sends the image's stop signal, as `docker stop` does: SIGTERM (exit code 143)
///   unless the image names another, such as SIGINT (130) for `postgres`.
/// - [`copy_in`](ContainerRuntime::copy_in) writes with a read-only root only below a volume or bind mount. It never
///   writes at or below a tmpfs, which Docker mounts only at start, over the file, and never onto a mount target.
/// - Secret environment goes to Docker only in the body of the create call, and an error message never holds a secret
///   value. The backend logs nothing; bollard logs request bodies at `debug` under the target `bollard`, so keep that
///   target at `info` or above.
pub struct DockerRuntime {
    client: Docker,
    negotiated: OnceCell<Docker>,
}

impl fmt::Debug for DockerRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockerRuntime").field("negotiated", &self.negotiated.initialized()).finish_non_exhaustive()
    }
}

impl DockerRuntime {
    /// A runtime for the daemon at `DOCKER_HOST`, or at the local socket when that is not set. It does not contact
    /// the daemon: the API version is agreed on the first call, so a host starts while the daemon is down and its
    /// calls fail with the retryable `DaemonUnavailable` until it is up. Fails with `DaemonUnavailable` when the
    /// socket file does not exist, and with `Other` for a `DOCKER_HOST` scheme it does not support (`https`, `ssh`).
    pub fn connect() -> Result<Self, ContainerError> {
        let client = Docker::connect_with_defaults().map_err(|e| fail("connect to", "docker", &e))?;
        Ok(Self::with_client(client))
    }

    /// A runtime for the daemon at `host`, read as `DOCKER_HOST` would be: `unix:///path/docker.sock`,
    /// `tcp://host:port` or `http://host:port`. For a host that finds its daemon another way, such as the endpoint of
    /// the current Docker CLI context; otherwise as [`connect`](Self::connect), which it does not change.
    pub fn connect_to(host: &str) -> Result<Self, ContainerError> {
        let client = Docker::connect_with_host(host).map_err(|e| fail("connect to", host, &e))?;
        Ok(Self::with_client(client))
    }

    fn with_client(client: Docker) -> Self {
        Self { client, negotiated: OnceCell::new() }
    }

    /// The client at the agreed API version; a failed agreement is not kept, so the next call tries again.
    async fn docker(&self) -> Result<&Docker, ContainerError> {
        self.negotiated
            .get_or_try_init(|| async {
                let docker =
                    self.client.clone().negotiate_version().await.map_err(|e| fail("connect to", "docker", &e))?;
                let version = docker.client_version();
                if version < API_FLOOR {
                    return Err(ContainerError::new(
                        ErrorKind::Other,
                        format!("connect to docker: API {version} is older than {API_FLOOR} (Docker 25)"),
                    ));
                }
                Ok(docker)
            })
            .await
    }

    /// The container whose name is exactly `name`, with its full ID; `None` when there is none.
    async fn find(&self, docker: &Docker, name: &str) -> Result<Option<Found>, ContainerError> {
        match docker.inspect_container(name, None::<InspectContainerOptions>).await {
            Ok(info) if info.name.as_deref().and_then(|n| n.strip_prefix('/')) == Some(name) => {
                Ok(info.id.clone().map(|id| Found { id, info }))
            }
            Ok(_) => Ok(None),
            Err(e) if status_code(&e) == Some(404) => Ok(None),
            Err(e) => Err(fail("inspect", name, &e)),
        }
    }

    /// The status of the found container, or `Other` for a state Docker did not report.
    fn status(name: &str, found: &Found) -> Result<ContainerStatus, ContainerError> {
        found
            .info
            .state
            .as_ref()
            .and_then(map::status_of)
            .ok_or_else(|| ContainerError::new(ErrorKind::Other, format!("inspect {name}: Docker reports no state")))
    }

    /// Pulls `image` unless Docker has it: `ImageNotFound` when the registry does not.
    async fn pull_if_missing(&self, docker: &Docker, image: &str) -> Result<(), ContainerError> {
        match docker.inspect_image(image).await {
            Ok(_) => return Ok(()),
            Err(e) if status_code(&e) == Some(404) => {}
            Err(e) => return Err(fail("inspect image", image, &e)),
        }
        let reference = map::pull_reference(image);
        let options = CreateImageOptionsBuilder::new().from_image(&reference).build();
        let mut pull = docker.create_image(Some(options), None, None);
        while let Some(progress) = pull.next().await {
            if let Err(e) = progress {
                let kind = if map::image_unknown(&e) { ErrorKind::ImageNotFound } else { kind_of(&e) };
                return Err(ContainerError::new(kind, format!("pull {image}: {}", message_of(&e))));
            }
        }
        Ok(())
    }

    async fn create_unscrubbed(&self, spec: &ContainerSpec) -> Result<(), ContainerError> {
        let name = spec.name.as_str();
        let docker = self.docker().await?;
        if self.find(docker, name).await?.is_some() {
            return Ok(());
        }
        if let Some(bad) = map::bad_env_name(spec) {
            return Err(ContainerError::new(
                ErrorKind::InvalidSpec,
                format!("create {name}: environment variable name {bad:?} is empty or holds `=`"),
            ));
        }
        self.pull_if_missing(docker, &spec.image).await?;
        let options = CreateContainerOptionsBuilder::new().name(name).build();
        match docker.create_container(Some(options), map::create_body(spec)).await {
            Ok(_) => Ok(()),
            Err(e) if status_code(&e) == Some(409) => Ok(()), // created by someone else since the inspect
            Err(e) => Err(fail("create", name, &e)),
        }
    }

    /// Removes the container `id`, forced, with its anonymous volumes; gone already or being removed is `Ok`.
    async fn remove_id(docker: &Docker, name: &str, id: &str) -> Result<(), ContainerError> {
        let options = RemoveContainerOptionsBuilder::new().force(true).v(true).build();
        match docker.remove_container(id, Some(options)).await {
            Ok(()) => Ok(()),
            Err(e) if matches!(status_code(&e), Some(404 | 409)) => Ok(()),
            Err(e) => Err(fail("remove", name, &e)),
        }
    }
}

/// A container found by its exact name.
struct Found {
    id: String,
    info: ContainerInspectResponse,
}

fn conflict(action: &str, name: &str) -> ContainerError {
    ContainerError::new(ErrorKind::Conflict, format!("{action} {name}: no container of that name"))
}

fn label_filter(label: &Label) -> HashMap<&'static str, Vec<String>> {
    HashMap::from([("label", vec![format!("{}={}", label.key, label.value)])])
}

fn labels(map: &std::collections::BTreeMap<String, String>) -> HashMap<String, String> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Sorts a refusal of a sweep: `DaemonUnavailable` ends the sweep, anything else is a skip with Docker's message.
fn skip_or_end(swept: &mut Swept, resource: Resource, name: &str, error: &DockerError) -> Result<(), ContainerError> {
    if kind_of(error) == ErrorKind::DaemonUnavailable {
        return Err(fail("sweep", name, error));
    }
    swept.skipped.push(Skipped::new(resource, name, message_of(error)));
    Ok(())
}

#[async_trait]
impl ContainerRuntime for DockerRuntime {
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError> {
        self.create_unscrubbed(spec)
            .await
            .map_err(|error| ContainerError::new(error.kind, map::scrub(&error.message, spec)))
    }

    async fn copy_in(&self, name: &str, files: &[CopyFile]) -> Result<(), ContainerError> {
        let docker = self.docker().await?;
        let found = self.find(docker, name).await?.ok_or_else(|| conflict("copy into", name))?;
        let status = Self::status(name, &found)?;
        if status != ContainerStatus::Created {
            return Err(ContainerError::new(
                ErrorKind::Conflict,
                format!("copy into {name}: the container is {status:?}, not created"),
            ));
        }
        let host_config = found.info.host_config.as_ref();
        let read_only_root = host_config.and_then(|h| h.readonly_rootfs).unwrap_or(false);
        let tmpfs = host_config.and_then(|h| h.tmpfs.as_ref());
        let mounts = map::mount_targets(found.info.mounts.as_deref().unwrap_or_default(), tmpfs);
        for upload in map::uploads(name, files, read_only_root, &mounts)? {
            let options = UploadToContainerOptionsBuilder::new().path(&upload.dir).build();
            let body = bollard::body_full(upload.tar.into());
            docker
                .upload_to_container(&found.id, Some(options), body)
                .await
                .map_err(|e| fail("copy into", name, &e))?;
        }
        Ok(())
    }

    async fn start(&self, name: &str) -> Result<(), ContainerError> {
        let docker = self.docker().await?;
        let found = self.find(docker, name).await?.ok_or_else(|| conflict("start", name))?;
        if Self::status(name, &found)? != ContainerStatus::Created {
            return Ok(()); // running or ended: a container runs at most once
        }
        match docker.start_container(&found.id, None::<StartContainerOptions>).await {
            Ok(()) => Ok(()),
            Err(e) if status_code(&e) == Some(304) => Ok(()), // started by someone else since the inspect
            Err(e) => Err(fail("start", name, &e)),
        }
    }

    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError> {
        let docker = self.docker().await?;
        match self.find(docker, name).await? {
            Some(found) => Self::status(name, &found),
            None => Ok(ContainerStatus::Missing),
        }
    }

    async fn logs(&self, name: &str, tail_lines: u32) -> Result<String, ContainerError> {
        let docker = self.docker().await?;
        let found = self.find(docker, name).await?.ok_or_else(|| conflict("logs of", name))?;
        let tail = tail_lines.to_string();
        let options = LogsOptionsBuilder::new().follow(false).stdout(true).stderr(true).tail(&tail).build();
        let mut stream = docker.logs(&found.id, Some(options));
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => bytes.extend_from_slice(chunk.as_ref()),
                Err(e) if status_code(&e) == Some(404) => return Err(conflict("logs of", name)),
                Err(e) => return Err(fail("logs of", name, &e)),
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn stop(&self, name: &str, grace: Duration) -> Result<(), ContainerError> {
        let docker = self.docker().await?;
        let Some(found) = self.find(docker, name).await? else { return Ok(()) };
        if Self::status(name, &found)? != ContainerStatus::Running {
            return Ok(()); // created stays created, an ended one keeps its end
        }
        let seconds = grace.as_secs() + u64::from(grace.subsec_nanos() > 0);
        let options = StopContainerOptionsBuilder::new().t(i32::try_from(seconds).unwrap_or(i32::MAX)).build();
        let docker = docker.clone().with_timeout(grace.saturating_add(STOP_SLACK));
        match docker.stop_container(&found.id, Some(options)).await {
            Ok(()) => Ok(()),
            Err(e) if matches!(status_code(&e), Some(304 | 404)) => Ok(()), // stopped or removed since the inspect
            Err(e) => Err(fail("stop", name, &e)),
        }
    }

    async fn remove(&self, name: &str) -> Result<(), ContainerError> {
        let docker = self.docker().await?;
        match self.find(docker, name).await? {
            Some(found) => Self::remove_id(docker, name, &found.id).await,
            None => Ok(()),
        }
    }

    async fn list(&self, label: &Label) -> Result<Vec<ContainerSummary>, ContainerError> {
        let docker = self.docker().await?;
        let filter = label_filter(label);
        let options = ListContainersOptionsBuilder::new().all(true).filters(&filter).build();
        let what = format!("{}={}", label.key, label.value);
        let listed = docker.list_containers(Some(options)).await.map_err(|e| fail("list", &what, &e))?;
        let mut summaries = Vec::new();
        for id in listed.into_iter().filter_map(|c| c.id) {
            let info = match docker.inspect_container(&id, None::<InspectContainerOptions>).await {
                Ok(info) => info,
                Err(e) if status_code(&e) == Some(404) => continue, // removed since the list
                Err(e) => return Err(fail("inspect", &id, &e)),
            };
            let name = info.name.as_deref().unwrap_or_default().trim_start_matches('/').to_owned();
            let status = Self::status(&name, &Found { id, info })?;
            summaries.push(ContainerSummary::new(name, status));
        }
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(summaries)
    }

    async fn ensure_network(&self, spec: &NetworkSpec) -> Result<(), ContainerError> {
        let docker = self.docker().await?;
        let name = spec.name.as_str();
        // The internal flag of the network named exactly `name`; `None` when there is none.
        let existing = || async {
            match docker.inspect_network(name, None::<InspectNetworkOptions>).await {
                Ok(network) if network.name.as_deref() == Some(name) => Ok(Some(network.internal.unwrap_or(false))),
                Ok(_) => Ok(None),
                Err(e) if status_code(&e) == Some(404) => Ok(None),
                Err(e) => Err(fail("inspect network", name, &e)),
            }
        };
        let compare = |internal: bool| {
            if internal == spec.internal {
                Ok(())
            } else {
                Err(ContainerError::new(
                    ErrorKind::Conflict,
                    format!("ensure network {name}: it exists with internal = {internal}, not {}", spec.internal),
                ))
            }
        };
        if let Some(internal) = existing().await? {
            return compare(internal);
        }
        let request = NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            internal: Some(spec.internal),
            labels: Some(labels(&spec.labels)),
            ..Default::default()
        };
        match docker.create_network(request).await {
            Ok(_) => Ok(()),
            Err(e) if status_code(&e) == Some(409) => match existing().await? {
                Some(internal) => compare(internal),
                None => Err(fail("create network", name, &e)),
            },
            Err(e) => Err(fail("create network", name, &e)),
        }
    }

    async fn ensure_volume(&self, spec: &VolumeSpec) -> Result<(), ContainerError> {
        let docker = self.docker().await?;
        // Docker returns an existing volume of that name unchanged.
        let request = VolumeCreateRequest {
            name: Some(spec.name.clone()),
            labels: Some(labels(&spec.labels)),
            ..Default::default()
        };
        docker.create_volume(request).await.map(drop).map_err(|e| fail("ensure volume", &spec.name, &e))
    }

    async fn sweep(&self, label: &Label) -> Result<Swept, ContainerError> {
        let docker = self.docker().await?;
        let filter = label_filter(label);
        let what = format!("{}={}", label.key, label.value);
        let mut swept = Swept::default();

        let options = ListContainersOptionsBuilder::new().all(true).filters(&filter).build();
        let containers = docker.list_containers(Some(options)).await.map_err(|e| fail("sweep", &what, &e))?;
        for container in containers {
            let Some(id) = container.id else { continue };
            let name = container.names.and_then(|n| n.into_iter().next()).unwrap_or_else(|| id.clone());
            let name = name.trim_start_matches('/').to_owned();
            let options = RemoveContainerOptionsBuilder::new().force(true).v(true).build();
            match docker.remove_container(&id, Some(options)).await {
                Ok(()) => swept.containers.push(name),
                Err(e) if map::gone(Resource::Container, &e) => {} // gone, or another caller removes it
                Err(e) => skip_or_end(&mut swept, Resource::Container, &name, &e)?,
            }
        }

        let options = ListNetworksOptionsBuilder::new().filters(&filter).build();
        let networks = docker.list_networks(Some(options)).await.map_err(|e| fail("sweep", &what, &e))?;
        for network in networks {
            let (Some(id), Some(name)) = (network.id, network.name) else { continue };
            match docker.remove_network(&id).await {
                Ok(()) => swept.networks.push(name),
                Err(e) if map::gone(Resource::Network, &e) => {}
                Err(e) => skip_or_end(&mut swept, Resource::Network, &name, &e)?,
            }
        }

        let options = ListVolumesOptionsBuilder::new().filters(&filter).build();
        let volumes = docker.list_volumes(Some(options)).await.map_err(|e| fail("sweep", &what, &e))?;
        for volume in volumes.volumes.unwrap_or_default() {
            match docker.remove_volume(&volume.name, None::<RemoveVolumeOptions>).await {
                Ok(()) => swept.volumes.push(volume.name),
                Err(e) if map::gone(Resource::Volume, &e) => {}
                Err(e) => skip_or_end(&mut swept, Resource::Volume, &volume.name, &e)?,
            }
        }
        swept.containers.sort();
        swept.networks.sort();
        swept.volumes.sort();
        Ok(swept)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A runtime on a socket file with no daemon behind it: bound once, then closed. `tag` keeps tests apart.
    fn dead_socket(tag: &str) -> (std::path::PathBuf, DockerRuntime) {
        let path = std::env::temp_dir().join(format!("cicada-ctr-{}-{tag}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        let runtime = DockerRuntime::connect_to(&format!("unix://{}", path.display())).unwrap();
        (path, runtime)
    }

    #[test]
    fn connect_to_refuses_a_missing_socket_and_an_unsupported_scheme() {
        let missing = DockerRuntime::connect_to("unix:///nonexistent/cicada/docker.sock").unwrap_err();
        assert_eq!(missing.kind, ErrorKind::DaemonUnavailable, "{missing}");
        for host in ["https://docker.example:2376", "ssh://user@docker.example"] {
            assert_eq!(DockerRuntime::connect_to(host).unwrap_err().kind, ErrorKind::Other, "{host}");
        }
        DockerRuntime::connect_to("tcp://127.0.0.1:1").unwrap();
    }

    #[tokio::test]
    async fn a_socket_without_a_daemon_is_daemon_unavailable_and_not_kept() {
        let (path, runtime) = dead_socket("inspect");
        for _ in 0..2 {
            let error = runtime.inspect("job-1").await.unwrap_err();
            assert_eq!(error.kind, ErrorKind::DaemonUnavailable, "{error}");
            assert!(error.is_retryable());
        }
        assert!(!runtime.negotiated.initialized(), "a failed agreement is not kept");
        let error = runtime.sweep(&Label::new("run", "a")).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::DaemonUnavailable);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn every_error_of_create_is_scrubbed() {
        let (path, runtime) = dead_socket("create");
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        // A secret that the message of the failed connection holds.
        spec.secret_env.insert("TOKEN".into(), "docker".into());
        let error = runtime.create(&spec).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::DaemonUnavailable);
        assert!(error.message.starts_with("connect to ***: "), "{error}");
        assert!(!error.message.contains("docker"), "{error}");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn debug_shows_no_client() {
        let (path, runtime) = dead_socket("debug");
        assert_eq!(format!("{runtime:?}"), "DockerRuntime { negotiated: false, .. }");
        std::fs::remove_file(path).unwrap();
    }
}
