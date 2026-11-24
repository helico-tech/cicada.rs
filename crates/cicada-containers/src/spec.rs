//! What a caller asks the runtime for: a container, its mounts and networks, a network, a volume, files and labels.

use std::collections::BTreeMap;
use std::fmt;

/// A container to create. Make one with [`ContainerSpec::new`] and set the other fields by name; every field not set
/// is the image's or the runtime's default, and the hardening fields are off.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
pub struct ContainerSpec {
    /// The container's name: its identity, chosen by its owner. [`create`](crate::ContainerRuntime::create) keeps an
    /// existing container of this name.
    pub name: String,
    /// The image, such as `alpine:3`.
    pub image: String,
    /// Replaces the image's entrypoint.
    pub entrypoint: Option<Vec<String>>,
    /// Replaces the image's command.
    pub command: Option<Vec<String>>,
    /// The working directory.
    pub workdir: Option<String>,
    /// Plain environment variables.
    pub env: BTreeMap<String, String>,
    /// Secret environment variables: `Debug` shows their names only, and a backend never puts their values on a
    /// command line.
    pub secret_env: BTreeMap<String, String>,
    /// Labels, by which [`list`](crate::ContainerRuntime::list) and [`sweep`](crate::ContainerRuntime::sweep) find it.
    pub labels: BTreeMap<String, String>,
    /// The user (and group) the process runs as, such as `1000:1000`.
    pub user: Option<String>,
    /// A CPU limit in thousandths of a CPU: 1500 is one and a half.
    pub cpu_millis: Option<u64>,
    /// A memory limit in bytes.
    pub memory_bytes: Option<u64>,
    /// Volumes and bind mounts.
    pub mounts: Vec<Mount>,
    /// The networks it joins.
    pub network: Networking,
    /// Mounts the root file system read-only. With it, [`copy_in`](crate::ContainerRuntime::copy_in) writes only to a
    /// volume or a bind mount.
    pub read_only_root: bool,
    /// Drops every Linux capability.
    pub drop_all_capabilities: bool,
    /// Keeps the process from gaining privileges (`no-new-privileges`).
    pub no_new_privileges: bool,
    /// In-memory file systems, mounted at start.
    pub tmpfs: Vec<Tmpfs>,
    /// Runs an init process as PID 1, which forwards signals and reaps children.
    pub init: bool,
    /// The most processes the container may run.
    pub pids_limit: Option<u64>,
}

impl ContainerSpec {
    /// A container `name` of `image`, with every other field at its default.
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            entrypoint: None,
            command: None,
            workdir: None,
            env: BTreeMap::new(),
            secret_env: BTreeMap::new(),
            labels: BTreeMap::new(),
            user: None,
            cpu_millis: None,
            memory_bytes: None,
            mounts: Vec::new(),
            network: Networking::Default,
            read_only_root: false,
            drop_all_capabilities: false,
            no_new_privileges: false,
            tmpfs: Vec::new(),
            init: false,
            pids_limit: None,
        }
    }
}

impl fmt::Debug for ContainerSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContainerSpec")
            .field("name", &self.name)
            .field("image", &self.image)
            .field("entrypoint", &self.entrypoint)
            .field("command", &self.command)
            .field("workdir", &self.workdir)
            .field("env", &self.env)
            .field("secret_env", &self.secret_env.keys().collect::<Vec<_>>())
            .field("labels", &self.labels)
            .field("user", &self.user)
            .field("cpu_millis", &self.cpu_millis)
            .field("memory_bytes", &self.memory_bytes)
            .field("mounts", &self.mounts)
            .field("network", &self.network)
            .field("read_only_root", &self.read_only_root)
            .field("drop_all_capabilities", &self.drop_all_capabilities)
            .field("no_new_privileges", &self.no_new_privileges)
            .field("tmpfs", &self.tmpfs)
            .field("init", &self.init)
            .field("pids_limit", &self.pids_limit)
            .finish()
    }
}

/// A label `key=value`, by which containers, networks and volumes are listed and swept.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Label {
    /// The label's key.
    pub key: String,
    /// The label's value.
    pub value: String,
}

impl Label {
    /// The label `key=value`.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { key: key.into(), value: value.into() }
    }
}

/// A volume or bind mount of a container.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// A named volume or a host path.
    pub kind: MountKind,
    /// The volume's name, or the host path of a bind mount.
    pub source: String,
    /// The absolute path in the container.
    pub target: String,
    /// Mounts it read-only.
    pub read_only: bool,
}

impl Mount {
    /// A writable mount of `source` at `target`.
    pub fn new(kind: MountKind, source: impl Into<String>, target: impl Into<String>) -> Self {
        Self { kind, source: source.into(), target: target.into(), read_only: false }
    }
}

/// What a [`Mount`] mounts.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    /// A named volume of the runtime.
    Volume,
    /// A path of the host.
    Bind,
}

/// The networks a container joins.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Networking {
    /// The runtime's default network.
    #[default]
    Default,
    /// No network at all.
    None,
    /// These networks, each with its aliases.
    Attach(Vec<NetworkAttachment>),
}

