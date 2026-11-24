# Design

A host collects a run's artifacts after it ended and reads a small progress file while it runs. The shape is agreed
with Factory, the consumer:

```rust
async fn copy_out(&self, name: &str, path: &str, limits: CopyLimits) -> Result<CopyOut, ContainerError>;
pub struct CopyLimits { pub max_files: u32, pub max_bytes: u64 }       // CopyLimits::new(max_files, max_bytes)
pub struct CopyOut { pub files: Vec<CopyFile>, pub skipped: Vec<SkippedFile> }
pub struct SkippedFile { pub path: String, pub reason: String }
```

`path` names a folder (every regular file below it) or one file. Each `CopyFile` has its absolute path in the
container, its contents and its permission bits, so a host can hand it to `copy_in` of another container. All three
structs are `non_exhaustive`. It works on a created, running or ended container until `remove`.

## Decisions

1. **Limits are a file count and a byte budget; no per-file limit.** Entries come in the archive's order. A regular file
   goes into `files` when its size fits what is left of `max_bytes`; else it is skipped ("N bytes, more than the M
   left") and the copy goes on, so one huge file costs only itself (Factory's issue 0006). `max_files` bounds the
   entries of `files` and `skipped` together. Once it is reached, the copy stops at the next countable entry (one that
   would go into either list) and adds one last `SkippedFile` for `path` ("more than N files; the rest was not read");
   with exactly `max_files` entries in the folder there is no such entry (design review, blocking 1). A limit of zero,
   files or bytes, is `InvalidSpec` before any call. So memory is at most `max_bytes`, plus one entry's headers, plus
   `max_files + 1` paths, whatever the folder holds. A file is whole or skipped: its size is in its header, checked
   before any byte is kept, and it is read into a buffer of exactly that size. Not taken: a per-file limit (the budget
   already lets one big file skip and the rest land; `CopyLimits` is `non_exhaustive` with a constructor, so one can be
   added later); listing every skipped file (unbounded); an upper bound of our own (the caller's limits are the bound).
2. **`SkippedFile`, not `Skipped`.** `Skipped { resource, name, reason }` is already `Swept`'s; a file has no
   resource kind. Same idea, its own type, a string reason as in `Swept`. Not taken: `Resource::File` in `Skipped`
   (a field that is always the same here, and `name` for a path).
3. **Only regular files are copied; links are never followed.** A symbolic link, inside the folder or out of it, is
   skipped ("a symbolic link to <target>, not followed"); so is a hard link, a device, a pipe or a socket. A folder
   is not listed: the paths of its files show it. An entry whose name is absolute, has a `..` part or is not under
   `path` is skipped ("outside the folder"). Not taken: following links inside the folder (needs resolving targets
   against the folder, and a link can be swapped between check and read); following all links (reads outside the
   folder).
4. **Paths.** `path` passes the shared path rule's parsing from `container-consistency`: absolute, no `..`, names a
   file once empty and `.` parts are dropped; else `InvalidSpec`. So `/` is refused. The mount checks of `copy_in`
   do not apply: reading a mount, such as a volume at `/out`, is the main use. Files come back with cleaned paths.
5. **What a container gives.** A missing container fails with `Conflict`, as `logs` and `copy_in` do. A missing path
   gives an empty `CopyOut`. A file being written while it is read can come back half written: the rustdoc says to write
   a progress file to a temporary name and rename it. Files written to a tmpfs are gone once the container ended (Docker
   then shows the image's files under that path, if any): artifacts belong on the container's own file system or on a
   volume (Factory's review).
6. **Docker streams the archive.** The backend finds the container (inspect, as every call), then asks Docker for the
   archive of `path` (`download_from_container`) and reads it as a stream: bollard's byte stream becomes a reader
   (`tokio_util::io::StreamReader`), and the `tar` crate reads it in `spawn_blocking` through
   `tokio_util::io::SyncIoBridge`. The `tar` crate reads long names (PAX and GNU) and skips the data of an entry that is
   not read, in 32 KiB reads, so a skipped file is never held. A kept file is read with `read_exact` into a buffer of
   its size. bollard's errors are no `io::Error`, and `tar` wraps the errors it passes on, so the reader's `map_err`
   keeps the first bollard error in a side slot (`Arc<Mutex<Option<…>>>`) and hands `tar` a plain `io::Error`; when
   `tar` fails, the slot decides the kind, through `kind_of`. A 404 in the slot means a missing path, unless a second
   inspect finds the container gone (removed during the call): then `Conflict`. Any other error in the middle of the
   stream fails the whole call with its kind (`DaemonUnavailable` for a lost daemon); no half result. Not taken here: a
   custom payload in the `io::Error`, downcast after `tar` (it sits under `tar`'s own wrapper). `tar` is already a
   dependency of the feature `docker`; it adds `tokio-util` (already in the workspace and `Cargo.lock`) with its `io`
   and `io-util` features. Not taken: buffering the whole archive (unbounded); `tokio-tar` or `async-tar` (a new crate);
   parsing tar blocks by hand (PAX and GNU names again).
7. **One collector for both runtimes.** A private collector takes entries in order (path, kind, size, a way to read
   the contents) and applies decisions 1 and 3; the fake and Docker both feed it, so they keep and skip the same
   files. The fake orders its files as Docker's archive walks a folder: by path, part by part (`/out/a/c` before
   `/out/a-b`). Not taken: limits in each backend (they would drift, as `copy_in` did).
8. **The fake holds files.** It already keeps what `copy_in` wrote, under cleaned paths. `put_file(name, CopyFile)`
   adds a file the program "wrote", in any state (it panics for a missing container: the test is wrong). On an ended
   container, `copy_out` leaves out files whose deepest mount is a tmpfs (the shared path rule's mount choice), as on
   Docker. `Call::CopyOut` and `Method::CopyOut` record and script it. The fake has no links.
9. **The trait gets a default method**, as `container-logs` decided (its decision 5): `copy_out` fails with `Other`
   ("not supported by this runtime") unless a runtime overrides it, so no implementation outside this repo breaks at
   a patch. The proposal's Impact line is corrected to say so.

## Limits

The rustdoc of `copy_out` names each one; the handoff carries them to the guide.

- Dropping the future does not stop the blocking reader: it reads the archive to its end and then drops it.
- A file that grows, shrinks or vanishes while Docker builds the archive can come back short, or be missing,
  without an error: moby logs some walk errors and goes on (not verified).
- The order of the entries, and so which files fit the limits first, may differ between Docker engine versions.
- A file being written while it is read can come back half written (decision 5).

## Spec deltas

`container-runtime`: *Files can be copied out within limits*, *A copy out reads only regular files* and *The fake
holds files to copy out* are added; the default method joins a new requirement next to `container-logs`' one.
`container-docker`: *Docker's archive is read as a stream* is added. No guarantee, wire format, schema or actor
contract changes, so no ADR.

## Tests

Unit tests of the collector (budget skip and go on, the file count stop with its last entry and exactly at the limit
without it, zero limits, links and other kinds, names outside the folder, whole or skipped), of the fake (`copy_in` then
`copy_out`, `put_file`, the walk order, a tmpfs file gone after the end, a missing path, `Conflict`, `InvalidSpec`,
scripted failures) and of the archive reader over an in-memory tar (a long PAX name, a skipped big entry never read into
memory, a stream error mid-file whose kind comes back through the side slot, a 404 first chunk).
The E2E class `e2e/tests/suite/container_docker.rs` adds:

- an exited container that wrote `/out/a`, `/out/sub/b` (mode 0o755), a link out (`/out/out -> /etc/passwd`) and a
  link in (`/out/in -> a`): `copy_out("/out")` gives `a` and `sub/b` with contents and modes, and skips both links;
- limits: files of 10, 20 and 5000 bytes with `max_bytes` 100 keep the small ones and skip the big one; five files
  with `max_files` 2 give two entries and the "rest was not read" entry, two files give two entries and no more; a
  zero limit is `InvalidSpec`;
- one file by its path; a file in a volume after the end; a file written to a tmpfs is gone after the end; a progress
  file read while running; a file from `copy_in` read back from a created container;
- a missing path is empty, a missing container is `Conflict`, a relative path is `InvalidSpec`.

## Docs it breaks

None: no README or guide block, link, guarantee row or skill route names the container runtime. `docs-handoff.md`
carries the call, the limits, the link rule, the tmpfs note and the rename advice for the container guide.

## Version

0.20.9, with the rest of the container line; released when the line, through `container-limits-and-kinds`, has
landed.
