//! Copying out: [`CopyLimits`], [`CopyOut`] and [`SkippedFile`], and the private collector that both runtimes feed
//! with the entries of a folder in order, so they keep and skip the same files.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::paths::path_parts;
use crate::{ContainerError, CopyFile, ErrorKind};

/// The limits of one [`copy_out`](crate::ContainerRuntime::copy_out): how many entries its result holds, kept and
/// skipped files together, how many bytes of contents it keeps, and how many it reads. None may be zero.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CopyLimits {
    /// The most entries of [`CopyOut::files`] and [`CopyOut::skipped`] together, besides the one entry that says the
    /// rest was not read.
    pub max_files: u32,
    /// The most bytes of contents of all kept files together.
    pub max_bytes: u64,
    /// The most bytes of contents of regular files the copy reads, kept and skipped alike: a skipped file's data
    /// passes through Docker's archive all the same. A file that would take the count past it stops the copy, as
    /// the file limit does.
    pub max_read: u64,
}

impl CopyLimits {
    /// At most `max_files` entries and `max_bytes` bytes of contents, reading at most four times `max_bytes`
    /// (saturating).
    pub const fn new(max_files: u32, max_bytes: u64) -> Self {
        Self { max_files, max_bytes, max_read: max_bytes.saturating_mul(4) }
    }

    /// These limits reading at most `bytes` bytes of contents.
    pub const fn with_max_read(self, bytes: u64) -> Self {
        Self { max_read: bytes, ..self }
    }
}

/// What a [`copy_out`](crate::ContainerRuntime::copy_out) read: the files it kept, whole, and those it skipped.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopyOut {
    /// The kept files in the order the runtime read them, each with its absolute path, contents and permission bits.
    pub files: Vec<CopyFile>,
    /// What it left, with the reason: a file over the bytes left, a link or another kind of file, an entry outside
    /// the folder, and last, once the file limit is reached, one entry for the folder.
    pub skipped: Vec<SkippedFile>,
}

impl CopyOut {
    /// A result of `files` kept and `skipped` left.
    pub fn new(files: Vec<CopyFile>, skipped: Vec<SkippedFile>) -> Self {
        Self { files, skipped }
    }
}

/// An entry a [`copy_out`](crate::ContainerRuntime::copy_out) left out, or a file an
/// [`OutputStore`](crate::OutputStore) refused; in an [`OutputsReport`](crate::OutputsReport) as
/// `{"path": …, "reason": …}`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedFile {
    /// Its absolute path in the container, or the archive's name for an entry outside the folder.
    pub path: String,
    /// Why it is left, such as its size and the bytes left.
    pub reason: String,
}

impl SkippedFile {
    /// The entry at `path`, left for `reason`.
    pub fn new(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self { path: path.into(), reason: reason.into() }
    }
}

/// The kind of an entry of a folder. The fake has only files and folders; Docker's archive has the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(test, feature = "docker")), allow(dead_code))]
pub(crate) enum EntryKind {
    /// A regular file.
    File,
    /// A folder, which is not listed: the paths of its files show it.
    Folder,
    /// A symbolic link to its target.
    Symlink(String),
    /// A hard link to an earlier entry.
    HardLink(String),
    /// A device, a pipe, a socket or any other kind, named for the reason.
    Other(&'static str),
}

/// Applies the limits and the link rule to the entries of the folder or file `path`, in order (design, decisions 1, 3
/// and 7). Entries are named as in Docker's archive: relative to the folder that holds `path`, so `out/a` for the
/// file `/out/a` of `/out`.
pub(crate) struct Collector {
    /// The parts of the parent of `path`, which entry names are relative to.
    parent: Vec<String>,
    /// The last part of `path`, which every entry name under it starts with.
    base: String,
    /// `path` cleaned, for the entry that says the rest was not read.
    root: String,
    max_files: u32,
    left: u64,
    max_read: u64,
    /// What is left of `max_read`.
    read_left: u64,
    out: CopyOut,
    stopped: bool,
}

impl Collector {
    /// A collector for `path` of the container `name`; `InvalidSpec` for a path the shared rule refuses or a zero
    /// limit.
    pub(crate) fn new(name: &str, path: &str, limits: CopyLimits) -> Result<Self, ContainerError> {
        let mut parts = path_parts("copy out of", name, path)?;
        if limits.max_files == 0 || limits.max_bytes == 0 || limits.max_read == 0 {
            return Err(ContainerError::new(
                ErrorKind::InvalidSpec,
                format!(
                    "copy out of {name}: limits of {} files, {} bytes and {} bytes read; none may be zero",
                    limits.max_files, limits.max_bytes, limits.max_read
                ),
            ));
        }
        let root = format!("/{}", parts.join("/"));
        let base = parts.pop().unwrap_or_default().to_owned(); // `path_parts` gives at least one part
        Ok(Self {
            parent: parts.into_iter().map(str::to_owned).collect(),
            base,
            root,
            max_files: limits.max_files,
            left: limits.max_bytes,
            max_read: limits.max_read,
            read_left: limits.max_read,
            out: CopyOut::default(),
            stopped: false,
        })
    }

