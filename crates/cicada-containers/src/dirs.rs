//! The rule of `make_dirs`, shared by every runtime so they cannot drift: the check of the named folders, the walk
//! that finds what exists through a runtime's stat and follows links to a folder, and the folders to make.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;

use crate::paths::{MountTarget, place_as};
use crate::{ContainerError, CopyDir, ErrorKind};

/// The action that starts every message of `make_dirs`.
pub(crate) const MAKE: &str = "make folders in";

/// The most links the walk of one named folder follows; one more is `Conflict`.
pub(crate) const MAX_LINKS: usize = 8;

/// The mode of a missing parent that is not named itself.
const PARENT_MODE: u32 = 0o755;

/// What a runtime's stat found at an absolute path that passes no link: the stat does not follow its last part.
#[derive(Debug)]
pub(crate) enum Stat {
    /// A folder.
    Folder,
    /// A symbolic link, with its target as stored. The fake has none.
    #[cfg_attr(not(feature = "docker"), allow(dead_code))]
    Link(String),
    /// A file, or anything else that is neither a folder nor a link.
    Other,
    /// Nothing.
    Missing,
    /// The stat failed; the walk fails with this error. The fake's never does.
    #[cfg_attr(not(feature = "docker"), allow(dead_code))]
    Failed(ContainerError),
}

/// The folders `dirs` names in the created container `name`, checked before any call: each as its parts with its
/// mode, in path order, once. `InvalidSpec` for a mode above `0o7777`, a path the shared rule refuses (see
/// [`place_as`]), and one path named twice with two modes.
pub(crate) fn named(
    name: &str,
    dirs: &[CopyDir],
    read_only_root: bool,
    mounts: &[MountTarget],
) -> Result<Vec<(Vec<String>, u32)>, ContainerError> {
    let invalid = |why: String| ContainerError::new(ErrorKind::InvalidSpec, format!("{MAKE} {name}: {why}"));
    let mut named: BTreeMap<Vec<String>, u32> = BTreeMap::new();
    for dir in dirs {
        if dir.mode > 0o7777 {
            return Err(invalid(format!("mode {:#o} of {} is above 0o7777", dir.mode, dir.path)));
        }
        let (parts, _) = place_as(MAKE, name, &dir.path, read_only_root, mounts)?;
        let parts = parts.into_iter().map(str::to_owned).collect();
        match named.entry(parts) {
            Entry::Vacant(vacant) => {
                vacant.insert(dir.mode);
            }
            Entry::Occupied(taken) if *taken.get() != dir.mode => {
                return Err(invalid(format!(
                    "{} is named with modes {:#o} and {:#o}",
                    dir.path,
                    taken.get(),
                    dir.mode
                )));
            }
            Entry::Occupied(_) => {}
        }
    }
    Ok(named.into_iter().collect())
}

/// Where a named folder lands: the deepest existing folder on its way, by its path without links, and the named parts
/// below it, which are missing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// The parts of the existing folder, which passes no link; empty for the root.
    pub existing: Vec<String>,
    /// The missing parts below it, the named folder last; empty when the named folder exists.
    pub missing: Vec<String>,
}

/// Walks the named folder `parts` of the container `name` from the root, one `stat` per part, each on a path that
/// passes no link. A folder goes on; a link goes on at its target, against the link's folder when relative, with `..`
/// stopping at the root, up to [`MAX_LINKS`]; the first missing named part ends the walk. `Conflict` for a file at a
/// part, a link's target that is missing or not a folder, and one link too many; a failed stat fails the walk.
pub(crate) async fn walk<S, F>(name: &str, parts: &[String], mut stat: S) -> Result<Found, ContainerError>
where
    S: FnMut(String) -> F,
    F: Future<Output = Stat>,
{
    let named = format!("/{}", parts.join("/"));
    let conflict = |why: String| ContainerError::new(ErrorKind::Conflict, format!("{MAKE} {name}: {named}: {why}"));
    let mut existing: Vec<String> = Vec::new();
    // The parts still to walk, each with whether it comes from a link's target, which must exist.
    let mut left: VecDeque<(String, bool)> = parts.iter().map(|part| (part.clone(), false)).collect();
    let mut links = 0;
    while let Some((part, from_link)) = left.pop_front() {
        match part.as_str() {
            "" | "." => continue,
            ".." => {
                existing.pop();
                continue;
            }
            _ => {}
        }
        existing.push(part);
        let path = format!("/{}", existing.join("/"));
        match stat(path.clone()).await {
            Stat::Folder => {}
            Stat::Link(target) => {
                links += 1;
                if links > MAX_LINKS {
                    return Err(conflict(format!("more than {MAX_LINKS} links on the way")));
                }
                existing.pop();
                if target.starts_with('/') {
                    existing.clear();
                }
                for part in target.split('/').rev() {
                    left.push_front((part.to_owned(), true));
                }
            }
            Stat::Other => return Err(conflict(format!("{path} is a file, not a folder"))),
            Stat::Missing if from_link => return Err(conflict(format!("{path}, the target of a link, is missing"))),
            Stat::Missing => {
                let part = existing.pop().into_iter();
                // Every part a link brought is walked before the named parts after it: the rest are named.
                let missing = part.chain(left.into_iter().map(|(part, _)| part)).collect();
                return Ok(Found { existing, missing });
            }
            Stat::Failed(error) => return Err(error),
        }
    }
    Ok(Found { existing, missing: Vec::new() })
}

