//! A store for container outputs that the host provides: [`OutputStore`], keyed by container name, holds a container's
//! log lines with their log cursor and its output files, outside the container. With it, its types, its error
//! [`OutputError`] and the input rules every store applies, [`check_container_name`] and [`clean_output_path`]. Behind
//! the feature `testing`, the in-memory `MemoryOutputStore` and the conformance check `check_output_store`.

#[cfg(any(test, feature = "testing"))]
mod check;
#[cfg(any(test, feature = "testing"))]
mod memory;

use std::fmt;

use async_trait::async_trait;

#[cfg(any(test, feature = "testing"))]
pub use check::check_output_store;
#[cfg(any(test, feature = "testing"))]
pub use memory::MemoryOutputStore;

use crate::actor::MAX_NAME_LENGTH;
use crate::{CopyFile, LogCursor, LogLine, LogPage};

/// Where a host keeps a container's outputs, keyed by the container's name, so one store serves every job. Used as
/// `Arc<dyn OutputStore>`.
///
/// **One write for lines and cursor.** [`append_logs`](Self::append_logs) stores a page's lines and its cursor together
/// or not at all, and only when the stored cursor equals the one the writer read before. So a writer that crashed after
/// a write reads the cursor again with [`log_end`](Self::log_end) and goes on without a repeated or lost line, and two
/// writers of one container cannot interleave: the second fails with [`OutputErrorKind::Conflict`].
///
/// **Input rules.** A container name and a file path can come from a user, such as through a host's HTTP route. Every
/// method first checks the name with [`check_container_name`] and a path with [`clean_output_path`], and fails with
/// [`OutputErrorKind::Invalid`] before it touches anything. Paths that differ only in empty and `.` parts, such as
/// `/a`, `//a` and `/./a`, are one file.
///
/// **Case.** Paths differ by case: `/out/Job` and `/out/job` are two files, and so are the containers `Job` and `job`.
/// A store on a case-insensitive file system, such as the defaults of macOS and Windows, must not map them to one
/// file or folder: keep them apart, for example by encoding the names, or the second write replaces the first.
///
/// **What a store promises that no test proves.** That a write is all or nothing also after a crash of the process,
/// and that the guard of `append_logs` holds under writers in parallel, also on other nodes. `check_output_store`
/// (feature `testing`) checks every other rule.
///
/// After [`remove`](Self::remove), a container starts afresh: its cursor is `None` again.
#[async_trait]
pub trait OutputStore: Send + Sync + 'static {
    /// The stored cursor, `None` when nothing is stored, and the number of stored lines.
    async fn log_end(&self, container: &str) -> Result<LogEnd, OutputError>;

    /// Stores the lines of `page` and its `next` cursor in one write, when the stored cursor equals `after` (`None` for
    /// a container with nothing stored), and gives the number of stored lines after the write. Otherwise it fails with
    /// [`OutputErrorKind::Conflict`] and stores nothing. An empty page stores its cursor alone.
    async fn append_logs(&self, container: &str, after: Option<&LogCursor>, page: &LogPage)
    -> Result<u64, OutputError>;

    /// Up to `limit` stored lines from line number `from` (0-based, in stored order), each with its number; none past
    /// the end.
    async fn read_logs(&self, container: &str, from: u64, limit: u32) -> Result<Vec<StoredLine>, OutputError>;

    /// Stores `file` under the container at its cleaned path, replacing one at the same path. The store takes what it
    /// is given: the limits are those of [`copy_out`](crate::ContainerRuntime::copy_out).
    async fn put_file(&self, container: &str, file: &CopyFile) -> Result<(), OutputError>;

    /// The stored files of the container, in the byte order of their cleaned paths.
    async fn files(&self, container: &str) -> Result<Vec<StoredFile>, OutputError>;

    /// The stored file at `path`, with its cleaned path, or `None`.
    async fn read_file(&self, container: &str, path: &str) -> Result<Option<CopyFile>, OutputError>;

    /// Deletes the container's lines, cursor and files; `Ok` when nothing is stored.
    async fn remove(&self, container: &str) -> Result<(), OutputError>;
}

