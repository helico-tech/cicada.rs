//! [`check_output_store`]: the conformance check of an [`OutputStore`] (feature `testing`).

use std::fmt::Debug;

use super::{LogEnd, OutputError, OutputErrorKind, OutputStore, StoredFile};
use crate::{CopyFile, LogCursor, LogLine, LogPage, LogStream};

/// The container names the check uses, and removes before and after it runs.
const A: &str = "cicada-output-check-a";
const B: &str = "cicada-output-check-b";

/// Runs the rules of [`OutputStore`] against `store` and panics with the rule it broke, so a host checks its own store
/// the way Cicada checks its store backends. It uses container names of its own (`cicada-output-check-a` and `-b`),
/// removes them before and after, and so runs on a store that holds other data.
///
/// It checks the input rules (`Invalid` for a bad name or path, and one file for `/a`, `//a` and `/./a`), `log_end`
/// before and after appends, appends read back across pages, the guard of `append_logs` (a wrong or repeated `after`
/// fails with `Conflict` and stores nothing), an empty page that moves the cursor, `read_logs` past the end, files
/// replaced, listed in path order, read and missing, `remove` of everything and of nothing, and two containers apart.
///
/// It cannot prove that a write is all or nothing after a crash, nor that the guard holds under writers in parallel:
/// the host's store promises that. Nor does it check a file at `/a` next to one at `/a/b`: `copy_out` gives only
/// regular files, which a container cannot hold at both paths.
pub async fn check_output_store(store: &dyn OutputStore) {
    ok(store.remove(A).await, "remove of the check's container");
    ok(store.remove(B).await, "remove of the check's container");
    input_rules(store).await;
    logs(store).await;
    files(store).await;
    apart_and_removed(store).await;
    ok(store.remove(A).await, "remove of the check's container");
    ok(store.remove(B).await, "remove of the check's container");
}

/// Every method refuses a bad name, and a path method a bad path, with `Invalid` before it touches anything.
async fn input_rules(store: &dyn OutputStore) {
    let page = LogPage::new(vec![line(0, "x")], cursor("1:1"));
    let file = CopyFile::new("/out/a.txt", "x");
    for name in ["../x", "a/b", "", ".."] {
        let rule = format!("a bad container name {name:?} is Invalid");
        invalid(store.log_end(name).await, &rule);
        invalid(store.append_logs(name, None, &page).await, &rule);
        invalid(store.read_logs(name, 0, 10).await, &rule);
        invalid(store.put_file(name, &file).await, &rule);
        invalid(store.files(name).await, &rule);
        invalid(store.read_file(name, "/out/a.txt").await, &rule);
        invalid(store.remove(name).await, &rule);
    }
    for path in ["out/a.txt", "../x", "/out/../../etc/passwd", "/", ""] {
        let rule = format!("a bad path {path:?} is Invalid");
        invalid(store.put_file(A, &CopyFile::new(path, "x")).await, &rule);
        invalid(store.read_file(A, path).await, &rule);
    }
    equal(ok(store.files(A).await, "files"), Vec::new(), "a refused put_file stores nothing");
    equal(ok(store.log_end(A).await, "log_end"), LogEnd::default(), "a refused call stores nothing");
}

