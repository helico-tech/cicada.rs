//! [`MemoryOutputStore`]: an in-memory [`OutputStore`] for tests (feature `testing`).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;

use super::{
    LogEnd, OutputError, OutputErrorKind, OutputStore, StoredFile, StoredLine, check_container_name, clean_output_path,
};
use crate::{CopyFile, LogCursor, LogLine, LogPage};

/// An in-memory [`OutputStore`] that keeps every rule of the trait; clones share their outputs. Its guarded append
/// runs under one lock, so it holds under writers in parallel within the process.
///
/// ```
/// use cicada_containers::{MemoryOutputStore, OutputStore, check_output_store};
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), cicada_containers::OutputError> {
/// let store = MemoryOutputStore::new();
/// check_output_store(&store).await;
/// assert_eq!(store.log_end("job-1").await?.lines, 0);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default)]
pub struct MemoryOutputStore {
    containers: Arc<Mutex<BTreeMap<String, Outputs>>>,
}

/// What the store holds of one container.
#[derive(Debug, Default)]
struct Outputs {
    cursor: Option<LogCursor>,
    lines: Vec<LogLine>,
    /// By cleaned path, so they list in path order.
    files: BTreeMap<String, CopyFile>,
}

impl MemoryOutputStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Outputs>> {
        // A panic under the lock leaves whole values only: every change below is one assignment or push.
        self.containers.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl OutputStore for MemoryOutputStore {
    async fn log_end(&self, container: &str) -> Result<LogEnd, OutputError> {
        check_container_name(container)?;
        Ok(self.lock().get(container).map_or_else(LogEnd::default, |o| LogEnd::new(o.cursor, o.lines.len() as u64)))
    }

    async fn append_logs(
        &self,
        container: &str,
        after: Option<&LogCursor>,
        page: &LogPage,
    ) -> Result<u64, OutputError> {
        check_container_name(container)?;
        let mut containers = self.lock();
        let stored = containers.get(container).and_then(|o| o.cursor);
        if stored.as_ref() != after {
            let show = |cursor: Option<&LogCursor>| cursor.map_or_else(|| "none".to_owned(), ToString::to_string);
            return Err(OutputError::new(
                OutputErrorKind::Conflict,
                format!("logs of {container}: the stored cursor is {}, not {}", show(stored.as_ref()), show(after)),
            ));
        }
        let outputs = containers.entry(container.to_owned()).or_default();
        outputs.lines.extend(page.lines.iter().cloned());
        outputs.cursor = Some(page.next);
        Ok(outputs.lines.len() as u64)
    }

    async fn read_logs(&self, container: &str, from: u64, limit: u32) -> Result<Vec<StoredLine>, OutputError> {
        check_container_name(container)?;
        let containers = self.lock();
        let Some(outputs) = containers.get(container) else { return Ok(Vec::new()) };
        Ok((outputs.lines.iter().enumerate())
            .skip(usize::try_from(from).unwrap_or(usize::MAX))
            .take(limit as usize)
            .map(|(number, line)| StoredLine::new(number as u64, line.clone()))
            .collect())
    }

    async fn put_file(&self, container: &str, file: &CopyFile) -> Result<(), OutputError> {
        check_container_name(container)?;
        let path = clean_output_path(&file.path)?;
        let stored = CopyFile { path: path.clone(), contents: file.contents.clone(), mode: file.mode };
        self.lock().entry(container.to_owned()).or_default().files.insert(path, stored);
        Ok(())
    }

    async fn files(&self, container: &str) -> Result<Vec<StoredFile>, OutputError> {
        check_container_name(container)?;
        let containers = self.lock();
        let Some(outputs) = containers.get(container) else { return Ok(Vec::new()) };
        Ok(outputs.files.values().map(|f| StoredFile::new(&f.path, f.contents.len() as u64, f.mode)).collect())
    }

    async fn read_file(&self, container: &str, path: &str) -> Result<Option<CopyFile>, OutputError> {
        check_container_name(container)?;
        let path = clean_output_path(path)?;
        Ok(self.lock().get(container).and_then(|o| o.files.get(&path)).cloned())
    }

    async fn remove(&self, container: &str) -> Result<(), OutputError> {
        check_container_name(container)?;
        self.lock().remove(container);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogStream;

    fn cursor(text: &str) -> LogCursor {
        text.parse().unwrap()
    }

    fn page(texts: &[&str], next: &str) -> LogPage {
        let lines = texts.iter().map(|t| LogLine::new(cicada::time::from_micros(0), LogStream::Out, *t)).collect();
        LogPage::new(lines, cursor(next))
    }

    #[tokio::test]
    async fn a_guarded_append_stores_lines_and_cursor_together() {
        let store = MemoryOutputStore::new();
        assert_eq!(store.append_logs("c", None, &page(&["a", "b"], "1:2")).await.unwrap(), 2);
        let stale = store.append_logs("c", None, &page(&["a", "b"], "1:2")).await.unwrap_err();
        assert_eq!(stale.kind, OutputErrorKind::Conflict);
        assert_eq!(stale.message, "logs of c: the stored cursor is 1:2, not none");
        assert_eq!(store.append_logs("c", Some(&cursor("1:2")), &page(&["c"], "2:1")).await.unwrap(), 3);
        assert_eq!(store.log_end("c").await.unwrap(), LogEnd::new(Some(cursor("2:1")), 3));
        let texts: Vec<String> = store
            .read_logs("c", 1, 5)
            .await
            .unwrap()
            .into_iter()
            .map(|s| format!("{}{}", s.number, s.line.text))
            .collect();
        assert_eq!(texts, ["1b", "2c"]);
        assert!(store.read_logs("c", u64::MAX, 5).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn clones_share_and_remove_starts_afresh() {
        let store = MemoryOutputStore::new();
        let clone = store.clone();
        clone.put_file("c", &CopyFile::new("//out/./a", "x")).await.unwrap();
        assert_eq!(store.files("c").await.unwrap(), [StoredFile::new("/out/a", 1, 0o644)]);
        assert_eq!(store.read_file("c", "/out/a").await.unwrap().unwrap().path, "/out/a");
        store.remove("c").await.unwrap();
        assert!(clone.files("c").await.unwrap().is_empty());
        assert_eq!(clone.log_end("c").await.unwrap(), LogEnd::default());
    }

    #[tokio::test]
    async fn bad_input_touches_nothing() {
        let store = MemoryOutputStore::new();
        let error = store.put_file("c", &CopyFile::new("out/a", "x")).await.unwrap_err();
        assert_eq!(error.kind, OutputErrorKind::Invalid);
        assert_eq!(
            store.append_logs("../c", None, &page(&["a"], "1:1")).await.unwrap_err().kind,
            OutputErrorKind::Invalid
        );
        assert!(store.lock().is_empty());
    }
}