/// A network a container joins, and the names it has there besides its own.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkAttachment {
    /// The network's name.
    pub name: String,
    /// Further names of the container on that network.
    pub aliases: Vec<String>,
}

impl NetworkAttachment {
    /// The network `name`, without aliases.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), aliases: Vec::new() }
    }
}

/// An in-memory file system of a container, mounted at start.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tmpfs {
    /// The absolute path in the container.
    pub target: String,
    /// Its size limit in bytes; the runtime's default when not set.
    pub size_bytes: Option<u64>,
    /// Its permission bits, such as `0o1777`.
    pub mode: Option<u32>,
    /// The user that owns it.
    pub uid: Option<u32>,
    /// The group that owns it.
    pub gid: Option<u32>,
    /// Allows running programs from it; when off, the backend's default applies (Docker's is `noexec`).
    pub exec: bool,
}

impl Tmpfs {
    /// A tmpfs at `target` with the runtime's defaults.
    pub fn new(target: impl Into<String>) -> Self {
        Self { target: target.into(), size_bytes: None, mode: None, uid: None, gid: None, exec: false }
    }
}

/// A network for [`ensure_network`](crate::ContainerRuntime::ensure_network).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkSpec {
    /// The network's name: its identity.
    pub name: String,
    /// Its labels.
    pub labels: BTreeMap<String, String>,
    /// Gives it no route outside: its containers reach only each other.
    pub internal: bool,
}

impl NetworkSpec {
    /// A network `name` without labels, not internal.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), labels: BTreeMap::new(), internal: false }
    }
}

/// A volume for [`ensure_volume`](crate::ContainerRuntime::ensure_volume).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeSpec {
    /// The volume's name: its identity.
    pub name: String,
    /// Its labels.
    pub labels: BTreeMap<String, String>,
}

impl VolumeSpec {
    /// A volume `name` without labels.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), labels: BTreeMap::new() }
    }
}

/// A file for [`copy_in`](crate::ContainerRuntime::copy_in). `Debug` shows its path, mode and length, not its contents.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
pub struct CopyFile {
    /// The absolute path in the container.
    pub path: String,
    /// The file's contents.
    pub contents: Vec<u8>,
    /// Its permission bits.
    pub mode: u32,
}

impl CopyFile {
    /// A file at `path` with `contents` and mode `0o644`.
    pub fn new(path: impl Into<String>, contents: impl Into<Vec<u8>>) -> Self {
        Self { path: path.into(), contents: contents.into(), mode: 0o644 }
    }
}

impl fmt::Debug for CopyFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CopyFile")
            .field("path", &self.path)
            .field("mode", &format_args!("{:#o}", self.mode))
            .field("len", &self.contents.len())
            .finish()
    }
}

/// A folder for [`make_dirs`](crate::ContainerRuntime::make_dirs): its absolute path and the mode it is made with when
/// it is missing.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyDir {
    /// The absolute path in the container.
    pub path: String,
    /// Its permission bits, at most `0o7777`; an existing folder keeps its own.
    pub mode: u32,
}

impl CopyDir {
    /// A folder at `path` with mode `0o755`.
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into(), mode: 0o755 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_of_a_spec_with_a_secret_shows_its_name_only() {
        let mut spec = ContainerSpec::new("job-1", "alpine:3");
        spec.secret_env.insert("TOKEN".into(), "s3cret".into());
        let debug = format!("{spec:?}");
        assert!(debug.contains("TOKEN"), "{debug}");
        assert!(!debug.contains("s3cret"), "{debug}");
        assert!(!format!("{spec:#?}").contains("s3cret"));
    }

    #[test]
    fn debug_output_of_a_file_shows_no_contents() {
        let file = CopyFile::new("/work/token", "s3cret");
        assert_eq!(format!("{file:?}"), r#"CopyFile { path: "/work/token", mode: 0o644, len: 6 }"#);
    }

    #[test]
    fn a_new_folder_has_mode_0o755() {
        let dir = CopyDir::new("/out");
        assert_eq!((dir.path.as_str(), dir.mode), ("/out", 0o755));
    }

    #[test]
    fn a_new_spec_is_plain() {
        let spec = ContainerSpec::new("job-1", "alpine:3");
        assert_eq!((spec.name.as_str(), spec.image.as_str()), ("job-1", "alpine:3"));
        assert_eq!((spec.entrypoint, spec.command, spec.workdir, spec.user), (None, None, None, None));
        assert!(spec.env.is_empty() && spec.secret_env.is_empty() && spec.labels.is_empty());
        assert!(spec.mounts.is_empty());
        assert_eq!(spec.network, Networking::Default);
        assert_eq!((spec.cpu_millis, spec.memory_bytes, spec.pids_limit), (None, None, None));
        assert!(!spec.read_only_root && !spec.drop_all_capabilities && !spec.no_new_privileges && !spec.init);
        assert!(spec.tmpfs.is_empty());
    }
}