/// The guarded append, `log_end` and `read_logs`.
async fn logs(store: &dyn OutputStore) {
    equal(ok(store.log_end(A).await, "log_end"), LogEnd::default(), "log_end of nothing is no cursor and 0 lines");
    equal(ok(store.read_logs(A, 0, 10).await, "read_logs"), Vec::new(), "read_logs of nothing is empty");

    let first = LogPage::new(vec![line(1, "l0"), line(2, "l1"), line(3, "l2")], cursor("3:1"));
    equal(ok(store.append_logs(A, None, &first).await, "append_logs"), 3, "append_logs gives the stored lines");
    equal(ok(store.log_end(A).await, "log_end"), LogEnd::new(Some(first.next), 3), "log_end after an append");

    let other = LogPage::new(vec![line(9, "bad")], cursor("9:1"));
    let stale = "an append with a wrong `after` fails with Conflict";
    conflict(store.append_logs(A, Some(&cursor("2:1")), &other).await, stale);
    conflict(store.append_logs(A, None, &other).await, stale);
    let repeat = "a repeat of the last append with its old `after` fails with Conflict";
    conflict(store.append_logs(A, None, &first).await, repeat);
    let unchanged = "a refused append stores nothing";
    equal(ok(store.log_end(A).await, "log_end"), LogEnd::new(Some(first.next), 3), unchanged);
    equal(texts(store, A, 0, 100).await, numbered(&["l0", "l1", "l2"]), unchanged);

    let second = LogPage::new(vec![line(4, "l3"), line(5, "l4")], cursor("5:1"));
    equal(ok(store.append_logs(A, Some(&first.next), &second).await, "append_logs"), 5, "a second append");
    let empty = LogPage::new(Vec::new(), cursor("6:0"));
    let moved = "an empty page stores its cursor alone";
    equal(ok(store.append_logs(A, Some(&second.next), &empty).await, "append_logs"), 5, moved);
    equal(ok(store.log_end(A).await, "log_end"), LogEnd::new(Some(empty.next), 5), moved);

    let all = numbered(&["l0", "l1", "l2", "l3", "l4"]);
    let mut paged = Vec::new();
    for from in [0, 2, 4] {
        paged.extend(texts(store, A, from, 2).await);
    }
    equal(paged, all.clone(), "read_logs pages by line number across appends");
    equal(texts(store, A, 0, 100).await, all, "read_logs gives up to `limit` lines in stored order");
    equal(texts(store, A, 1, 0).await, Vec::new(), "read_logs with limit 0 gives none");
    equal(texts(store, A, 5, 10).await, Vec::new(), "read_logs past the end gives none");
    equal(texts(store, A, u64::MAX, 10).await, Vec::new(), "read_logs past the end gives none");
    let stored = ok(store.read_logs(A, 3, 1).await, "read_logs");
    equal(stored.into_iter().map(|s| s.line).collect(), vec![line(4, "l3")], "read_logs gives the stored line whole");
}

/// `put_file`, `files` and `read_file`.
async fn files(store: &dyn OutputStore) {
    let mut first = CopyFile::new("/out/a.txt", "one");
    first.mode = 0o644;
    ok(store.put_file(A, &first).await, "put_file");
    let mut second = CopyFile::new("//out/./a.txt", "second");
    second.mode = 0o600;
    ok(store.put_file(A, &second).await, "put_file");
    for path in ["/a", "//a", "/./a"] {
        ok(store.put_file(A, &CopyFile::new(path, path)).await, "put_file");
    }
    ok(store.put_file(A, &CopyFile::new("/out/b.bin", vec![0u8, 255])).await, "put_file");
    ok(store.put_file(A, &CopyFile::new("/out.log", "")).await, "put_file");
    let mut expected = vec![
        StoredFile::new("/out/a.txt", 6, 0o600),
        StoredFile::new("/a", 4, 0o644),
        StoredFile::new("/out/b.bin", 2, 0o644),
        StoredFile::new("/out.log", 0, 0o644),
    ];
    expected.sort_by(|a, b| a.path.cmp(&b.path));
    let rule = "files lists each cleaned path once, replaced by the last put, in path order";
    equal(ok(store.files(A).await, "files"), expected, rule);

    let read = ok(store.read_file(A, "/out/./a.txt").await, "read_file");
    let read = read.unwrap_or_else(|| panic!("check_output_store: read_file gives a stored file: got none"));
    let rule = "read_file gives the last file put at the path, under its cleaned path";
    equal((read.path.as_str(), read.contents.as_slice(), read.mode), ("/out/a.txt", b"second".as_slice(), 0o600), rule);
    let read = ok(store.read_file(A, "/a").await, "read_file").map(|f| f.contents);
    equal(read, Some(b"/./a".to_vec()), "`/a`, `//a` and `/./a` are one file");
    equal(ok(store.read_file(A, "/missing").await, "read_file"), None, "read_file of a missing file is none");
    equal(ok(store.read_file(A, "/out").await, "read_file"), None, "read_file of a folder is none");
}