/// The folders to make, by their parts without links, parents first: each with its mode and whether it is named.
#[derive(Debug, Default)]
pub(crate) struct ToMake {
    dirs: BTreeMap<Vec<String>, (u32, bool)>,
}

/// A folder to make: its parts without links, its mode and the parts of the mount that holds it with a read-only root.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Make {
    /// Its parts, without links.
    pub parts: Vec<String>,
    /// Its mode.
    pub mode: u32,
    /// With a read-only root, the parts of the deepest mount above it; else `None`.
    pub mount: Option<Vec<String>>,
}

impl ToMake {
    /// Adds the missing folders of `found` for the named folder with `mode`: the named one with `mode`, its missing
    /// parents with `0o755`. A named mode beats the implied `0o755`, in any order; two named modes for one folder,
    /// which a link can bring, are `InvalidSpec`.
    pub(crate) fn add(&mut self, name: &str, found: Found, mode: u32) -> Result<(), ContainerError> {
        let mut parts = found.existing;
        let last = found.missing.len();
        for (at, part) in found.missing.into_iter().enumerate() {
            parts.push(part);
            let named = at + 1 == last;
            let wanted = (if named { mode } else { PARENT_MODE }, named);
            match self.dirs.entry(parts.clone()) {
                Entry::Vacant(vacant) => {
                    vacant.insert(wanted);
                }
                Entry::Occupied(mut taken) => match (*taken.get(), named) {
                    ((other, true), true) if other != mode => {
                        return Err(ContainerError::new(
                            ErrorKind::InvalidSpec,
                            format!("{MAKE} {name}: /{} is named with modes {other:#o} and {mode:#o}", parts.join("/")),
                        ));
                    }
                    ((_, false), true) => {
                        taken.insert(wanted);
                    }
                    _ => {}
                },
            }
        }
        Ok(())
    }

