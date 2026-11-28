//! [`DockerRuntime`]: the [`ContainerRuntime`] over the Docker Engine API (feature `docker`).

mod archive;
mod auth;
mod context;
mod map;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use bollard::errors::Error as DockerError;
use bollard::models::{ContainerInspectResponse, VolumeCreateRequest};
use bollard::query_parameters::{
    ContainerArchiveInfoOptionsBuilder, CreateContainerOptionsBuilder, CreateImageOptionsBuilder,
    DownloadFromContainerOptionsBuilder, InspectContainerOptions, InspectNetworkOptions, ListContainersOptionsBuilder,
    ListNetworksOptionsBuilder, ListVolumesOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveVolumeOptions, StartContainerOptions, StopContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use bollard::{ClientVersion, Docker};
use futures::StreamExt;
use tokio::sync::OnceCell;

use self::map::{Upload, fail, kind_of, message_of, status_code};
use crate::copy::Collector;
use crate::dirs::{self, MAKE, Stat, ToMake};
use crate::logs::{Pager, check_max_lines};
use crate::paths::MountTarget;
use crate::{
    ContainerError, ContainerRuntime, ContainerSpec, ContainerStatus, ContainerSummary, CopyDir, CopyFile, CopyLimits,
    CopyOut, ErrorKind, Label, LogCursor, LogPage, NetworkSpec, Resource, RuntimeInfo, Skipped, Swept, VolumeSpec,
};

/// The oldest Docker Engine API the backend speaks: Docker 25, which joins several networks in one create.
const API_FLOOR: ClientVersion = ClientVersion { major_version: 1, minor_version: 44 };

/// How much longer than the grace period a stop may take before the request gives up.
const STOP_SLACK: Duration = Duration::from_secs(60);

/// A [`ContainerRuntime`] over the Docker Engine API, by [bollard](https://docs.rs/bollard), without TLS.
///
/// [`connect`](Self::connect) finds the daemon as the `docker` CLI does, in this order: `DOCKER_HOST`; else the CLI
/// context `DOCKER_CONTEXT` names; else the local socket `/var/run/docker.sock` when it exists; else the
/// `currentContext` of the CLI's `config.json`. A context's endpoint must be `unix://`, `tcp://` or `http://` without
/// TLS; other setups use [`connect_to`](Self::connect_to). It needs Docker 25 or later (API 1.44) and the host's Tokio
/// runtime.
///
/// - Containers are found by their exact name: Docker would also take an ID prefix, so every call inspects first,
///   treats another name as missing and acts on the full ID it found.
/// - [`create`](ContainerRuntime::create) pulls a missing image with the machine's `docker login`, read as the CLI
///   reads it at each pull: `config.json` in the folder set with [`docker_config`](Self::docker_config), else in
///   `DOCKER_CONFIG`, else in `$HOME/.docker`. For the image's registry (Docker Hub when its first part names no host)
///   it takes the helper `credHelpers` names, else the `credsStore` helper, else the `auths` entry. A helper runs as
///   `docker-credential-<name> get` from `PATH` and is killed after 10 s. A missing or broken file, a registry without
///   an entry or a failing helper gives a pull without credentials, so a public image still works. The message of a
///   failed pull ends with where its credentials came from, or why there were none, never with a credential. An
///   existing container is kept, even when its image is gone.
/// - [`list`](ContainerRuntime::list) leaves out a container removed since Docker listed it, and one whose state
///   Docker does not report; [`inspect`](ContainerRuntime::inspect), [`start`](ContainerRuntime::start),
///   [`copy_in`](ContainerRuntime::copy_in) and [`stop`](ContainerRuntime::stop) of such a name fail with `Other`.
/// - [`stop`](ContainerRuntime::stop) sends the image's stop signal, as `docker stop` does: SIGTERM (exit code 143)
///   unless the image names another, such as SIGINT (130) for `postgres`.
/// - [`copy_in`](ContainerRuntime::copy_in) writes with a read-only root only below a volume or bind mount. It never
///   writes at or below a tmpfs, which Docker mounts only at start, over the file, and never onto a mount target.
/// - [`make_dirs`](ContainerRuntime::make_dirs) asks Docker's archive stat for each part of a named folder from the
///   root, follows links itself, and uploads a tar of only the missing folders under their paths without links, owned
///   by uid and gid 0, so no existing folder's owner or mode changes. Docker's stat answers a link with its target
///   already resolved, so a chain of links counts as one; a link loop makes the stat fail, and the call with `Other`.
/// - [`logs_since`](ContainerRuntime::logs_since) reads Docker's log with timestamps from the cursor's second and
///   stops once the page is full. [`logs`](ContainerRuntime::logs) and `logs_since` of a container whose log driver
///   cannot be read (`none`; Docker's 501) fail with `Unsupported`.
/// - [`info`](ContainerRuntime::info) asks `GET /version` and gives its `Arch` and `Os` as they are, with bollard's
///   request timeout (120 s) and no limit of its own.
/// - [`copy_out`](ContainerRuntime::copy_out) reads Docker's archive of the path as a stream, on a blocking thread of
///   the host's Tokio runtime, and holds only the kept files. An error during the stream fails the whole call.
/// - Secret environment goes to Docker only in the body of the create call, and an error message never holds a secret
///   value. The backend logs nothing; bollard logs request bodies at `debug` under the target `bollard`, so keep that
///   target at `info` or above.
pub struct DockerRuntime {
    client: Docker,
    negotiated: OnceCell<Docker>,
    docker_config: Option<PathBuf>,
}

impl fmt::Debug for DockerRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockerRuntime").field("negotiated", &self.negotiated.initialized()).finish_non_exhaustive()
    }
}