/// Two containers keep their own outputs; `remove` deletes all of one, and nothing is `Ok`.
async fn apart_and_removed(store: &dyn OutputStore) {
    let lines: Vec<LogLine> = (0..25).map(|i| line(i, &format!("b{i}"))).collect();
    let page = LogPage::new(lines, cursor("24:1"));
    equal(ok(store.append_logs(B, None, &page).await, "append_logs"), 25, "a second container starts at nothing");
    ok(store.put_file(B, &CopyFile::new("/out/a.txt", "b")).await, "put_file");
    for (from, until) in [(0, 10), (10, 20), (20, 25)] {
        let expected: Vec<(u64, String)> = (from..until).map(|i| (i, format!("b{i}"))).collect();
        equal(texts(store, B, from, 10).await, expected, "read_logs pages 25 lines by 10");
    }
    equal(ok(store.log_end(A).await, "log_end").lines, 5, "two containers keep their own lines");
    let read = ok(store.read_file(A, "/out/a.txt").await, "read_file").map(|f| f.contents);
    equal(read, Some(b"second".to_vec()), "two containers keep their own files");

    ok(store.remove(A).await, "remove");
    let gone = "remove deletes a container's lines, cursor and files";
    equal(ok(store.log_end(A).await, "log_end"), LogEnd::default(), gone);
    equal(ok(store.read_logs(A, 0, 10).await, "read_logs"), Vec::new(), gone);
    equal(ok(store.files(A).await, "files"), Vec::new(), gone);
    equal(ok(store.read_file(A, "/out/a.txt").await, "read_file"), None, gone);
    ok(store.remove(A).await, "remove of nothing is Ok");
    let kept = "remove keeps the other containers";
    equal(ok(store.log_end(B).await, "log_end"), LogEnd::new(Some(page.next), 25), kept);
    equal(ok(store.files(B).await, "files").len(), 1, kept);

    let fresh = LogPage::new(vec![line(30, "again")], cursor("30:1"));
    let rule = "after remove, a container starts afresh from no cursor";
    equal(ok(store.append_logs(A, None, &fresh).await, "append_logs"), 1, rule);
}

/// The line `text` at Unix second `second` after 2025-11-24 16:00 UTC, on standard error for an odd second.
fn line(second: i64, text: &str) -> LogLine {
    let stream = if second % 2 == 0 { LogStream::Out } else { LogStream::Err };
    LogLine::new(cicada::time::from_micros(1_764_000_000_000_000 + second * 1_000_000), stream, text)
}

fn cursor(text: &str) -> LogCursor {
    text.parse().expect("a cursor")
}

/// `texts` numbered from 0.
fn numbered(texts: &[&str]) -> Vec<(u64, String)> {
    (0..).zip(texts.iter().map(|t| (*t).to_owned())).collect()
}

/// The numbers and texts of `read_logs(container, from, limit)`.
async fn texts(store: &dyn OutputStore, container: &str, from: u64, limit: u32) -> Vec<(u64, String)> {
    let lines = ok(store.read_logs(container, from, limit).await, "read_logs");
    lines.into_iter().map(|stored| (stored.number, stored.line.text)).collect()
}

fn ok<T>(result: Result<T, OutputError>, rule: &str) -> T {
    result.unwrap_or_else(|error| panic!("check_output_store: {rule}: the call failed: {error}"))
}

fn invalid<T: Debug>(result: Result<T, OutputError>, rule: &str) {
    fails_with(result, OutputErrorKind::Invalid, rule);
}

fn conflict<T: Debug>(result: Result<T, OutputError>, rule: &str) {
    fails_with(result, OutputErrorKind::Conflict, rule);
}

fn fails_with<T: Debug>(result: Result<T, OutputError>, kind: OutputErrorKind, rule: &str) {
    match result {
        Err(error) if error.kind == kind => {}
        other => panic!("check_output_store: {rule}: got {other:?}"),
    }
}