    /// The folders to make, parents first, each checked by the shared path rule (see [`place_as`]) on its path
    /// without links, with its mount for a read-only root.
    pub(crate) fn finish(
        self,
        name: &str,
        read_only_root: bool,
        mounts: &[MountTarget],
    ) -> Result<Vec<Make>, ContainerError> {
        self.dirs
            .into_iter()
            .map(|(parts, (mode, _))| {
                let path = format!("/{}", parts.join("/"));
                let (_, mount) = place_as(MAKE, name, &path, read_only_root, mounts)?;
                let mount = mount.filter(|_| read_only_root).map(|m| m.parts.clone());
                Ok(Make { parts, mode, mount })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::task::{Context, Poll, Waker};

    use super::*;
    use crate::paths::Mounted;

    /// What a stat finds in a test: `d` a folder, `f` a file, `>target` a link.
    fn tree(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries.iter().map(|(path, what)| ((*path).to_owned(), (*what).to_owned())).collect()
    }

    fn walk_in(tree: &HashMap<String, String>, path: &str) -> Result<Found, ContainerError> {
        let parts: Vec<String> = path.split('/').filter(|p| !p.is_empty()).map(str::to_owned).collect();
        let stat = |path: String| {
            std::future::ready(match tree.get(&path).map(String::as_str) {
                Some("d") => Stat::Folder,
                Some("f") => Stat::Other,
                Some("err") => Stat::Failed(ContainerError::new(ErrorKind::Other, format!("stat {path}"))),
                Some(link) => Stat::Link(link.trim_start_matches('>').to_owned()),
                None => Stat::Missing,
            })
        };
        let mut walk = std::pin::pin!(walk("c", &parts, stat));
        match walk.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(found) => found,
            Poll::Pending => panic!("the walk waited"),
        }
    }

    fn found(existing: &str, missing: &str) -> Found {
        let parts = |s: &str| s.split('/').filter(|p| !p.is_empty()).map(str::to_owned).collect();
        Found { existing: parts(existing), missing: parts(missing) }
    }

    fn kind(result: Result<Found, ContainerError>) -> ErrorKind {
        result.unwrap_err().kind
    }

    #[test]
    fn named_folders_are_checked_and_kept_once_in_path_order() {
        let dirs = [CopyDir { mode: 0o777, ..CopyDir::new("/b/./c") }, CopyDir::new("/a"), CopyDir::new("/a/")];
        assert_eq!(
            named("c", &dirs, false, &[]).unwrap(),
            [(vec!["a".to_owned()], 0o755), (vec!["b".to_owned(), "c".to_owned()], 0o777)]
        );
        let two_modes = [CopyDir { mode: 0o777, ..CopyDir::new("/out") }, CopyDir::new("/out/")];
        let error = named("c", &two_modes, false, &[]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidSpec);
        assert!(error.message.starts_with("make folders in c: /out/ is named with modes 0o777 and 0o755"), "{error}");
        let bad_mode = [CopyDir { mode: 0o10000, ..CopyDir::new("/out") }];
        assert_eq!(named("c", &bad_mode, false, &[]).unwrap_err().kind, ErrorKind::InvalidSpec);
        let top = [CopyDir { mode: 0o7777, ..CopyDir::new("/out") }];
        named("c", &top, false, &[]).unwrap();
        let work = [MountTarget::new("/work", Mounted::Mount { writable: true })];
        for bad in ["out", "/", "/a/../b", "/work"] {
            assert_eq!(named("c", &[CopyDir::new(bad)], false, &work).unwrap_err().kind, ErrorKind::InvalidSpec);
        }
        assert_eq!(named("c", &[CopyDir::new("/etc/x")], true, &work).unwrap_err().kind, ErrorKind::InvalidSpec);
    }

    #[test]
    fn the_walk_stops_at_the_first_missing_part() {
        let tree = tree(&[("/tmp", "d"), ("/data", "d"), ("/data/in", "d")]);
        assert_eq!(walk_in(&tree, "/tmp/out/a").unwrap(), found("/tmp", "out/a"));
        assert_eq!(walk_in(&tree, "/data/in").unwrap(), found("/data/in", ""), "it exists");
        assert_eq!(walk_in(&tree, "/new").unwrap(), found("", "new"), "below the root");
    }

    #[test]
    fn a_file_on_the_way_is_a_conflict() {
        let tree = tree(&[("/etc", "d"), ("/etc/passwd", "f")]);
        let error = walk_in(&tree, "/etc/passwd/x").unwrap_err();
        assert_eq!(error.kind, ErrorKind::Conflict);
        assert_eq!(error.message, "make folders in c: /etc/passwd/x: /etc/passwd is a file, not a folder");
        assert_eq!(kind(walk_in(&tree, "/etc/passwd")), ErrorKind::Conflict, "at the path itself");
    }

    #[test]
    fn a_failed_stat_fails_the_walk() {
        let tree = tree(&[("/a", "err")]);
        assert_eq!(kind(walk_in(&tree, "/a/b")), ErrorKind::Other);
    }

    #[test]
    fn an_absolute_link_is_followed_to_its_folder() {
        let tree = tree(&[("/run", "d"), ("/run/postgresql", "d"), ("/var", "d"), ("/var/run", ">/run")]);
        assert_eq!(walk_in(&tree, "/var/run/app").unwrap(), found("/run", "app"));
        assert_eq!(walk_in(&tree, "/var/run/postgresql/sock").unwrap(), found("/run/postgresql", "sock"));
        assert_eq!(walk_in(&tree, "/var/run").unwrap(), found("/run", ""), "a link to a folder is that folder");
    }

    #[test]
    fn a_relative_link_resolves_against_its_folder_and_dot_dot_stops_at_the_root() {
        let tree = tree(&[
            ("/srv", "d"),
            ("/srv/data", "d"),
            ("/a", "d"),
            ("/a/b", "d"),
            ("/a/b/sibling", ">../../srv/./data"),
            ("/a/b/up", ">../../../../srv"),
            ("/a/b/here", ">data"),
            ("/a/b/data", "d"),
        ]);
        assert_eq!(walk_in(&tree, "/a/b/sibling/x").unwrap(), found("/srv/data", "x"));
        assert_eq!(walk_in(&tree, "/a/b/up/x").unwrap(), found("/srv", "x"), "`..` stops at the root");
        assert_eq!(walk_in(&tree, "/a/b/here/x").unwrap(), found("/a/b/data", "x"));
    }

    #[test]
    fn a_link_to_no_folder_is_a_conflict() {
        let tree = tree(&[("/a", ">/missing"), ("/f", "f"), ("/b", ">/f"), ("/c", ">/missing/deeper")]);
        for path in ["/a", "/a/x", "/b/x", "/c/x"] {
            assert_eq!(kind(walk_in(&tree, path)), ErrorKind::Conflict, "{path}");
        }
    }

    #[test]
    fn eight_links_in_a_row_are_followed_and_a_ninth_is_a_conflict() {
        let mut entries: Vec<(String, String)> = (1..=8).map(|i| (format!("/l{i}"), format!(">/l{}", i + 1))).collect();
        entries.push(("/l9".into(), "d".into()));
        let tree: HashMap<String, String> = entries.into_iter().collect();
        assert_eq!(walk_in(&tree, "/l2/x").unwrap(), found("/l9", "x"), "seven links");
        assert_eq!(walk_in(&tree, "/l1/x").unwrap(), found("/l9", "x"), "eight links");
        let mut tree = tree;
        tree.insert("/l0".into(), ">/l1".into());
        let error = walk_in(&tree, "/l0/x").unwrap_err();
        assert_eq!(error.kind, ErrorKind::Conflict);
        assert!(error.message.contains("more than 8 links"), "{error}");
        tree.insert("/loop".into(), ">/loop".into());
        assert_eq!(kind(walk_in(&tree, "/loop/x")), ErrorKind::Conflict, "a loop ends");
    }

    fn make(parts: &str, mode: u32) -> Make {
        Make { parts: parts.split('/').filter(|p| !p.is_empty()).map(str::to_owned).collect(), mode, mount: None }
    }

    #[test]
    fn a_named_mode_beats_the_implied_one_in_any_order() {
        for reversed in [false, true] {
            let mut adds = vec![(found("", "a/b"), 0o755), (found("", "a"), 0o777)];
            if reversed {
                adds.reverse();
            }
            let mut to_make = ToMake::default();
            for (found, mode) in adds {
                to_make.add("c", found, mode).unwrap();
            }
            assert_eq!(to_make.finish("c", false, &[]).unwrap(), [make("/a", 0o777), make("/a/b", 0o755)]);
        }
    }

    #[test]
    fn missing_parents_get_0o755_and_come_first() {
        let mut to_make = ToMake::default();
        to_make.add("c", found("/tmp", "out/a/b"), 0o777).unwrap();
        to_make.add("c", found("/tmp", ""), 0o700).unwrap();
        let expected = [make("/tmp/out", 0o755), make("/tmp/out/a", 0o755), make("/tmp/out/a/b", 0o777)];
        assert_eq!(to_make.finish("c", false, &[]).unwrap(), expected, "an existing folder is not made");
    }

    #[test]
    fn two_named_modes_through_a_link_are_invalid() {
        let mut to_make = ToMake::default();
        to_make.add("c", found("/run", "app"), 0o777).unwrap();
        to_make.add("c", found("/run", "app"), 0o777).unwrap();
        assert_eq!(to_make.add("c", found("/run", "app"), 0o755).unwrap_err().kind, ErrorKind::InvalidSpec);
    }

    #[test]
    fn the_path_rule_applies_to_the_folders_to_make() {
        let mounts = [
            MountTarget::new("/work", Mounted::Mount { writable: true }),
            MountTarget::new("/scratch", Mounted::Tmpfs),
        ];
        let mut to_make = ToMake::default();
        to_make.add("c", found("/work", "cache/a"), 0o777).unwrap();
        let made = to_make.finish("c", true, &mounts).unwrap();
        let work = Some(vec!["work".to_owned()]);
        assert_eq!(made[0], Make { mount: work.clone(), ..make("/work/cache", 0o755) });
        assert_eq!(made[1], Make { mount: work, ..make("/work/cache/a", 0o777) });
        // A link that leads onto the read-only root or a tmpfs.
        for (existing, read_only_root) in [("/etc", true), ("/scratch", false)] {
            let mut to_make = ToMake::default();
            to_make.add("c", found(existing, "x"), 0o755).unwrap();
            let error = to_make.finish("c", read_only_root, &mounts).unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidSpec, "{existing}");
        }
    }
}
