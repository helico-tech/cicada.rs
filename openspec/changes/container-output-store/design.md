# Design

The first change of the container-outputs line (owner: CO-SPLIT). A host keeps a container's log and output files
outside the container, in a store it provides (Factory: a folder per run), and reads them back. The later changes
write through this interface: the log archive, progress, the collection at the end, retention. ADR 0056 keeps a layer
with a single consumer at that consumer; the owner decided on 2026-11-25 (CO-SPLIT) to move these general parts out of
Factory into `cicada-containers` all the same, as any host that runs containers needs them (design review, blocking 2).

```rust
#[async_trait]
pub trait OutputStore: Send + Sync + 'static {
    async fn log_end(&self, container: &str) -> Result<LogEnd, OutputError>;   // cursor and number of lines
    async fn append_logs(&self, container: &str, after: Option<&LogCursor>, page: &LogPage) -> Result<u64, OutputError>;
    async fn read_logs(&self, container: &str, from: u64, limit: u32) -> Result<Vec<StoredLine>, OutputError>;
    async fn put_file(&self, container: &str, file: &CopyFile) -> Result<(), OutputError>;
    async fn files(&self, container: &str) -> Result<Vec<StoredFile>, OutputError>;
    async fn read_file(&self, container: &str, path: &str) -> Result<Option<CopyFile>, OutputError>;
    async fn remove(&self, container: &str) -> Result<(), OutputError>;
}
```

## Decisions

1. **An async trait per container name, like `ContainerRuntime`.** Used as `Arc<dyn OutputStore>`. The container name
   is the key (the actor derives it from its address), so one store serves every job. Not taken: a store per
   container (a host would build one per run); sync methods (a store on a network or a database would block the
   runtime).
2. **Lines and cursor in one write, guarded by the cursor before.** `append_logs(container, after, page)` stores the
   page's lines and its `next` cursor together, or nothing, and only when the stored cursor equals `after` (`None`
   for a container with nothing stored); otherwise it fails with `Conflict` and stores nothing. It returns the number
   of stored lines after the write. So an archiver that crashed after a write reads the cursor again and goes on
   without a repeated or lost line, and two archivers of one container cannot interleave. An empty page with a
   different `next` stores the cursor alone. Not taken: separate calls for lines and cursor (a crash between them
   repeats or loses lines); no guard (two writers would double lines).
   `log_end(container)` gives the stored cursor (`None` when nothing is stored) and the number of stored lines, so a
   log view can show the last lines and an archiver resumes.
3. **Read back by line number.** `read_logs(container, from, limit)` gives up to `limit` lines from line number `from`
   (0-based, in stored order), each a `StoredLine` with its number and its `LogLine`; past the end it gives none. A
   line number means the same in every store, and a host's HTTP route pages with it. Not taken: byte offsets (they
   depend on the store's encoding).
4. **Files as the runtime gives them.** `put_file` stores a `CopyFile` (absolute path in the container, contents,
   mode) under the container, replacing one at the same path; `files` lists `StoredFile { path, size, mode }` in path
   order; `read_file` gives the file or `None`. `remove(container)` deletes the logs, the cursor and the files;
   removing nothing is `Ok`. Limits stay with `copy_out`: the store takes what it is given.
5. **Its own error: `OutputError { kind, message }`** with `OutputErrorKind::{Invalid, Conflict, Unavailable, Other}`
   and `is_retryable()` true only for `Unavailable`, mirroring `ContainerError` without its runtime kinds. Not taken:
   `ContainerError` (its kinds name a daemon and images).
6. **Input rules, as the runtime's.** A container name and a file path can reach a store from a user (a host's HTTP
   route), and a folder store turns them into file paths. A name must follow the container actor's rule (a letter or
   digit first, then only `A-Za-z0-9_.-`, so no `/` and no `..`), and a path the rule of `copy_out` (absolute, no `..`
   part, not `/`; empty and `.` parts dropped, so `/a`, `//a` and `/./a` are one file). The crate exports
   `check_container_name` and `clean_output_path`, which give the cleaned value or `Invalid`; every store applies
   them before it touches anything, and the conformance check proves it refuses `../x`, `a/b` as a name, and a
   relative path with `Invalid` (design review, blocking 1).
7. **`LogLine` and `LogStream` get serde**, so a store that writes to disk has a format: `{"time", "stream", "text"}`
   with `stream` as `out` or `err`. Additive.
8. **An in-memory store and a conformance check, behind `testing`.** `MemoryOutputStore` for tests, and
   `check_output_store(store)`, an async function that runs the rules above against any implementation and panics
   with the rule it broke, so a host checks its own store (Factory's folder store) the way Cicada checks its backends.
   It uses container names of its own, so it runs on a store that holds other data. It cannot prove that a write is
   all or nothing after a crash, or safe under parallel writers: the host's store promises that, and the rustdoc
   says so. A file at `/a` next to one at `/a/b` is not checked: `copy_out` gives only regular files, which a
   container cannot hold at both paths. Not taken: a folder store in this crate (a host's own layout and retention;
   Factory keeps its own). A later change can add one.
9. **After `remove`, a container starts afresh.** Its cursor is `None` again, so a writer that still runs would archive
   the log again from the start. The log archive and retention changes keep an archiver from running after a
   remove; their proposals say so.

## Tests

`check_output_store` passes on `MemoryOutputStore`: the input rules (`Invalid` for a bad name or path, one file for
`/a`, `//a` and `/./a`); `log_end` before and after appends; append and read back across pages; a guarded append with a
wrong `after` stores nothing and fails with `Conflict`; a repeat of the last append with its old `after` fails, so nothing
is doubled; an empty page moves the cursor; `read_logs` past the end; files replaced, listed in order, read, missing;
`remove` of everything and of nothing; two containers apart. A store that breaks a rule makes it panic (a deliberately
broken store in the test).

## Docs

Nothing breaks. The handoff gives the page the docs session writes for `cicada-containers` the trait, the one-write
rule, the input rules and the conformance check.