fn equal<T: Debug + PartialEq>(got: T, expected: T, rule: &str) {
    assert!(got == expected, "check_output_store: {rule}: got {got:?}, expected {expected:?}");
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::{MemoryOutputStore, StoredLine};

    #[tokio::test]
    async fn the_memory_store_passes() {
        let store = MemoryOutputStore::new();
        ok(store.put_file("other", &CopyFile::new("/kept", "x")).await, "put_file");
        check_output_store(&store).await;
        check_output_store(&store).await;
        equal(ok(store.files("other").await, "files").len(), 1, "the check leaves other containers alone");
        equal(ok(store.files(A).await, "files"), Vec::new(), "the check removes its own containers");
    }

    /// How [`Broken`] breaks a rule.
    #[derive(Clone, Copy)]
    enum Fault {
        /// `append_logs` ignores `after`.
        NoGuard,
        /// The name rule is not checked by `files`.
        AcceptsBadName,
        /// `put_file` keeps the path as given.
        RawPaths,
        /// `remove` keeps the files.
        KeepsFiles,
    }

    /// A memory store with one fault.
    struct Broken(MemoryOutputStore, Fault);

    #[async_trait]
    impl OutputStore for Broken {
        async fn log_end(&self, container: &str) -> Result<LogEnd, OutputError> {
            self.0.log_end(container).await
        }

        async fn append_logs(
            &self,
            container: &str,
            after: Option<&LogCursor>,
            page: &LogPage,
        ) -> Result<u64, OutputError> {
            let after = match self.1 {
                Fault::NoGuard => self.0.log_end(container).await?.cursor,
                _ => after.copied(),
            };
            self.0.append_logs(container, after.as_ref(), page).await
        }

        async fn read_logs(&self, container: &str, from: u64, limit: u32) -> Result<Vec<StoredLine>, OutputError> {
            self.0.read_logs(container, from, limit).await
        }

        async fn put_file(&self, container: &str, file: &CopyFile) -> Result<(), OutputError> {
            if let Fault::RawPaths = self.1 {
                crate::clean_output_path(&file.path)?;
                let path = format!("/raw{}", file.path.replace('/', "|"));
                return self.0.put_file(container, &CopyFile::new(path, file.contents.clone())).await;
            }
            self.0.put_file(container, file).await
        }

        async fn files(&self, container: &str) -> Result<Vec<StoredFile>, OutputError> {
            if let Fault::AcceptsBadName = self.1
                && container.contains('/')
            {
                return Ok(Vec::new());
            }
            self.0.files(container).await
        }

        async fn read_file(&self, container: &str, path: &str) -> Result<Option<CopyFile>, OutputError> {
            self.0.read_file(container, path).await
        }

        async fn remove(&self, container: &str) -> Result<(), OutputError> {
            if let Fault::KeepsFiles = self.1 {
                let files = self.0.files(container).await?;
                let mut kept = Vec::new();
                for file in files {
                    kept.extend(self.0.read_file(container, &file.path).await?);
                }
                self.0.remove(container).await?;
                for file in kept {
                    self.0.put_file(container, &file).await?;
                }
                return Ok(());
            }
            self.0.remove(container).await
        }
    }

    async fn broken(fault: Fault) {
        check_output_store(&Broken(MemoryOutputStore::new(), fault)).await;
    }

    #[tokio::test]
    #[should_panic(expected = "check_output_store: an append with a wrong `after` fails with Conflict")]
    async fn a_store_without_the_guard_fails() {
        broken(Fault::NoGuard).await;
    }

    #[tokio::test]
    #[should_panic(expected = "check_output_store: a bad container name \"../x\" is Invalid")]
    async fn a_store_that_takes_a_bad_name_fails() {
        broken(Fault::AcceptsBadName).await;
    }

    #[tokio::test]
    #[should_panic(expected = "check_output_store: files lists each cleaned path once")]
    async fn a_store_that_keeps_raw_paths_fails() {
        broken(Fault::RawPaths).await;
    }

    #[tokio::test]
    #[should_panic(expected = "check_output_store: remove deletes a container's lines, cursor and files")]
    async fn a_store_whose_remove_keeps_files_fails() {
        broken(Fault::KeepsFiles).await;
    }
}