    /// `path` cleaned: absolute, without empty and `.` parts.
    pub(crate) fn root(&self) -> &str {
        &self.root
    }

    /// The entry `name` of `kind`: kept, skipped or passed over. A regular file of `size` bytes is kept when it fits
    /// the bytes left: `read` then fills a buffer of exactly that size. Every regular file, kept or skipped, counts
    /// against the read limit. Returns `false` once the file limit or the read limit stops the copy: the caller reads
    /// no more entries. An error of `read` is returned as it is.
    pub(crate) fn offer<E>(
        &mut self,
        name: &str,
        kind: EntryKind,
        size: u64,
        mode: u32,
        read: impl FnOnce(&mut [u8]) -> Result<(), E>,
    ) -> Result<bool, E> {
        if self.stopped {
            return Ok(false);
        }
        if kind == EntryKind::Folder {
            return Ok(true);
        }
        if self.out.files.len() + self.out.skipped.len() >= self.max_files as usize {
            let reason = format!("more than {} files; the rest was not read", self.max_files);
            self.out.skipped.push(SkippedFile::new(self.root.clone(), reason));
            self.stopped = true;
            return Ok(false);
        }
        if kind == EntryKind::File {
            if size > self.read_left {
                let reason = format!("the rest was not read: over the read limit of {} bytes", self.max_read);
                self.out.skipped.push(SkippedFile::new(self.root.clone(), reason));
                self.stopped = true;
                return Ok(false);
            }
            self.read_left -= size;
        }
        let Some(path) = self.absolute(name) else {
            self.out.skipped.push(SkippedFile::new(name, "outside the folder"));
            return Ok(true);
        };
        let reason = match kind {
            EntryKind::File => match usize::try_from(size).ok().filter(|_| size <= self.left) {
                Some(len) => {
                    let mut contents = vec![0; len];
                    if len > 0 {
                        read(&mut contents)?;
                    }
                    self.left -= size;
                    self.out.files.push(CopyFile { path, contents, mode: mode & 0o7777 });
                    return Ok(true);
                }
                None => format!("{size} bytes, more than the {} left", self.left),
            },
            EntryKind::Symlink(target) => format!("a symbolic link to {target}, not followed"),
            EntryKind::HardLink(target) => format!("a hard link to {target}, not followed"),
            EntryKind::Other(what) => format!("{what}, not a regular file"),
            EntryKind::Folder => unreachable!("folders return above"),
        };
        self.out.skipped.push(SkippedFile::new(path, reason));
        Ok(true)
    }

    /// What was kept and skipped.
    pub(crate) fn finish(self) -> CopyOut {
        self.out
    }

