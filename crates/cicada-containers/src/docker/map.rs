//! The pure half of the Docker backend: a spec onto Docker's create body, Docker's state onto a status, its errors
//! onto kinds, the secret scrub, the paths and tars of `copy_in` and `make_dirs`, the archive stat, and the frames of a
//! timed log read and Docker's version onto the runtime info. Nothing here talks to the daemon.

use std::collections::{BTreeMap, HashMap};

use bollard::ClientVersion;
use bollard::container::LogOutput;
use bollard::errors::Error as DockerError;
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, ContainerState, ContainerStateStatusEnum, EndpointSettings,
    HostConfig, Mount as DockerMount, MountPoint, MountType, NetworkCreateRequest, NetworkInspect, NetworkingConfig,
    SystemInfo, SystemVersion,
};

use crate::dirs::{Make, Stat};
use crate::paths::{MountTarget, Mounted, place};
use crate::{
    ContainerError, ContainerSpec, ContainerStatus, ContainerSummary, CopyFile, ErrorKind, LogCursor, LogLine,
    LogStream, MountKind, NetworkSpec, Networking, OUT_OF_MEMORY, Resource, RuntimeInfo, Tmpfs,
};

const API_ISOLATED_NETWORK: ClientVersion = ClientVersion { major_version: 1, minor_version: 48 };
const GATEWAY_MODES: [&str; 2] =
    ["com.docker.network.bridge.gateway_mode_ipv4", "com.docker.network.bridge.gateway_mode_ipv6"];

/// Isolation needs an internal network and a daemon that implements the isolated gateway modes (Docker 28).
pub(super) fn check_network(spec: &NetworkSpec, version: ClientVersion) -> Result<(), ContainerError> {
    spec.check_isolation()?;
    if spec.isolated && version < API_ISOLATED_NETWORK {
        return Err(ContainerError::new(
            ErrorKind::Unsupported,
            format!(
                "ensure network {}: isolated requires Docker 28 (API {API_ISOLATED_NETWORK}), negotiated API {version}",
                spec.name,
            ),
        ));
    }
    Ok(())
}

/// Creates a bridge with isolated gateway modes for both IP families when requested.
pub(super) fn network_request(spec: &NetworkSpec) -> NetworkCreateRequest {
    NetworkCreateRequest {
        name: spec.name.clone(),
        driver: Some("bridge".to_owned()),
        internal: Some(spec.internal),
        options: spec.isolated.then(|| GATEWAY_MODES.map(|key| (key.to_owned(), "isolated".to_owned())).into()),
        labels: Some(hash_map(&spec.labels)),
        ..Default::default()
    }
}

/// The internal and isolated flags of an inspected network. Isolation needs both gateway modes.
pub(super) fn network_flags(network: &NetworkInspect) -> (bool, bool) {
    let isolated = network.options.as_ref().is_some_and(|options| {
        GATEWAY_MODES.iter().all(|&key| options.get(key).is_some_and(|mode| mode == "isolated"))
    });
    (network.internal.unwrap_or(false), isolated)
}

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

/// The error of a log read of `name`: Docker's 501 (a logging driver that does not support reading, such as `none`)
/// is `Unsupported`, as no retry can read that log; every other error is as [`fail`] maps it. Only the status counts:
/// once Docker streams, its errors are text in the log.
pub(super) fn log_read_failed(name: &str, error: &DockerError) -> ContainerError {
    match status_code(error) {
        Some(501) => ContainerError::new(ErrorKind::Unsupported, format!("logs of {name}: {}", message_of(error))),
        _ => fail("logs of", name, error),
    }
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

pub(super) fn hash_map(map: &BTreeMap<String, String>) -> HashMap<String, String> {
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
            (true, _) => failed(OUT_OF_MEMORY),
            (false, Some(reason)) => failed(reason),
            (false, None) => ContainerStatus::Exited { code: code.unwrap_or(-1) },
        },
        S::DEAD => failed(error.unwrap_or("dead")),
        S::EMPTY => return None,
    })
}