impl DockerRuntime {
    /// A runtime for the daemon the `docker` CLI would use, found in this order:
    ///
    /// 1. `DOCKER_HOST`, when it is set.
    /// 2. The context `DOCKER_CONTEXT` names, when it is set and not empty, even when the local socket exists; the
    ///    context `default` is the local socket.
    /// 3. The local socket `/var/run/docker.sock`, when it exists.
    /// 4. The `currentContext` of `config.json` in `DOCKER_CONFIG`, else in `$HOME/.docker`; none, `default` or a
    ///    missing `config.json` is the local socket.
    ///
    /// A context's endpoint is `Endpoints.docker.Host` of `contexts/meta/<sha256 of its name>/meta.json` in that
    /// folder ([`docker_config`](Self::docker_config) does not apply: it is set after `connect`).
    ///
    /// It does not contact the daemon: the API version is agreed on the first call, so a host starts while the daemon
    /// is down and its calls fail with the retryable `DaemonUnavailable` until it is up. Fails with
    /// `DaemonUnavailable` when the socket file does not exist, naming the context and its endpoint for a context's.
    /// Fails with `Other` for a `DOCKER_HOST` scheme it does not support (`https`, `ssh`), and, naming the context and
    /// where its name came from, for a context it cannot read (no meta file, no docker endpoint, broken JSON, no config
    /// folder) or use: one with TLS (`SkipTLSVerify`, or TLS files under `contexts/tls`) or a scheme other than
    /// `unix`, `tcp` and `http`. It never falls back to the local socket then; set `DOCKER_HOST` or use
    /// [`connect_to`](Self::connect_to).
    pub fn connect() -> Result<Self, ContainerError> {
        let docker_context = std::env::var("DOCKER_CONTEXT").ok();
        let config_dir = auth::config_dir(None);
        Self::connect_from(&context::Inputs {
            docker_host: std::env::var("DOCKER_HOST").is_ok(),
            docker_context: docker_context.as_deref(),
            config_dir: config_dir.as_deref(),
            socket_exists: Path::new(context::LOCAL_SOCKET).exists(),
        })
    }

    /// [`connect`](Self::connect) with what it reads from the process given as values.
    fn connect_from(inputs: &context::Inputs<'_>) -> Result<Self, ContainerError> {
        match context::resolve(inputs)? {
            context::Endpoint::Defaults => {
                let client = Docker::connect_with_defaults().map_err(|e| fail("connect to", "docker", &e))?;
                Ok(Self::with_client(client))
            }
            context::Endpoint::Context(context) => {
                Self::connect_host(&context.host, &format!("{} of {context}", context.host))
            }
        }
    }