    /// The absolute path of the entry `name`; `None` when it is absolute, has a `..` part or is not under `path`.
    fn absolute(&self, name: &str) -> Option<String> {
        if name.starts_with('/') {
            return None;
        }
        let parts: Vec<&str> = name.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
        if parts.first() != Some(&self.base.as_str()) || parts.contains(&"..") {
            return None;
        }
        let mut path = String::new();
        for part in self.parent.iter().map(String::as_str).chain(parts) {
            path.push('/');
            path.push_str(part);
        }
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    fn collector(path: &str, max_files: u32, max_bytes: u64) -> Collector {
        Collector::new("c", path, CopyLimits::new(max_files, max_bytes)).unwrap()
    }

    /// Offers a regular file whose contents are `size` copies of `byte`.
    fn file(collector: &mut Collector, name: &str, size: usize, byte: u8) -> bool {
        collector
            .offer(name, EntryKind::File, size as u64, 0o644, |buf: &mut [u8]| {
                assert_eq!(buf.len(), size, "a buffer of exactly the file's size");
                buf.fill(byte);
                Ok::<_, Infallible>(())
            })
            .unwrap()
    }

    fn other(collector: &mut Collector, name: &str, kind: EntryKind) -> bool {
        collector.offer(name, kind, 0, 0o777, |_: &mut [u8]| -> Result<(), Infallible> { panic!("not read") }).unwrap()
    }

    fn paths(out: &CopyOut) -> (Vec<&str>, Vec<(&str, &str)>) {
        let files = out.files.iter().map(|f| f.path.as_str()).collect();
        let skipped = out.skipped.iter().map(|s| (s.path.as_str(), s.reason.as_str())).collect();
        (files, skipped)
    }

    #[test]
    fn a_file_over_the_bytes_left_is_skipped_and_the_copy_goes_on() {
        let mut c = Collector::new("c", "/out", CopyLimits::new(10, 100).with_max_read(10_000)).unwrap();
        assert!(other(&mut c, "out/", EntryKind::Folder));
        assert!(file(&mut c, "out/a", 10, b'a'));
        assert!(file(&mut c, "out/big", 5000, b'x'));
        assert!(file(&mut c, "out/b", 20, b'b'));
        assert!(file(&mut c, "out/c", 71, b'c'), "71 bytes do not fit the 70 left");
        assert!(file(&mut c, "out/d", 70, b'd'), "70 do");
        let out = c.finish();
        assert_eq!(
            paths(&out),
            (
                vec!["/out/a", "/out/b", "/out/d"],
                vec![("/out/big", "5000 bytes, more than the 90 left"), ("/out/c", "71 bytes, more than the 70 left")]
            )
        );
        assert_eq!(out.files[0].contents, vec![b'a'; 10], "whole");
        assert_eq!(out.files[0].mode, 0o644);
    }

    #[test]
    fn the_file_limit_stops_at_the_next_countable_entry_with_one_last_entry() {
        let mut c = collector("/out", 2, 100);
        assert!(file(&mut c, "out/a", 1, b'a'));
        assert!(other(&mut c, "out/l", EntryKind::Symlink("a".into())));
        assert!(other(&mut c, "out/sub/", EntryKind::Folder), "a folder is not counted");
        assert!(!file(&mut c, "out/sub/b", 1, b'b'));
        assert!(!file(&mut c, "out/sub/c", 1, b'c'), "stopped");
        let out = c.finish();
        assert_eq!(out.files.len() + out.skipped.len(), 3);
        assert_eq!(out.skipped.last().unwrap(), &SkippedFile::new("/out", "more than 2 files; the rest was not read"));
    }

    #[test]
    fn exactly_at_the_file_limit_there_is_no_last_entry() {
        let mut c = collector("/out/", 2, 100);
        assert!(file(&mut c, "out/a", 1, b'a'));
        assert!(file(&mut c, "out/b", 1, b'b'));
        assert!(other(&mut c, "out/sub/", EntryKind::Folder), "a folder after the limit does not stop it");
        let out = c.finish();
        assert_eq!(paths(&out), (vec!["/out/a", "/out/b"], vec![]));
    }

    #[test]
    fn the_read_limit_counts_skipped_files_and_stops_at_the_file_past_it() {
        let limits = CopyLimits::new(10, 20).with_max_read(100);
        let mut c = Collector::new("c", "/out", limits).unwrap();
        assert!(file(&mut c, "out/a", 10, b'a'));
        assert!(file(&mut c, "out/big", 60, b'x'), "skipped over the bytes left, but read");
        assert!(other(&mut c, "out/l", EntryKind::Symlink("a".into())), "no contents, not counted");
        assert!(file(&mut c, "out/b", 30, b'b'), "exactly at the read limit");
        assert!(file(&mut c, "out/empty", 0, b'e'), "an empty file reads nothing");
        assert!(!file(&mut c, "out/c", 1, b'c'), "one byte past it");
        assert!(!file(&mut c, "out/d", 1, b'd'), "stopped");
        let out = c.finish();
        assert_eq!(
            paths(&out),
            (
                vec!["/out/a", "/out/empty"],
                vec![
                    ("/out/big", "60 bytes, more than the 10 left"),
                    ("/out/l", "a symbolic link to a, not followed"),
                    ("/out/b", "30 bytes, more than the 10 left"),
                    ("/out", "the rest was not read: over the read limit of 100 bytes"),
                ]
            )
        );
    }

    #[test]
    fn the_read_limit_is_four_times_the_bytes_kept_by_default() {
        assert_eq!(CopyLimits::new(1, 100).max_read, 400);
        assert_eq!(CopyLimits::new(1, u64::MAX).max_read, u64::MAX, "saturating");
        assert_eq!(CopyLimits::new(1, 100).with_max_read(7).max_read, 7);
    }

    #[test]
    fn a_zero_limit_or_a_bad_path_is_invalid() {
        for (limits, path) in [
            (CopyLimits::new(0, 1), "/out"),
            (CopyLimits::new(1, 0), "/out"),
            (CopyLimits::new(1, 1).with_max_read(0), "/out"),
            (CopyLimits::new(1, 1), "out"),
            (CopyLimits::new(1, 1), "/"),
        ] {
            let error = Collector::new("c", path, limits).err().unwrap();
            assert_eq!(error.kind, ErrorKind::InvalidSpec, "{limits:?} {path}");
            assert!(error.message.starts_with("copy out of c: "), "{error}");
        }
    }

    #[test]
    fn links_and_other_kinds_are_skipped_with_a_reason() {
        let mut c = collector("/out", 10, 100);
        other(&mut c, "out/out", EntryKind::Symlink("/etc/passwd".into()));
        other(&mut c, "out/in", EntryKind::Symlink("a".into()));
        other(&mut c, "out/h", EntryKind::HardLink("out/a".into()));
        other(&mut c, "out/null", EntryKind::Other("a device"));
        other(&mut c, "out/fifo", EntryKind::Other("a pipe"));
        assert_eq!(
            paths(&c.finish()).1,
            [
                ("/out/out", "a symbolic link to /etc/passwd, not followed"),
                ("/out/in", "a symbolic link to a, not followed"),
                ("/out/h", "a hard link to out/a, not followed"),
                ("/out/null", "a device, not a regular file"),
                ("/out/fifo", "a pipe, not a regular file"),
            ]
        );
    }

    #[test]
    fn a_name_outside_the_folder_is_skipped() {
        let mut c = collector("/data/out", 10, 100);
        for name in ["/etc/passwd", "out/../../etc/passwd", "other/a", "outer/a", ""] {
            assert!(file(&mut c, name, 1, b'x'), "{name}");
        }
        assert!(file(&mut c, "./out/./a", 1, b'a'));
        let out = c.finish();
        let (files, skipped) = paths(&out);
        assert_eq!(files, ["/data/out/a"]);
        assert_eq!(
            skipped.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
            ["/etc/passwd", "out/../../etc/passwd", "other/a", "outer/a", ""]
        );
        assert!(skipped.iter().all(|(_, reason)| *reason == "outside the folder"));
    }

    #[test]
    fn one_file_by_its_path_and_an_empty_file() {
        let mut c = collector("/work/./progress.json", 1, 1);
        assert!(file(&mut c, "progress.json", 0, b'x'));
        let out = c.finish();
        assert_eq!(out.files, [CopyFile::new("/work/progress.json", "")]);
    }

    #[test]
    fn a_failed_read_keeps_nothing_and_is_returned() {
        let mut c = collector("/out", 10, 100);
        let result = c.offer("out/a", EntryKind::File, 5, 0o644, |_: &mut [u8]| Err("cut"));
        assert_eq!(result, Err("cut"));
        assert_eq!(c.finish(), CopyOut::default());
    }
}
