//! The path rule of `copy_in`, shared by every runtime so they cannot drift: which paths are files, and which mount
//! of a created container a file lands on.

use crate::{ContainerError, ErrorKind};

/// The parts of an absolute `path` without empty and `.` parts; `InvalidSpec` for a relative path, a `..` part or no
/// part at all.
pub(crate) fn path_parts<'a>(name: &str, path: &'a str) -> Result<Vec<&'a str>, ContainerError> {
    let invalid = |why: &str| ContainerError::new(ErrorKind::InvalidSpec, format!("copy into {name}: {why} {path}"));
    if !path.starts_with('/') {
        return Err(invalid("relative path"));
    }
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
    if parts.contains(&"..") {
        return Err(invalid("a `..` part in"));
    }
    if parts.is_empty() {
        return Err(invalid("no file name in"));
    }
    Ok(parts)
}

/// What is mounted at a [`MountTarget`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mounted {
    /// A volume or bind mount, writable or not.
    Mount {
        /// Whether it is writable.
        writable: bool,
    },
    /// A tmpfs, which the runtime mounts only at start, over whatever was written below it.
    Tmpfs,
}

/// A mount of a created container: a volume, a bind mount or a tmpfs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MountTarget {
    /// The parts of its target, without empty and `.` parts.
    pub parts: Vec<String>,
    /// What is mounted there.
    pub mounted: Mounted,
}

impl MountTarget {
    /// The mount at the absolute `target`.
    pub(crate) fn new(target: &str, mounted: Mounted) -> Self {
        let parts = target.split('/').filter(|p| !p.is_empty() && *p != ".").map(str::to_owned).collect();
        Self { parts, mounted }
    }
}

/// Where `path` lands in the created container `name`: its parts (see [`path_parts`]) and the deepest mount strictly
/// above it, `None` for the root. Refuses with `InvalidSpec` a bad path, a path equal to a mount target (a file cannot
/// replace a mount point), a path whose deepest mount is a tmpfs (hidden at start) or a read-only mount, and with
/// `read_only_root` a path on no mount.
pub(crate) fn place<'p, 'm>(
    name: &str,
    path: &'p str,
    read_only_root: bool,
    mounts: &'m [MountTarget],
) -> Result<(Vec<&'p str>, Option<&'m MountTarget>), ContainerError> {
    let invalid = |why: &str| ContainerError::new(ErrorKind::InvalidSpec, format!("copy into {name}: {path} {why}"));
    let parts = path_parts(name, path)?;
    if mounts.iter().any(|m| m.parts == parts) {
        return Err(invalid("is the target of a mount"));
    }
    let mount = mounts
        .iter()
        .filter(|m| m.parts.len() < parts.len() && parts.iter().zip(&m.parts).all(|(a, b)| a == b))
        .max_by_key(|m| m.parts.len());
    match mount.map(|m| m.mounted) {
        Some(Mounted::Tmpfs) => return Err(invalid("is on a tmpfs, mounted only at start")),
        Some(Mounted::Mount { writable: false }) => return Err(invalid("is on a read-only mount")),
        None if read_only_root => return Err(invalid("is on the read-only root, on no volume or bind mount")),
        _ => {}
    }
    Ok((parts, mount))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RW: Mounted = Mounted::Mount { writable: true };
    const RO: Mounted = Mounted::Mount { writable: false };

    fn kind(result: Result<(Vec<&str>, Option<&MountTarget>), ContainerError>) -> ErrorKind {
        result.unwrap_err().kind
    }

    #[test]
    fn paths_must_be_absolute_without_parent_parts() {
        assert_eq!(path_parts("c", "/work//./a").unwrap(), ["work", "a"]);
        for bad in ["work/a", "/work/../etc/passwd", "/", "/./", "//", ""] {
            assert_eq!(path_parts("c", bad).unwrap_err().kind, ErrorKind::InvalidSpec, "{bad:?}");
        }
    }

    #[test]
    fn a_mount_target_is_parsed_like_a_path() {
        assert_eq!(MountTarget::new("/in/./out/", RW).parts, ["in", "out"]);
    }

    #[test]
    fn a_path_equal_to_a_mount_target_is_refused_on_any_root() {
        let mounts =
            [MountTarget::new("/work", RW), MountTarget::new("/in", RO), MountTarget::new("/scratch", Mounted::Tmpfs)];
        for read_only_root in [false, true] {
            for exact in ["/work", "/work/", "/./work/.", "/in", "/in/", "/scratch", "/scratch/"] {
                let error = place("c", exact, read_only_root, &mounts).unwrap_err();
                assert_eq!(error.kind, ErrorKind::InvalidSpec, "{exact} {read_only_root}");
            }
        }
    }

    #[test]
    fn a_path_whose_deepest_mount_is_a_tmpfs_is_refused_on_any_root() {
        let mounts = [MountTarget::new("/scratch", Mounted::Tmpfs), MountTarget::new("/scratch/vol", RW)];
        for read_only_root in [false, true] {
            assert_eq!(kind(place("c", "/scratch/c", read_only_root, &mounts)), ErrorKind::InvalidSpec);
            assert_eq!(kind(place("c", "/scratch/d/e", read_only_root, &mounts)), ErrorKind::InvalidSpec);
            let (parts, mount) = place("c", "/scratch/vol/a", read_only_root, &mounts).unwrap();
            assert_eq!(parts, ["scratch", "vol", "a"]);
            assert_eq!(mount, Some(&mounts[1]), "a volume below a tmpfs takes the file");
        }
        // A tmpfs below a volume hides what lies below it, not its siblings.
        let mounts = [MountTarget::new("/work", RW), MountTarget::new("/work/tmp", Mounted::Tmpfs)];
        assert_eq!(kind(place("c", "/work/tmp/a", false, &mounts)), ErrorKind::InvalidSpec);
        assert_eq!(place("c", "/work/a", true, &mounts).unwrap().1, Some(&mounts[0]));
    }

    #[test]
    fn the_deepest_mount_strictly_above_decides() {
        let mounts = [MountTarget::new("/work", RW), MountTarget::new("/work/cache", RW), MountTarget::new("/in", RO)];
        assert_eq!(place("c", "/work/cache/d", true, &mounts).unwrap().1, Some(&mounts[1]));
        assert_eq!(place("c", "/work/a", true, &mounts).unwrap().1, Some(&mounts[0]));
        assert_eq!(place("c", "/workspace/a", false, &mounts).unwrap().1, None, "a sibling is on the root");
        assert_eq!(kind(place("c", "/workspace/a", true, &mounts)), ErrorKind::InvalidSpec);
        assert_eq!(kind(place("c", "/in/a", false, &mounts)), ErrorKind::InvalidSpec, "a read-only mount");
        assert_eq!(kind(place("c", "/etc/b", true, &mounts)), ErrorKind::InvalidSpec, "the read-only root");
        assert_eq!(place("c", "/etc/b", false, &mounts).unwrap(), (vec!["etc", "b"], None));
        assert_eq!(kind(place("c", "relative", false, &[])), ErrorKind::InvalidSpec);
        assert_eq!(kind(place("c", "/", false, &[])), ErrorKind::InvalidSpec);
    }
}