/// The summary of an inspected container for `list`; `None` when Docker reports no state or an empty status, so
/// `list` leaves it out instead of inventing a status.
pub(super) fn summary(info: &ContainerInspectResponse) -> Option<ContainerSummary> {
    let status = info.state.as_ref().and_then(status_of)?;
    let name = info.name.as_deref().unwrap_or_default().trim_start_matches('/');
    Some(ContainerSummary::new(name, status))
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

/// The bit of Go's `os.FileMode` that Docker's archive stat sets for a folder (`ModeDir`).
const GO_MODE_DIR: u32 = 1 << 31;
/// The bit of Go's `os.FileMode` that Docker's archive stat sets for a symbolic link (`ModeSymlink`).
const GO_MODE_SYMLINK: u32 = 1 << 27;

/// What Docker's archive stat says of a path, from its Go file mode and link target: a folder, a link to
/// `link_target`, or anything else. The stat does not follow a link at the path's last part.
pub(super) fn stat_of(file_mode: u32, link_target: &str) -> Stat {
    if file_mode & GO_MODE_DIR != 0 {
        Stat::Folder
    } else if file_mode & GO_MODE_SYMLINK != 0 {
        Stat::Link(link_target.to_owned())
    } else {
        Stat::Other
    }
}

/// The uploads that make the folders of `made`, in its order (parents first): one tar of folder entries at `/`, or
/// with a read-only root one per mount, each folder named by its path relative to the upload's directory.
pub(super) fn dir_uploads(name: &str, made: &[Make]) -> Result<Vec<Upload>, ContainerError> {
    let mut groups: BTreeMap<Vec<String>, Vec<(String, u32)>> = BTreeMap::new();
    for make in made {
        let dir = make.mount.clone().unwrap_or_default();
        let relative = make.parts[dir.len()..].join("/");
        groups.entry(dir).or_default().push((relative, make.mode));
    }
    groups
        .into_iter()
        .map(|(dir, dirs)| {
            let tar = dir_tar(&dirs).map_err(|e| {
                ContainerError::new(
                    ErrorKind::InvalidSpec,
                    format!("make folders in {name}: cannot build the tar: {e}"),
                )
            })?;
            Ok(Upload { dir: format!("/{}", dir.join("/")), tar })
        })
        .collect()
}

/// A tar of folder entries under their relative paths: each with its mode, owned by uid and gid 0, with mtime 0.
fn dir_tar(dirs: &[(String, u32)]) -> std::io::Result<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, mode) in dirs {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(*mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        builder.append_data(&mut header, format!("{path}/"), std::io::empty())?;
    }
    builder.into_inner()
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

/// Docker's `since` for a read from `cursor` (0, the start, for `None`): bollard takes an `i32`, so a second after
/// January 2038 fails with `Other` rather than read from the wrong time.
pub(super) fn since(name: &str, cursor: Option<LogCursor>) -> Result<i32, ContainerError> {
    let second = cursor.map_or(0, LogCursor::second);
    i32::try_from(second).map_err(|_| {
        ContainerError::new(
            ErrorKind::Other,
            format!("logs of {name}: the cursor's second {second} does not fit Docker's 32-bit `since`"),
        )
    })
}

/// One frame of a log read with `timestamps=true`, which is one log entry: an RFC 3339 time, a space and the text,
/// with a newline unless Docker split the line at 16 KiB. The text is lossy UTF-8 without the newline; stderr is
/// [`LogStream::Err`], anything else (stdout, or the one stream of a TTY) [`LogStream::Out`]. A frame that does not
/// start with a time fails with `Other`.
pub(super) fn log_line(name: &str, frame: LogOutput) -> Result<LogLine, ContainerError> {
    let (stream, message) = match frame {
        LogOutput::StdErr { message } => (LogStream::Err, message),
        LogOutput::StdOut { message } | LogOutput::StdIn { message } | LogOutput::Console { message } => {
            (LogStream::Out, message)
        }
    };
    let bytes = message.strip_suffix(b"\n").unwrap_or(&message[..]);
    let time = bytes
        .iter()
        .position(|b| *b == b' ')
        .and_then(|space| Some((std::str::from_utf8(&bytes[..space]).ok()?, space)))
        .and_then(|(time, space)| Some((cicada::time::parse_instant(time).ok()?, space)));
    let Some((time, space)) = time else {
        let start = String::from_utf8_lossy(&bytes[..bytes.len().min(40)]).into_owned();
        return Err(ContainerError::new(
            ErrorKind::Other,
            format!("logs of {name}: Docker sent a log frame without a time: {start:?}"),
        ));
    };
    Ok(LogLine::new(time, stream, String::from_utf8_lossy(&bytes[space + 1..])))
}

/// The `Arch` and `Os` of Docker's `/version`, as they are, and the `ID` of its `/info`; a missing or empty `Arch` or
/// `Os` is `Other`, a missing or empty `ID` is no id.
pub(super) fn runtime_info(version: &SystemVersion, system: &SystemInfo) -> Result<RuntimeInfo, ContainerError> {
    let field = |value: &Option<String>, name: &str| match value.as_deref() {
        Some(value) if !value.is_empty() => Ok(value.to_owned()),
        _ => Err(ContainerError::new(ErrorKind::Other, format!("docker /version gave no {name}"))),
    };
    let info = RuntimeInfo::new(field(&version.arch, "Arch")?, field(&version.os, "Os")?);
    Ok(match system.id.as_deref() {
        Some(id) if !id.is_empty() => info.with_id(id),
        _ => info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mount, NetworkAttachment};

    #[test]
    fn isolated_networks_require_internal_and_api_1_48() {
        let api_1_44 = ClientVersion { major_version: 1, minor_version: 44 };
        let api_1_47 = ClientVersion { major_version: 1, minor_version: 47 };
        let api_1_48 = ClientVersion { major_version: 1, minor_version: 48 };
        let mut spec = NetworkSpec::new("net");
        check_network(&spec, api_1_44).unwrap();
        spec.isolated = true;
        assert_eq!(check_network(&spec, api_1_47).unwrap_err().kind, ErrorKind::InvalidSpec);
        assert_eq!(check_network(&spec, api_1_48).unwrap_err().kind, ErrorKind::InvalidSpec);
        spec.internal = true;
        let error = check_network(&spec, api_1_47).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported);
        assert!(error.message.contains("Docker 28") && error.message.contains("negotiated API 1.47"), "{error}");
        check_network(&spec, api_1_48).unwrap();
    }

    #[test]
    fn an_isolated_bridge_request_sets_both_gateway_modes() {
        let mut spec = NetworkSpec::new("net");
        spec.internal = true;
        spec.labels.insert("run".into(), "test".into());
        assert_eq!(network_request(&spec).options, None);
        spec.isolated = true;
        let request = network_request(&spec);
        assert_eq!(request.name, "net");
        assert_eq!(request.driver.as_deref(), Some("bridge"));
        assert_eq!(request.internal, Some(true));
        assert_eq!(request.labels, Some(HashMap::from([("run".into(), "test".into())])));
        assert_eq!(
            request.options,
            Some(HashMap::from([
                ("com.docker.network.bridge.gateway_mode_ipv4".into(), "isolated".into()),
                ("com.docker.network.bridge.gateway_mode_ipv6".into(), "isolated".into()),
            ])),
        );
    }

    #[test]
    fn an_inspected_network_needs_both_isolated_gateway_modes() {
        let mut network = NetworkInspect::default();
        assert_eq!(network_flags(&network), (false, false));
        network.internal = Some(true);
        assert_eq!(network_flags(&network), (true, false));
        let [ipv4, ipv6] = GATEWAY_MODES;
        for key in [ipv4, ipv6] {
            network.options = Some(HashMap::from([(key.into(), "isolated".into())]));
            assert_eq!(network_flags(&network), (true, false));
        }
        network.options = Some(HashMap::from([(ipv4.into(), "isolated".into()), (ipv6.into(), "nat".into())]));
        assert_eq!(network_flags(&network), (true, false));
        network.options.as_mut().unwrap().insert(ipv6.into(), "isolated".into());
        assert_eq!(network_flags(&network), (true, true));
    }

    fn server(status_code: u16, message: &str) -> DockerError {
        DockerError::DockerResponseServerError { status_code, message: message.into() }
    }

    fn frame(stderr: bool, bytes: &[u8]) -> LogOutput {
        let message = bytes.to_vec().into();
        if stderr { LogOutput::StdErr { message } } else { LogOutput::StdOut { message } }
    }

    #[test]
    fn a_log_frame_gives_its_time_stream_and_text() {
        let line = log_line("job-1", frame(false, b"2026-11-24T10:00:01.123456789Z hello world\n")).unwrap();
        assert_eq!(line.time, cicada::time::parse_instant("2026-11-24T10:00:01.123456789Z").unwrap());
        assert_eq!(line.time.timestamp_subsec_nanos(), 123_456_789);
        assert_eq!((line.stream, line.text.as_str()), (LogStream::Out, "hello world"));
        let line = log_line("job-1", frame(true, b"2026-11-24T10:00:01Z  two spaces\n")).unwrap();
        assert_eq!((line.stream, line.text.as_str()), (LogStream::Err, " two spaces"));
        // A part of a line Docker split at 16 KiB has no newline; an empty line keeps its space.
        assert_eq!(log_line("job-1", frame(false, b"2026-11-24T10:00:01Z part")).unwrap().text, "part");
        assert_eq!(log_line("job-1", frame(false, b"2026-11-24T10:00:01Z \n")).unwrap().text, "");
        let console = LogOutput::Console { message: b"2026-11-24T10:00:01Z tty\n".to_vec().into() };
        assert_eq!(log_line("job-1", console).unwrap().stream, LogStream::Out);
    }

    #[test]
    fn a_byte_that_is_not_utf8_becomes_the_replacement_character() {
        let line = log_line("job-1", frame(false, b"2026-11-24T10:00:01Z \xff\n")).unwrap();
        assert_eq!(line.text, "\u{fffd}");
    }

    #[test]
    fn a_log_frame_without_a_time_is_other() {
        for bytes in [&b"hello world\n"[..], b"", b"2026-11-24T10:00:01Z", b"\xff\xfe x\n", b"2026-13-01T00:00:00Z x"] {
            let error = log_line("job-1", frame(false, bytes)).unwrap_err();
            assert_eq!(error.kind, ErrorKind::Other, "{bytes:?}");
            assert!(error.message.starts_with("logs of job-1: "), "{error}");
        }
    }

    #[test]
    fn since_is_the_cursor_second_and_refuses_one_past_2038() {
        assert_eq!(since("job-1", None).unwrap(), 0);
        assert_eq!(since("job-1", Some("1764000000:3".parse().unwrap())).unwrap(), 1_764_000_000);
        assert_eq!(since("job-1", Some("2147483647:0".parse().unwrap())).unwrap(), i32::MAX);
        let error = since("job-1", Some("2147483648:0".parse().unwrap())).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Other, "{error}");
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
        assert_eq!(status_of(&oom), Some(ContainerStatus::Failed { code: Some(137), reason: OUT_OF_MEMORY.into() }));
        assert_eq!(OUT_OF_MEMORY, "out of memory", "the text Docker's backend gave before the constant");
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
    fn list_leaves_out_a_container_without_a_state() {
        let info = |state: Option<ContainerState>| ContainerInspectResponse {
            name: Some("/job-1".into()),
            state,
            ..Default::default()
        };
        let created = info(Some(state(ContainerStateStatusEnum::CREATED)));
        assert_eq!(summary(&created), Some(ContainerSummary::new("job-1", ContainerStatus::Created)));
        assert_eq!(summary(&info(None)), None, "no state");
        assert_eq!(summary(&info(Some(ContainerState::default()))), None, "no status");
        assert_eq!(summary(&info(Some(state(ContainerStateStatusEnum::EMPTY)))), None, "an empty status");
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
    fn a_log_read_answered_with_501_is_unsupported_and_a_500_stays_other() {
        let message = "configured logging driver does not support reading";
        let error = log_read_failed("job-1", &server(501, message));
        assert_eq!(error, ContainerError::new(ErrorKind::Unsupported, format!("logs of job-1: {message}")));
        assert!(!error.is_retryable());
        assert_eq!(log_read_failed("job-1", &server(500, "boom")).kind, ErrorKind::Other);
        assert_eq!(log_read_failed("job-1", &DockerError::RequestTimeoutError).kind, ErrorKind::DaemonUnavailable);
        assert_eq!(kind_of(&server(501, message)), ErrorKind::Other, "only a log read maps the 501");
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
    fn the_archive_stat_tells_a_folder_a_link_and_anything_else() {
        assert!(matches!(stat_of(GO_MODE_DIR | 0o755, ""), Stat::Folder));
        assert!(matches!(stat_of(GO_MODE_DIR | 0o1777, ""), Stat::Folder), "with the sticky bit");
        assert!(matches!(stat_of(GO_MODE_SYMLINK | 0o777, "/run"), Stat::Link(target) if target == "/run"));
        assert!(matches!(stat_of(0o644, ""), Stat::Other), "a regular file");
        assert!(matches!(stat_of(1 << 26 | 0o660, ""), Stat::Other), "a device");
    }

    fn make(path: &str, mode: u32, mount: Option<&str>) -> Make {
        let parts = |p: &str| p.split('/').filter(|p| !p.is_empty()).map(str::to_owned).collect::<Vec<_>>();
        Make { parts: parts(path), mode, mount: mount.map(parts) }
    }

    #[test]
    fn the_tar_of_make_dirs_holds_the_folders_parents_first_with_their_modes() {
        let made = [make("/run/app", 0o755, None), make("/run/app/sock", 0o777, None), make("/tmp/out", 0o700, None)];
        let uploads = dir_uploads("job-1", &made).unwrap();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].dir, "/");
        let mut archive = tar::Archive::new(uploads[0].tar.as_slice());
        let headers: Vec<(String, tar::EntryType, u32, u64, u64, u64)> = archive
            .entries()
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let header = entry.header();
                let path = entry.path().unwrap().display().to_string();
                let size = header.size().unwrap();
                (path, header.entry_type(), header.mode().unwrap(), header.uid().unwrap(), header.gid().unwrap(), size)
            })
            .collect();
        let folder = tar::EntryType::Directory;
        assert_eq!(
            headers,
            [
                ("run/app/".to_owned(), folder, 0o755, 0, 0, 0),
                ("run/app/sock/".to_owned(), folder, 0o777, 0, 0, 0),
                ("tmp/out/".to_owned(), folder, 0o700, 0, 0, 0),
            ]
        );
    }

    #[test]
    fn a_read_only_root_gets_one_folder_tar_per_mount() {
        let made = [
            make("/work/cache", 0o755, Some("/work")),
            make("/work/cache/a", 0o777, Some("/work")),
            make("/out/x", 0o777, Some("/out")),
        ];
        let uploads = dir_uploads("job-1", &made).unwrap();
        let dirs: Vec<&str> = uploads.iter().map(|u| u.dir.as_str()).collect();
        assert_eq!(dirs, ["/out", "/work"]);
        let paths = |tar: &[u8]| -> Vec<String> {
            let mut archive = tar::Archive::new(tar);
            archive.entries().unwrap().map(|e| e.unwrap().path().unwrap().display().to_string()).collect()
        };
        assert_eq!(paths(&uploads[0].tar), ["x/"]);
        assert_eq!(paths(&uploads[1].tar), ["cache/", "cache/a/"]);
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

    #[test]
    fn runtime_info_takes_arch_and_os_as_docker_gives_them() {
        let version = SystemVersion { arch: Some("arm64".into()), os: Some("linux".into()), ..Default::default() };
        assert_eq!(runtime_info(&version, &SystemInfo::default()).unwrap(), RuntimeInfo::new("arm64", "linux"));
    }

    #[test]
    fn runtime_info_takes_the_daemon_id_of_info_and_an_empty_one_is_none() {
        let version = SystemVersion { arch: Some("arm64".into()), os: Some("linux".into()), ..Default::default() };
        let system = SystemInfo { id: Some("4f2a:ZQ7Y".into()), ..Default::default() };
        let info = runtime_info(&version, &system).unwrap();
        assert_eq!(info, RuntimeInfo::new("arm64", "linux").with_id("4f2a:ZQ7Y"));
        assert_eq!(info.id.as_deref(), Some("4f2a:ZQ7Y"));
        let empty = SystemInfo { id: Some(String::new()), ..Default::default() };
        assert_eq!(runtime_info(&version, &empty).unwrap().id, None);
    }

    #[test]
    fn a_missing_or_empty_arch_or_os_is_other() {
        let cases = [
            (None, Some("linux"), "Arch"),
            (Some(""), Some("linux"), "Arch"),
            (Some("amd64"), None, "Os"),
            (Some("amd64"), Some(""), "Os"),
        ];
        for (arch, os, name) in cases {
            let version = SystemVersion { arch: arch.map(Into::into), os: os.map(Into::into), ..Default::default() };
            let error = runtime_info(&version, &SystemInfo::default()).unwrap_err();
            assert_eq!(error.kind, ErrorKind::Other, "{error}");
            assert_eq!(error.message, format!("docker /version gave no {name}"));
        }
    }
}