/// The end of a container's stored log, from [`OutputStore::log_end`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogEnd {
    /// The stored cursor, to read the runtime's log on from; `None` when nothing is stored.
    pub cursor: Option<LogCursor>,
    /// How many lines are stored.
    pub lines: u64,
}

impl LogEnd {
    /// The end at `cursor` after `lines` lines.
    pub fn new(cursor: Option<LogCursor>, lines: u64) -> Self {
        Self { cursor, lines }
    }
}

/// A stored log line and its number, from [`OutputStore::read_logs`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredLine {
    /// Its line number: 0-based, in stored order.
    pub number: u64,
    /// The line.
    pub line: LogLine,
}

impl StoredLine {
    /// The line `line` numbered `number`.
    pub fn new(number: u64, line: LogLine) -> Self {
        Self { number, line }
    }
}

/// A stored file without its contents, from [`OutputStore::files`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFile {
    /// Its cleaned absolute path in the container.
    pub path: String,
    /// Its length in bytes.
    pub size: u64,
    /// Its permission bits.
    pub mode: u32,
}

impl StoredFile {
    /// The file at `path` of `size` bytes with `mode`.
    pub fn new(path: impl Into<String>, size: u64, mode: u32) -> Self {
        Self { path: path.into(), size, mode }
    }
}

/// An error of an [`OutputStore`] call: its kind and a message from the store.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct OutputError {
    /// What went wrong, and with it whether a retry can help.
    pub kind: OutputErrorKind,
    /// The store's description.
    pub message: String,
}

impl OutputError {
    /// An error of `kind` with `message`.
    pub fn new(kind: OutputErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    /// True only for [`OutputErrorKind::Unavailable`].
    pub fn is_retryable(&self) -> bool {
        self.kind == OutputErrorKind::Unavailable
    }
}

/// The kind of an [`OutputError`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputErrorKind {
    /// A container name or a file path outside the input rules.
    Invalid,
    /// An [`append_logs`](OutputStore::append_logs) whose `after` is not the stored cursor.
    Conflict,
    /// The store could not be reached, or timed out; the only kind a retry is expected to cure.
    Unavailable,
    /// Anything else the store reports.
    Other,
}

impl fmt::Display for OutputErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid",
            Self::Conflict => "conflict",
            Self::Unavailable => "unavailable",
            Self::Other => "error",
        })
    }
}

/// The container name `name`, or [`OutputErrorKind::Invalid`] unless it follows the container actor's rule: a letter
/// or digit first, then only `A-Za-z0-9_.-`, and at most 200 characters. So a name holds no `/` and is never `..`.
///
/// ```
/// use cicada_containers::check_container_name;
///
/// assert_eq!(check_container_name("ci-job-1.a_b")?, "ci-job-1.a_b");
/// assert!(check_container_name("../x").is_err());
/// assert!(check_container_name("a/b").is_err());
/// assert!(check_container_name(&"a".repeat(201)).is_err());
/// # Ok::<(), cicada_containers::OutputError>(())
/// ```
pub fn check_container_name(name: &str) -> Result<&str, OutputError> {
    let mut chars = name.chars();
    let valid = name.len() <= MAX_NAME_LENGTH
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if valid {
        Ok(name)
    } else {
        Err(OutputError::new(
            OutputErrorKind::Invalid,
            format!(
                "{name:?} is not a container name: a letter or a digit first, then only A-Za-z0-9_.-, and at most \
                 {MAX_NAME_LENGTH} characters"
            ),
        ))
    }
}

