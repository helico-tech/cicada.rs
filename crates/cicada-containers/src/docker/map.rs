//! The pure half of the Docker backend: a spec onto Docker's create body, Docker's state onto a status, its errors
//! onto kinds, the secret scrub, and the paths and tars of `copy_in`. Nothing here talks to the daemon.

use std::collections::{BTreeMap, HashMap};

use bollard::errors::Error as DockerError;
use bollard::models::{
    ContainerCreateBody, ContainerState, ContainerStateStatusEnum, EndpointSettings, HostConfig, Mount as DockerMount,
    MountPoint, MountType, NetworkingConfig,
};

use crate::paths::{MountTarget, Mounted, place};
use crate::{
    ContainerError, ContainerSpec, ContainerStatus, CopyFile, ErrorKind, MountKind, Networking, Resource, Tmpfs,
};

/// The HTTP status of a refusal by the daemon; `None` for every other error.
pub(super) fn status_code(error: &DockerError) -> Option<u16> {
    match error {
        DockerError::DockerResponseServerError { status_code, .. } => Some(*status_code),
        _ => None,
    }
}

/// The kind of an error of the Docker client: no connection or a timeout is `DaemonUnavailable`, Docker's 409
/// `Conflict` and 400 `InvalidSpec`; everything else, 404 and 500 too, is `Other` (a caller maps an image's 404).
pub(super) fn kind_of(error: &DockerError) -> ErrorKind {
    match error {
        DockerError::IOError { .. }
        | DockerError::HyperResponseError { .. }
        | DockerError::HyperLegacyError { .. }
        | DockerError::RequestTimeoutError
        | DockerError::SocketNotFoundError(_) => ErrorKind::DaemonUnavailable,
        DockerError::DockerResponseServerError { status_code: 409, .. } => ErrorKind::Conflict,
        DockerError::DockerResponseServerError { status_code: 400, .. } => ErrorKind::InvalidSpec,
        _ => ErrorKind::Other,
    }
}

/// Whether a refused remove of a sweep says the `resource` is gone: 404 for every kind, and 409 for a container,
/// which a forced remove gets only when another caller is removing it already. A volume's 409 means in use.
pub(super) fn gone(resource: Resource, error: &DockerError) -> bool {
    match status_code(error) {
        Some(404) => true,
        Some(409) => resource == Resource::Container,
        _ => false,
    }
}