    /// A runtime for the daemon at `host`, read as `DOCKER_HOST` would be: `unix:///path/docker.sock`,
    /// `tcp://host:port` or `http://host:port`. For a host that finds its daemon in a way [`connect`](Self::connect)
    /// does not, such as a TLS or `ssh` context; otherwise as `connect`, which it does not change.
    pub fn connect_to(host: &str) -> Result<Self, ContainerError> {
        Self::connect_host(host, host)
    }

    /// A runtime for the daemon at `host`, whose errors name `label`.
    fn connect_host(host: &str, label: &str) -> Result<Self, ContainerError> {
        let client = Docker::connect_with_host(host).map_err(|e| fail("connect to", label, &e))?;
        Ok(Self::with_client(client))
    }

    fn with_client(client: Docker) -> Self {
        Self { client, negotiated: OnceCell::new(), docker_config: None }
    }

    /// Reads Docker's `config.json` for a pull from `dir` instead of `DOCKER_CONFIG` or `$HOME/.docker`: for a host
    /// that runs as a service whose home is not the user's, or a test with its own login.
    pub fn docker_config(mut self, dir: impl Into<PathBuf>) -> Self {
        self.docker_config = Some(dir.into());
        self
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

    /// Pulls `image` unless Docker has it, with the credentials of [`auth::lookup`]: `ImageNotFound` when the
    /// registry does not have it.
    async fn pull_if_missing(&self, docker: &Docker, image: &str) -> Result<(), ContainerError> {
        match docker.inspect_image(image).await {
            Ok(_) => return Ok(()),
            Err(e) if status_code(&e) == Some(404) => {}
            Err(e) => return Err(fail("inspect image", image, &e)),
        }
        let reference = map::pull_reference(image);
        let options = CreateImageOptionsBuilder::new().from_image(&reference).build();
        let auth::Lookup { credentials, source } =
            auth::lookup(self.docker_config.as_deref(), image, auth::HELPER_LIMIT).await;
        let mut pull = docker.create_image(Some(options), None, credentials);
        while let Some(progress) = pull.next().await {
            if let Err(e) = progress {
                return Err(pull_failed(image, &e, &source));
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

    /// The found container `name` if it is `Created`, with its read-only root flag and mounts; else `Conflict`, for
    /// `action` such as `copy into`.
    async fn created(
        &self,
        docker: &Docker,
        action: &str,
        name: &str,
    ) -> Result<(Found, bool, Vec<MountTarget>), ContainerError> {
        let found = self.find(docker, name).await?.ok_or_else(|| conflict(action, name))?;
        let status = Self::status(name, &found)?;
        if status != ContainerStatus::Created {
            return Err(ContainerError::new(
                ErrorKind::Conflict,
                format!("{action} {name}: the container is {status:?}, not created"),
            ));
        }
        let host_config = found.info.host_config.as_ref();
        let read_only_root = host_config.and_then(|h| h.readonly_rootfs).unwrap_or(false);
        let tmpfs = host_config.and_then(|h| h.tmpfs.as_ref());
        let mounts = map::mount_targets(found.info.mounts.as_deref().unwrap_or_default(), tmpfs);
        Ok((found, read_only_root, mounts))
    }

    /// Sends each of `uploads` into the container `id`, for `action` on `name`.
    async fn upload(
        docker: &Docker,
        action: &str,
        name: &str,
        id: &str,
        uploads: Vec<Upload>,
    ) -> Result<(), ContainerError> {
        for upload in uploads {
            let options = UploadToContainerOptionsBuilder::new().path(&upload.dir).build();
            let body = bollard::body_full(upload.tar.into());
            docker.upload_to_container(id, Some(options), body).await.map_err(|e| fail(action, name, &e))?;
        }
        Ok(())
    }

    /// What Docker's archive stat finds at `path` of the container `id`: its 404 is [`Stat::Missing`].
    async fn stat(docker: &Docker, name: &str, id: &str, path: String) -> Stat {
        let options = ContainerArchiveInfoOptionsBuilder::new().path(&path).build();
        match docker.get_container_archive_info(id, Some(options)).await {
            Ok(stat) => map::stat_of(stat.file_mode, &stat.link_target),
            Err(e) if status_code(&e) == Some(404) => Stat::Missing,
            Err(e) => Stat::Failed(fail(MAKE, name, &e)),
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

/// The error of a failed pull of `image`: `ImageNotFound` when the registry has no such image, else the kind of
/// [`kind_of`]; the message ends with `source`, where its credentials came from.
fn pull_failed(image: &str, error: &DockerError, source: &str) -> ContainerError {
    let kind = if map::image_unknown(error) { ErrorKind::ImageNotFound } else { kind_of(error) };
    ContainerError::new(kind, format!("pull {image}: {} ({source})", message_of(error)))
}

fn conflict(action: &str, name: &str) -> ContainerError {
    ContainerError::new(ErrorKind::Conflict, format!("{action} {name}: no container of that name"))
}

fn label_filter(label: &Label) -> HashMap<&'static str, Vec<String>> {
    HashMap::from([("label", vec![format!("{}={}", label.key, label.value)])])
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
        let (found, read_only_root, mounts) = self.created(docker, "copy into", name).await?;
        let uploads = map::uploads(name, files, read_only_root, &mounts)?;
        Self::upload(docker, "copy into", name, &found.id, uploads).await
    }

    async fn make_dirs(&self, name: &str, dirs: &[CopyDir]) -> Result<(), ContainerError> {
        let docker = self.docker().await?;
        let (container, read_only_root, mounts) = self.created(docker, MAKE, name).await?;
        let mut to_make = ToMake::default();
        let mut saw_missing = false;
        for (parts, mode) in dirs::named(name, dirs, read_only_root, &mounts)? {
            let found = dirs::walk(name, &parts, |path| Self::stat(docker, name, &container.id, path)).await?;
            saw_missing |= !found.missing.is_empty();
            to_make.add(name, found, mode)?;
        }
        // Docker's 404 also answers for a container removed since the inspect.
        if saw_missing && self.find(docker, name).await?.is_none() {
            return Err(conflict(MAKE, name));
        }
        let made = to_make.finish(name, read_only_root, &mounts)?;
        Self::upload(docker, MAKE, name, &container.id, map::dir_uploads(name, &made)?).await
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
                Err(e) => return Err(map::log_read_failed(name, &e)),
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn logs_since(
        &self,
        name: &str,
        cursor: Option<LogCursor>,
        max_lines: u32,
    ) -> Result<LogPage, ContainerError> {
        check_max_lines(name, max_lines)?;
        let since = map::since(name, cursor)?;
        let docker = self.docker().await?;
        let found = self.find(docker, name).await?.ok_or_else(|| conflict("logs of", name))?;
        let options =
            LogsOptionsBuilder::new().follow(false).stdout(true).stderr(true).timestamps(true).since(since).build();
        let mut stream = docker.logs(&found.id, Some(options));
        let mut pager = Pager::new(cursor, max_lines);
        while !pager.is_full() {
            match stream.next().await {
                Some(Ok(frame)) => pager.push(map::log_line(name, frame)?),
                Some(Err(e)) if status_code(&e) == Some(404) => return Err(conflict("logs of", name)),
                Some(Err(e)) => return Err(map::log_read_failed(name, &e)),
                None => break,
            }
        }
        Ok(pager.finish()) // dropping the stream ends the read
    }

    async fn copy_out(&self, name: &str, path: &str, limits: CopyLimits) -> Result<CopyOut, ContainerError> {
        let collector = Collector::new(name, path, limits)?;
        let docker = self.docker().await?;
        let found = self.find(docker, name).await?.ok_or_else(|| conflict("copy out of", name))?;
        let options = DownloadFromContainerOptionsBuilder::new().path(collector.root()).build();
        let stream = docker.download_from_container(&found.id, Some(options));
        match archive::settle(name, archive::read_archive(Box::pin(stream), collector).await)? {
            Some(out) => Ok(out),
            // Docker's 404: a missing path, unless the container was removed during the call.
            None => match self.find(docker, name).await? {
                Some(_) => Ok(CopyOut::default()),
                None => Err(conflict("copy out of", name)),
            },
        }
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
            summaries.extend(map::summary(&info)); // none when Docker reports no state
        }
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(summaries)
    }

    async fn ensure_network(&self, spec: &NetworkSpec) -> Result<(), ContainerError> {
        spec.check_isolation()?;
        let docker = self.docker().await?;
        // An older daemon can report the isolated modes in its options without implementing them.
        map::check_network(spec, docker.client_version())?;
        let name = spec.name.as_str();
        // The flags of the network named exactly `name`; `None` when there is none.
        let existing = || async {
            match docker.inspect_network(name, None::<InspectNetworkOptions>).await {
                Ok(network) if network.name.as_deref() == Some(name) => Ok(Some(map::network_flags(&network))),
                Ok(_) => Ok(None),
                Err(e) if status_code(&e) == Some(404) => Ok(None),
                Err(e) => Err(fail("inspect network", name, &e)),
            }
        };
        let compare = |(internal, isolated)| {
            for (flag, actual, expected) in
                [("internal", internal, spec.internal), ("isolated", isolated, spec.isolated)]
            {
                if actual != expected {
                    return Err(ContainerError::new(
                        ErrorKind::Conflict,
                        format!("ensure network {name}: it exists with {flag} = {actual}, not {expected}"),
                    ));
                }
            }
            Ok(())
        };
        if let Some(flags) = existing().await? {
            return compare(flags);
        }
        match docker.create_network(map::network_request(spec)).await {
            Ok(_) => Ok(()),
            Err(e) if status_code(&e) == Some(409) => match existing().await? {
                Some(flags) => compare(flags),
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
            labels: Some(map::hash_map(&spec.labels)),
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

    async fn info(&self) -> Result<RuntimeInfo, ContainerError> {
        let docker = self.docker().await?;
        let version = docker.version().await.map_err(|e| fail("read", "docker /version", &e))?;
        let system = docker.info().await.map_err(|e| fail("read", "docker /info", &e))?;
        map::runtime_info(&version, &system)
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

    /// Scenario: A context whose socket is missing.
    #[test]
    fn a_context_whose_socket_is_missing_is_daemon_unavailable_naming_both() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("contexts/meta/2d89b732b01a00a2d1675ed3cee9fd0f965daadf90603c989dd3afd4569c6896");
        std::fs::create_dir_all(&meta).unwrap();
        let host = "unix:///nonexistent/cicada/orbstack.sock";
        std::fs::write(meta.join("meta.json"), format!(r#"{{"Endpoints": {{"docker": {{"Host": "{host}"}}}}}}"#))
            .unwrap();
        let inputs = context::Inputs {
            docker_host: false,
            docker_context: Some("orbstack"),
            config_dir: Some(dir.path()),
            socket_exists: true,
        };
        let error = DockerRuntime::connect_from(&inputs).unwrap_err();
        assert_eq!(error.kind, ErrorKind::DaemonUnavailable, "{error}");
        assert!(
            error.message.starts_with(&format!("connect to {host} of context orbstack from DOCKER_CONTEXT: ")),
            "{error}"
        );
    }

    /// A context's live socket is connected as `connect_to` connects it, without contacting the daemon.
    #[tokio::test]
    async fn a_context_endpoint_is_connected_as_connect_to() {
        let (path, _) = dead_socket("context");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"currentContext": "orbstack"}"#).unwrap();
        let meta = dir.path().join("contexts/meta/2d89b732b01a00a2d1675ed3cee9fd0f965daadf90603c989dd3afd4569c6896");
        std::fs::create_dir_all(&meta).unwrap();
        let host = format!("unix://{}", path.display());
        std::fs::write(meta.join("meta.json"), format!(r#"{{"Endpoints": {{"docker": {{"Host": "{host}"}}}}}}"#))
            .unwrap();
        let inputs = context::Inputs {
            docker_host: false,
            docker_context: None,
            config_dir: Some(dir.path()),
            socket_exists: false,
        };
        let runtime = DockerRuntime::connect_from(&inputs).unwrap();
        let error = runtime.info().await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::DaemonUnavailable, "{error}");
        std::fs::remove_file(path).unwrap();
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
    async fn isolation_checks_precede_network_requests() {
        let (path, runtime) = dead_socket("network-isolation");
        let mut spec = NetworkSpec::new("net");
        spec.isolated = true;
        let error = runtime.ensure_network(&spec).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidSpec, "{error}");
        assert!(!runtime.negotiated.initialized());

        let version = ClientVersion { major_version: 1, minor_version: 47 };
        let agreed = Docker::connect_with_unix(path.to_str().unwrap(), 1, &version).unwrap();
        runtime.negotiated.set(agreed).unwrap();
        spec.internal = true;
        let error = runtime.ensure_network(&spec).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported, "{error}");
        std::fs::remove_file(path).unwrap();
    }

    /// Scenario: No daemon behind the socket.
    #[tokio::test]
    async fn info_without_a_daemon_is_daemon_unavailable_twice() {
        let (path, runtime) = dead_socket("info");
        for _ in 0..2 {
            let error = runtime.info().await.unwrap_err();
            assert_eq!(error.kind, ErrorKind::DaemonUnavailable, "{error}");
        }
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
        let runtime = runtime.docker_config("/srv/factory/.docker");
        assert_eq!(format!("{runtime:?}"), "DockerRuntime { negotiated: false, .. }");
        std::fs::remove_file(path).unwrap();
    }

    /// Scenario: A private image without a login; a helper that fails. The message says where the credentials came
    /// from, never what they are, and the kind stays the one Docker's answer maps to.
    #[tokio::test]
    async fn a_failed_pull_names_its_credential_source_and_never_a_credential() {
        let dir = tempfile::tempdir().unwrap();
        let image = "localhost:5000/team/app:1";
        let refused = DockerError::DockerStreamError {
            error: "Head \"http://localhost:5000/v2/\": no basic auth credentials".into(),
        };

        let none = auth::lookup(Some(dir.path()), image, auth::HELPER_LIMIT).await;
        let error = pull_failed(image, &refused, &none.source);
        assert_eq!(error.kind, ErrorKind::Other, "{error}");
        assert_eq!(
            error.message,
            "pull localhost:5000/team/app:1: Docker stream error: Head \"http://localhost:5000/v2/\": no basic auth credentials \
             (no credentials for localhost:5000)"
        );

        let auth = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "e2e-user:pass-s3cret");
        std::fs::write(
            dir.path().join("config.json"),
            format!(r#"{{"auths": {{"localhost:5000": {{"auth": "{auth}"}}}}}}"#),
        )
        .unwrap();
        let found = auth::lookup(Some(dir.path()), image, auth::HELPER_LIMIT).await;
        assert_eq!(found.credentials.as_ref().and_then(|c| c.password.as_deref()), Some("pass-s3cret"));
        let denied = DockerError::DockerStreamError { error: "pull access denied, repository does not exist".into() };
        let error = pull_failed(image, &denied, &found.source);
        assert_eq!(error.kind, ErrorKind::ImageNotFound, "{error}");
        assert!(error.message.ends_with(" (credentials from config.json)"), "{error}");
        for secret in ["e2e-user", "pass-s3cret", auth.as_str()] {
            assert!(!error.message.contains(secret), "{error}");
        }

        let helper = r#"{"credsStore": "cicada-no-such-helper"}"#;
        std::fs::write(dir.path().join("config.json"), helper).unwrap();
        let failing = auth::lookup(Some(dir.path()), image, auth::HELPER_LIMIT).await;
        assert!(failing.credentials.is_none());
        let error = pull_failed(image, &refused, &failing.source);
        assert!(
            error.message.ends_with(" (no credentials: docker-credential-cicada-no-such-helper not found)"),
            "{error}"
        );
    }
}