/// The cleaned form of the file path `path`: absolute, with its empty and `.` parts dropped, such as `/a/b` for
/// `//a/./b/`. [`OutputErrorKind::Invalid`] for a relative path, a `..` part or no part at all, the rule of
/// [`copy_out`](crate::ContainerRuntime::copy_out), and for a control character (NUL included) in a part, which a
/// container's file name can hold but a store need not keep.
///
/// ```
/// use cicada_containers::clean_output_path;
///
/// assert_eq!(clean_output_path("//out/./a.txt")?, "/out/a.txt");
/// assert!(clean_output_path("/out/../../etc/passwd").is_err());
/// assert!(clean_output_path("out/a.txt").is_err());
/// assert!(clean_output_path("/").is_err());
/// assert!(clean_output_path("/out/a\0b").is_err());
/// # Ok::<(), cicada_containers::OutputError>(())
/// ```
pub fn clean_output_path(path: &str) -> Result<String, OutputError> {
    let invalid = |why: &str| OutputError::new(OutputErrorKind::Invalid, format!("{path:?} {why}"));
    if !path.starts_with('/') {
        return Err(invalid("is a relative path"));
    }
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
    if parts.contains(&"..") {
        return Err(invalid("has a `..` part"));
    }
    if parts.is_empty() {
        return Err(invalid("names no file"));
    }
    if path.chars().any(char::is_control) {
        return Err(invalid("has a control character"));
    }
    Ok(parts.iter().fold(String::with_capacity(path.len()), |mut clean, part| {
        clean.push('/');
        clean.push_str(part);
        clean
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogStream;

    #[test]
    fn a_name_follows_the_container_actor_rule() {
        for good in ["a", "0", "ci-job-1", "A.b_c-9"] {
            assert_eq!(check_container_name(good).unwrap(), good);
        }
        for bad in ["", "..", ".a", "-a", "_a", "a/b", "../x", "a b", "a:b", "é", "a\0"] {
            assert_eq!(check_container_name(bad).unwrap_err().kind, OutputErrorKind::Invalid, "{bad:?}");
        }
        assert!(check_container_name(&"a".repeat(200)).is_ok(), "200 characters");
        let long = check_container_name(&"a".repeat(201)).unwrap_err();
        assert_eq!(long.kind, OutputErrorKind::Invalid);
        assert!(long.message.ends_with("at most 200 characters"), "{long}");
    }

    #[test]
    fn a_path_is_cleaned_or_invalid() {
        for (path, clean) in
            [("/a", "/a"), ("//a", "/a"), ("/./a", "/a"), ("/out//x/./y/", "/out/x/y"), ("/a..b", "/a..b")]
        {
            assert_eq!(clean_output_path(path).unwrap(), clean, "{path:?}");
        }
        for bad in ["", "a", "./a", "../x", "/", "//", "/./", "/a/../b", "/out/../../etc/passwd", "/.."] {
            assert_eq!(clean_output_path(bad).unwrap_err().kind, OutputErrorKind::Invalid, "{bad:?}");
        }
        for bad in ["/out/a\0b", "/out\n/a", "/out/\u{1b}[31m", "/a\u{7f}", "/a\u{85}"] {
            let error = clean_output_path(bad).unwrap_err();
            assert_eq!(error.kind, OutputErrorKind::Invalid, "{bad:?}");
            assert!(error.message.ends_with("has a control character"), "{error}");
        }
    }

    #[test]
    fn only_an_unavailable_store_is_retryable() {
        assert!(OutputError::new(OutputErrorKind::Unavailable, "down").is_retryable());
        for kind in [OutputErrorKind::Invalid, OutputErrorKind::Conflict, OutputErrorKind::Other] {
            assert!(!OutputError::new(kind, "x").is_retryable(), "{kind:?}");
        }
        assert_eq!(OutputError::new(OutputErrorKind::Conflict, "stale").to_string(), "conflict: stale");
    }

    #[test]
    fn a_log_line_has_a_serde_form() {
        let line = LogLine::new(cicada::time::from_micros(1_764_000_000_123_456), LogStream::Err, "boom");
        let json = serde_json::to_value(&line).unwrap();
        assert_eq!(json, serde_json::json!({"time": "2025-11-24T16:00:00.123456Z", "stream": "err", "text": "boom"}));
        assert_eq!(serde_json::from_value::<LogLine>(json).unwrap(), line);
        assert_eq!(serde_json::to_value(LogStream::Out).unwrap(), "out");
        assert!(serde_json::from_value::<LogStream>("stdout".into()).is_err());
    }
}