/// Docker's own message: the body of a refusal, or the client's description of any other error.
pub(super) fn message_of(error: &DockerError) -> String {
    match error {
        DockerError::DockerResponseServerError { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// The error of `action` on `name`, as `<action> <name>: <Docker's message>`, with its kind from [`kind_of`].
pub(super) fn fail(action: &str, name: &str, error: &DockerError) -> ContainerError {
    ContainerError::new(kind_of(error), format!("{action} {name}: {}", message_of(error)))
}

/// Whether a failed pull says the registry has no such image. The pull stream carries no code, so its text decides.
pub(super) fn image_unknown(error: &DockerError) -> bool {
    if status_code(error) == Some(404) {
        return true;
    }
    let text = match error {
        DockerError::DockerStreamError { error } => error.to_lowercase(),
        DockerError::DockerResponseServerError { message, .. } => message.to_lowercase(),
        _ => return false,
    };
    ["not found", "manifest unknown", "does not exist"].iter().any(|needle| text.contains(needle))
}

/// `message` with every non-empty secret value of `spec` replaced by `***`, the longest first, so a value that holds
/// another is replaced whole.
pub(super) fn scrub(message: &str, spec: &ContainerSpec) -> String {
    let mut secrets: Vec<&str> = spec.secret_env.values().map(String::as_str).filter(|v| !v.is_empty()).collect();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    secrets.iter().fold(message.to_owned(), |text, secret| text.replace(secret, "***"))
}

/// The first environment variable name, plain or secret, that Docker cannot take: an empty one, or one with `=`.
pub(super) fn bad_env_name(spec: &ContainerSpec) -> Option<&str> {
    spec.env.keys().chain(spec.secret_env.keys()).map(String::as_str).find(|name| name.is_empty() || name.contains('='))
}

/// The reference to pull: `image` with `:latest` when it names no tag or digest, as Docker pulls every tag of a bare
/// repository.
pub(super) fn pull_reference(image: &str) -> String {
    let last = image.rsplit('/').next().unwrap_or(image);
    if last.contains(':') || last.contains('@') { image.to_owned() } else { format!("{image}:latest") }
}

fn saturating(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn hash_map(map: &BTreeMap<String, String>) -> HashMap<String, String> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// The options of a tmpfs: `size=<bytes>[,mode=<octal>][,uid=<n>][,gid=<n>][,exec]`, each only when set. Docker adds
/// `noexec,nosuid,nodev` unless `exec` is there.
pub(super) fn tmpfs_options(tmpfs: &Tmpfs) -> String {
    let mut options = Vec::new();
    if let Some(size) = tmpfs.size_bytes {
        options.push(format!("size={size}"));
    }
    if let Some(mode) = tmpfs.mode {
        options.push(format!("mode={mode:o}"));
    }
    if let Some(uid) = tmpfs.uid {
        options.push(format!("uid={uid}"));
    }
    if let Some(gid) = tmpfs.gid {
        options.push(format!("gid={gid}"));
    }
    if tmpfs.exec {
        options.push("exec".to_owned());
    }
    options.join(",")
}

/// The body of Docker's create call for `spec`: every field that is set, with the secret environment next to the
/// plain one, and each hardening field that is on.
pub(super) fn create_body(spec: &ContainerSpec) -> ContainerCreateBody {
    let env: Vec<String> = spec.env.iter().chain(&spec.secret_env).map(|(k, v)| format!("{k}={v}")).collect();
    let mounts: Vec<DockerMount> = spec
        .mounts
        .iter()
        .map(|mount| DockerMount {
            typ: Some(match mount.kind {
                MountKind::Volume => MountType::VOLUME,
                MountKind::Bind => MountType::BIND,
            }),
            source: Some(mount.source.clone()),
            target: Some(mount.target.clone()),
            read_only: Some(mount.read_only),
            ..Default::default()
        })
        .collect();
    let (network_mode, networking_config) = match &spec.network {
        Networking::Default => (None, None),
        Networking::None => (Some("none".to_owned()), None),
        Networking::Attach(networks) => match networks.first() {
            None => (Some("none".to_owned()), None),
            Some(first) => {
                let endpoints = networks
                    .iter()
                    .map(|network| {
                        let aliases = (!network.aliases.is_empty()).then(|| network.aliases.clone());
                        (network.name.clone(), EndpointSettings { aliases, ..Default::default() })
                    })
                    .collect();
                (Some(first.name.clone()), Some(NetworkingConfig { endpoints_config: Some(endpoints) }))
            }
        },
    };
    let memory = spec.memory_bytes.map(saturating);
    let host_config = HostConfig {
        nano_cpus: spec.cpu_millis.map(|millis| saturating(millis.saturating_mul(1_000_000))),
        memory,
        memory_swap: memory,
        mounts: (!mounts.is_empty()).then_some(mounts),
        network_mode,
        readonly_rootfs: spec.read_only_root.then_some(true),
        cap_drop: spec.drop_all_capabilities.then(|| vec!["ALL".to_owned()]),
        security_opt: spec.no_new_privileges.then(|| vec!["no-new-privileges".to_owned()]),
        tmpfs: (!spec.tmpfs.is_empty())
            .then(|| spec.tmpfs.iter().map(|t| (t.target.clone(), tmpfs_options(t))).collect()),
        init: spec.init.then_some(true),
        pids_limit: spec.pids_limit.map(saturating),
        ..Default::default()
    };
    ContainerCreateBody {
        image: Some(spec.image.clone()),
        entrypoint: spec.entrypoint.clone(),
        cmd: spec.command.clone(),
        working_dir: spec.workdir.clone(),
        user: spec.user.clone(),
        env: (!env.is_empty()).then_some(env),
        labels: (!spec.labels.is_empty()).then(|| hash_map(&spec.labels)),
        host_config: Some(host_config),
        networking_config,
        ..Default::default()
    }
}

/// The status of a container from Docker's state; `None` for a state Docker did not report.
pub(super) fn status_of(state: &ContainerState) -> Option<ContainerStatus> {
    use ContainerStateStatusEnum as S;
    let error = state.error.as_deref().filter(|e| !e.is_empty());
    let code = state.exit_code;
    let failed = |reason: &str| ContainerStatus::Failed { code, reason: reason.to_owned() };
    Some(match state.status? {
        S::CREATED => match error {
            Some(reason) => failed(reason),
            None => ContainerStatus::Created,
        },
        S::RUNNING | S::PAUSED | S::RESTARTING | S::STOPPING => ContainerStatus::Running,
        S::EXITED | S::REMOVING => match (state.oom_killed == Some(true), error) {
            (true, _) => failed("out of memory"),
            (false, Some(reason)) => failed(reason),
            (false, None) => ContainerStatus::Exited { code: code.unwrap_or(-1) },
        },
        S::DEAD => failed(error.unwrap_or("dead")),
        S::EMPTY => return None,
    })
}

/// The mounts of an inspected, created container: its volume and bind mounts, and its tmpfs targets from `tmpfs` of
/// its host config, as Docker makes a tmpfs mount point only at start.
pub(super) fn mount_targets(mounts: &[MountPoint], tmpfs: Option<&HashMap<String, String>>) -> Vec<MountTarget> {
    let volumes_and_binds =
        mounts.iter().filter(|mount| matches!(mount.typ.as_deref(), Some("volume" | "bind"))).filter_map(|mount| {
            let writable = mount.rw.unwrap_or(true);
            Some(MountTarget::new(mount.destination.as_deref()?, Mounted::Mount { writable }))
        });
    let tmpfs = tmpfs.into_iter().flat_map(HashMap::keys).map(|target| MountTarget::new(target, Mounted::Tmpfs));
    volumes_and_binds.chain(tmpfs).collect()
}

/// One upload of `copy_in`: a tar of files under the directory `dir`.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Upload {
    /// The absolute directory Docker extracts the tar into.
    pub dir: String,
    /// The tar.
    pub tar: Vec<u8>,
}

/// The uploads that write `files` into the created container `name`: one tar at `/`, or with a read-only root one
/// per mount target. Refuses, before any write, every path the shared rule refuses (see [`place`]).
pub(super) fn uploads(
    name: &str,
    files: &[CopyFile],
    read_only_root: bool,
    mounts: &[MountTarget],
) -> Result<Vec<Upload>, ContainerError> {
    let invalid = |why: String| ContainerError::new(ErrorKind::InvalidSpec, format!("copy into {name}: {why}"));
    // Per upload directory (as parts), its files with their paths relative to it.
    let mut groups: BTreeMap<Vec<String>, Vec<(String, &CopyFile)>> = BTreeMap::new();
    for file in files {
        let (parts, mount) = place(name, &file.path, read_only_root, mounts)?;
        let dir: Vec<String> = match mount {
            Some(m) if read_only_root => m.parts.clone(),
            _ => Vec::new(),
        };
        let relative = parts[dir.len()..].join("/");
        groups.entry(dir).or_default().push((relative, file));
    }
    groups
        .into_iter()
        .map(|(dir, files)| {
            let tar = tar_of(&files).map_err(|e| invalid(format!("cannot build the tar: {e}")))?;
            Ok(Upload { dir: format!("/{}", dir.join("/")), tar })
        })
        .collect()
}

/// A tar of `files` under their relative paths: each with its mode, owned by uid and gid 0, with mtime 0.
fn tar_of(files: &[(String, &CopyFile)]) -> std::io::Result<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, file) in files {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(file.contents.len() as u64);
        header.set_mode(file.mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        builder.append_data(&mut header, path, file.contents.as_slice())?;
    }
    builder.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mount, NetworkAttachment};

    fn server(status_code: u16, message: &str) -> DockerError {
        DockerError::DockerResponseServerError { status_code, message: message.into() }
    }

    #[test]
    fn a_full_spec_reaches_every_field_of_the_create_body() {
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        spec.entrypoint = Some(vec!["/bin/sh".into(), "-c".into()]);
        spec.command = Some(vec!["echo hi".into()]);
        spec.workdir = Some("/work".into());
        spec.user = Some("1000:1000".into());
        spec.env.insert("PLAIN".into(), "1".into());
        spec.secret_env.insert("TOKEN".into(), "s3cret".into());
        spec.labels.insert("run".into(), "r1".into());
        spec.cpu_millis = Some(1500);
        spec.memory_bytes = Some(64 << 20);
        let mut readonly = Mount::new(MountKind::Bind, "/srv/in", "/in");
        readonly.read_only = true;
        spec.mounts = vec![Mount::new(MountKind::Volume, "work", "/work"), readonly];
        let mut db = NetworkAttachment::new("db-net");
        db.aliases = vec!["db".into()];
        spec.network = Networking::Attach(vec![NetworkAttachment::new("front"), db]);
        spec.read_only_root = true;
        spec.drop_all_capabilities = true;
        spec.no_new_privileges = true;
        let mut tmp = Tmpfs::new("/tmp");
        tmp.size_bytes = Some(1 << 20);
        tmp.mode = Some(0o1777);
        tmp.uid = Some(1000);
        tmp.gid = Some(1000);
        tmp.exec = true;
        spec.tmpfs = vec![tmp, Tmpfs::new("/run")];
        spec.init = true;
        spec.pids_limit = Some(256);

        let body = create_body(&spec);
        assert_eq!(body.image.as_deref(), Some("alpine:3"));
        assert_eq!(body.entrypoint, Some(vec!["/bin/sh".to_owned(), "-c".to_owned()]));
        assert_eq!(body.cmd, Some(vec!["echo hi".to_owned()]));
        assert_eq!((body.working_dir.as_deref(), body.user.as_deref()), (Some("/work"), Some("1000:1000")));
        assert_eq!(body.env, Some(vec!["PLAIN=1".to_owned(), "TOKEN=s3cret".to_owned()]));
        assert_eq!(body.labels, Some(HashMap::from([("run".to_owned(), "r1".to_owned())])));
        let host = body.host_config.unwrap();
        assert_eq!(host.nano_cpus, Some(1_500_000_000));
        assert_eq!((host.memory, host.memory_swap), (Some(64 << 20), Some(64 << 20)), "no swap beyond the limit");
        let mounts = host.mounts.unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(
            (mounts[0].typ, mounts[0].source.as_deref(), mounts[0].target.as_deref(), mounts[0].read_only),
            (Some(MountType::VOLUME), Some("work"), Some("/work"), Some(false))
        );
        assert_eq!((mounts[1].typ, mounts[1].read_only), (Some(MountType::BIND), Some(true)));
        assert_eq!(host.binds, None, "typed mounts, not bind strings");
        assert_eq!(host.network_mode.as_deref(), Some("front"));
        let endpoints = body.networking_config.unwrap().endpoints_config.unwrap();
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints["front"].aliases, None);
        assert_eq!(endpoints["db-net"].aliases, Some(vec!["db".to_owned()]));
        assert_eq!(host.readonly_rootfs, Some(true));
        assert_eq!(host.cap_drop, Some(vec!["ALL".to_owned()]));
        assert_eq!(host.security_opt, Some(vec!["no-new-privileges".to_owned()]));
        let tmpfs = host.tmpfs.unwrap();
        assert_eq!(tmpfs["/tmp"], "size=1048576,mode=1777,uid=1000,gid=1000,exec");
        assert_eq!(tmpfs["/run"], "");
        assert_eq!((host.init, host.pids_limit), (Some(true), Some(256)));
    }

    #[test]
    fn a_plain_spec_sets_nothing_but_its_image() {
        let body = create_body(&ContainerSpec::new("job-1", "alpine:3"));
        assert_eq!((body.entrypoint, body.cmd, body.env, body.labels), (None, None, None, None));
        assert_eq!(body.networking_config, None);
        assert_eq!(body.host_config, Some(HostConfig::default()), "no limit, no hardening, the default network");
    }

    #[test]
    fn networking_none_and_an_empty_attach_join_no_network() {
        for network in [Networking::None, Networking::Attach(Vec::new())] {
            let mut spec = ContainerSpec::new("job-1", "alpine:3");
            spec.network = network;
            let body = create_body(&spec);
            assert_eq!(body.host_config.unwrap().network_mode.as_deref(), Some("none"));
            assert_eq!(body.networking_config, None);
        }
    }

    fn state(status: ContainerStateStatusEnum) -> ContainerState {
        ContainerState { status: Some(status), exit_code: Some(0), ..Default::default() }
    }

    #[test]
    fn docker_states_map_onto_statuses() {
        use ContainerStateStatusEnum as S;
        assert_eq!(status_of(&state(S::CREATED)), Some(ContainerStatus::Created));
        for running in [S::RUNNING, S::PAUSED, S::RESTARTING, S::STOPPING] {
            assert_eq!(status_of(&state(running)), Some(ContainerStatus::Running), "{running:?}");
        }
        for exited in [S::EXITED, S::REMOVING] {
            let three = ContainerState { exit_code: Some(3), ..state(exited) };
            assert_eq!(status_of(&three), Some(ContainerStatus::Exited { code: 3 }), "{exited:?}");
        }
        let oom = ContainerState { oom_killed: Some(true), exit_code: Some(137), ..state(S::EXITED) };
        assert_eq!(status_of(&oom), Some(ContainerStatus::Failed { code: Some(137), reason: "out of memory".into() }));
        let error = ContainerState { error: Some("lost".into()), exit_code: Some(1), ..state(S::EXITED) };
        assert_eq!(status_of(&error), Some(ContainerStatus::Failed { code: Some(1), reason: "lost".into() }));
        let empty_error = ContainerState { error: Some(String::new()), ..state(S::EXITED) };
        assert_eq!(status_of(&empty_error), Some(ContainerStatus::Exited { code: 0 }));
        let refused =
            ContainerState { error: Some("exec: nope: not found".into()), exit_code: Some(127), ..state(S::CREATED) };
        assert_eq!(
            status_of(&refused),
            Some(ContainerStatus::Failed { code: Some(127), reason: "exec: nope: not found".into() })
        );
        assert_eq!(status_of(&state(S::DEAD)), Some(ContainerStatus::Failed { code: Some(0), reason: "dead".into() }));
        assert_eq!(status_of(&state(S::EMPTY)), None);
        assert_eq!(status_of(&ContainerState::default()), None);
    }

    #[test]
    fn docker_errors_map_onto_kinds() {
        let io = DockerError::IOError { err: std::io::Error::from(std::io::ErrorKind::ConnectionRefused) };
        assert_eq!(kind_of(&io), ErrorKind::DaemonUnavailable);
        assert_eq!(kind_of(&DockerError::RequestTimeoutError), ErrorKind::DaemonUnavailable);
        assert_eq!(kind_of(&DockerError::SocketNotFoundError("/x.sock".into())), ErrorKind::DaemonUnavailable);
        assert_eq!(kind_of(&server(409, "in use")), ErrorKind::Conflict);
        assert_eq!(kind_of(&server(400, "bad")), ErrorKind::InvalidSpec);
        for other in [404, 403, 500] {
            assert_eq!(kind_of(&server(other, "x")), ErrorKind::Other, "{other}");
        }
        assert_eq!(kind_of(&DockerError::UnsupportedURISchemeError { uri: "ssh://h".into() }), ErrorKind::Other);
        let error = fail("start", "job-1", &server(400, "no such executable"));
        assert_eq!(error, ContainerError::new(ErrorKind::InvalidSpec, "start job-1: no such executable"));
    }

    #[test]
    fn a_sweep_counts_404_as_gone_and_409_only_for_a_container() {
        for resource in [Resource::Container, Resource::Network, Resource::Volume] {
            assert!(gone(resource, &server(404, "no such")), "{resource:?}");
            assert_eq!(gone(resource, &server(409, "busy")), resource == Resource::Container, "{resource:?}");
            assert!(!gone(resource, &server(500, "internal")), "{resource:?}");
            assert!(!gone(resource, &DockerError::RequestTimeoutError), "{resource:?}");
        }
    }

    #[test]
    fn a_pull_error_naming_a_missing_image_is_image_not_found() {
        assert!(image_unknown(&server(404, "anything")));
        for text in ["manifest for busybox:nope not found: manifest unknown", "repository does not exist", "Not Found"]
        {
            assert!(image_unknown(&DockerError::DockerStreamError { error: text.into() }), "{text}");
        }
        assert!(!image_unknown(&DockerError::DockerStreamError { error: "toomanyrequests".into() }));
        assert!(!image_unknown(&server(500, "internal")));
        assert!(!image_unknown(&DockerError::RequestTimeoutError));
    }

    #[test]
    fn secret_values_are_scrubbed_longest_first() {
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        spec.secret_env.insert("A".into(), "abc".into());
        spec.secret_env.insert("B".into(), "abcdef".into());
        spec.secret_env.insert("EMPTY".into(), String::new());
        spec.env.insert("PLAIN".into(), "visible".into());
        assert_eq!(scrub("bad env abcdef and abc, visible", &spec), "bad env *** and ***, visible");
    }

    #[test]
    fn an_env_name_docker_cannot_take_is_found_by_name() {
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        spec.env.insert("OK".into(), "1".into());
        assert_eq!(bad_env_name(&spec), None);
        spec.secret_env.insert("A=B".into(), "s3cret".into());
        assert_eq!(bad_env_name(&spec), Some("A=B"));
        spec.secret_env.clear();
        spec.env.insert(String::new(), "1".into());
        assert_eq!(bad_env_name(&spec), Some(""));
    }

    #[test]
    fn a_bare_repository_is_pulled_as_latest() {
        assert_eq!(pull_reference("busybox"), "busybox:latest");
        assert_eq!(pull_reference("localhost:5000/team/app"), "localhost:5000/team/app:latest");
        assert_eq!(pull_reference("busybox:1.37.0"), "busybox:1.37.0");
        assert_eq!(pull_reference("alpine@sha256:abc"), "alpine@sha256:abc");
    }

    fn target(path: &str, writable: bool) -> MountTarget {
        MountTarget::new(path, Mounted::Mount { writable })
    }

    #[test]
    fn volume_and_bind_mounts_and_the_tmpfs_of_the_host_config_are_targets() {
        let point = |typ: &str, destination: &str, rw: bool| MountPoint {
            typ: Some(typ.into()),
            destination: Some(destination.into()),
            rw: Some(rw),
            ..Default::default()
        };
        let mounts = [point("volume", "/work", true), point("bind", "/in/", false), point("npipe", "/pipe", true)];
        assert_eq!(mount_targets(&mounts, None), [target("/work", true), target("/in", false)]);
        let tmpfs = HashMap::from([("/scratch".to_owned(), "size=1024".to_owned())]);
        assert_eq!(mount_targets(&[], Some(&tmpfs)), [MountTarget::new("/scratch", Mounted::Tmpfs)]);
    }

    fn entries(tar: &[u8]) -> Vec<(String, u32, u64, u64, u64, Vec<u8>)> {
        let mut archive = tar::Archive::new(tar);
        archive
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let header = entry.header().clone();
                let mut contents = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
                let path = entry.path().unwrap().display().to_string();
                (
                    path,
                    header.mode().unwrap(),
                    header.uid().unwrap(),
                    header.gid().unwrap(),
                    header.mtime().unwrap(),
                    contents,
                )
            })
            .collect()
    }

    #[test]
    fn a_writable_root_gets_one_tar_at_the_root() {
        let mut script = CopyFile::new("/work/run.sh", "echo hi");
        script.mode = 0o755;
        let long = format!("/work/{}/file", "d".repeat(120));
        let files = [script, CopyFile::new("/etc/job.json", "{}"), CopyFile::new(long.clone(), "x")];
        let uploads = uploads("job-1", &files, false, &[target("/work", true)]).unwrap();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].dir, "/");
        let entries = entries(&uploads[0].tar);
        assert_eq!(entries[0], ("work/run.sh".into(), 0o755, 0, 0, 0, b"echo hi".to_vec()));
        assert_eq!(entries[1], ("etc/job.json".into(), 0o644, 0, 0, 0, b"{}".to_vec()));
        assert_eq!(entries[2].0, &long[1..], "a long path survives");
    }

    #[test]
    fn a_read_only_root_gets_one_tar_per_mount() {
        let mounts = [target("/work", true), target("/work/cache", true), target("/out", true)];
        let files =
            [CopyFile::new("/work/a", "1"), CopyFile::new("/out/b/c", "2"), CopyFile::new("/work/cache/d", "3")];
        let uploads = uploads("job-1", &files, true, &mounts).unwrap();
        let dirs: Vec<&str> = uploads.iter().map(|u| u.dir.as_str()).collect();
        assert_eq!(dirs, ["/out", "/work", "/work/cache"]);
        assert_eq!(entries(&uploads[0].tar)[0].0, "b/c");
        assert_eq!(entries(&uploads[1].tar)[0].0, "a");
        assert_eq!(entries(&uploads[2].tar)[0].0, "d", "the deepest mount holds it");
    }

    #[test]
    fn a_path_off_every_mount_of_a_read_only_root_is_refused() {
        let mounts = [target("/work", true)];
        let files = [CopyFile::new("/work/a", "1"), CopyFile::new("/etc/b", "2")];
        let error = uploads("job-1", &files, true, &mounts).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidSpec);
        assert!(error.message.contains("/etc/b"), "{error}");
        for read_only_root in [false, true] {
            let mount_itself = [CopyFile::new("/work/", "1")];
            let error = uploads("job-1", &mount_itself, read_only_root, &mounts).unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidSpec, "{read_only_root}");
        }
        let sibling = [CopyFile::new("/workspace/a", "1")];
        assert_eq!(uploads("job-1", &sibling, true, &mounts).unwrap_err().kind, ErrorKind::InvalidSpec);
    }

    #[test]
    fn a_path_on_a_tmpfs_is_refused_on_a_writable_root() {
        let mounts = [MountTarget::new("/scratch", Mounted::Tmpfs)];
        let files = [CopyFile::new("/etc/a", "1"), CopyFile::new("/scratch/c", "2")];
        let error = uploads("job-1", &files, false, &mounts).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidSpec);
        assert!(error.message.contains("/scratch/c"), "{error}");
    }

    #[test]
    fn a_path_on_a_read_only_mount_is_refused() {
        let mounts = [target("/in", false), target("/in/rw", true)];
        for read_only_root in [false, true] {
            let error = uploads("job-1", &[CopyFile::new("/in/a", "1")], read_only_root, &mounts).unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidSpec, "{read_only_root}");
            uploads("job-1", &[CopyFile::new("/in/rw/a", "1")], read_only_root, &mounts).unwrap();
        }
    }
}
