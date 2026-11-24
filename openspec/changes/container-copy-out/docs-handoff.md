# Docs handoff for container-copy-out

Version: 0.20.9. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` can copy files out of a container within limits, so a host collects a run's artifacts after it
ended and reads a small progress file while it runs. Additive; no README or guide block, link, guarantee row or skill
route names the container runtime.

- `ContainerRuntime::copy_out(name, path, limits: CopyLimits) -> Result<CopyOut, ContainerError>`.
  `CopyLimits { max_files: u32, max_bytes: u64 }` with `CopyLimits::new(max_files, max_bytes)`;
  `CopyOut { files: Vec<CopyFile>, skipped: Vec<SkippedFile> }` with `CopyOut::new` and `Default`;
  `SkippedFile { path: String, reason: String }` with `SkippedFile::new`. All three are `non_exhaustive` and exported
  from the crate root.
- The method has a default that fails with `Other`: a host's own runtime compiles unchanged.
- `FakeRuntime::put_file(name, CopyFile)` adds a file the program "wrote", in any state; it panics for a missing
  container or a bad path. `Call::CopyOut { name, path, limits }` and `Method::CopyOut` record and script the call.

Proved by the unit tests in `crates/cicada-containers/src/copy.rs` (the collector: the byte budget, the file limit
with and without its last entry, zero limits, links and other kinds, names outside the folder, a file never cut),
`src/fake.rs` (`copy_out_returns_what_copy_in_wrote_and_what_the_program_wrote`, `copy_out_walks_a_folder_part_by_part`,
`copy_out_skips_a_file_over_the_bytes_left_and_stops_at_the_file_limit`, `copy_out_of_what_is_not_there`,
`a_tmpfs_is_empty_once_the_container_ended`, `a_scripted_failure_of_copy_out_is_recorded`,
`a_runtime_without_logs_since_or_copy_out_compiles_and_fails_with_other`) and `src/docker/archive.rs` (long GNU and PAX
names, a 64 MiB skipped entry streamed past, a stream error mid-file as `DaemonUnavailable`, a 404 first chunk), and by
`ContainerDockerTest` in `e2e/tests/suite/container_docker.rs` (`copy_out_collects_regular_files_and_skips_links`,
`copy_out_keeps_within_its_limits`, `copy_out_after_the_end_reads_a_volume_and_not_a_tmpfs`,
`copy_out_reads_a_progress_file_whole_while_it_is_renamed`,
`copy_out_reads_copy_in_back_and_refuses_what_is_not_there`). The rustdoc of `copy_out`, the three types and
`DockerRuntime` says the same.

## 1. Collecting a run's files

**Where.** The container guide, when the docs session writes it (no guide covers `cicada-containers` yet): the
runtime section, after `copy_in`.
**Text.** `copy_out(name, path, limits)` returns the regular files at or below `path`: every file below a folder, or
the one file `path` names. It works on a created, running or ended container, until `remove`; a host collects
artifacts after `ContainerEnded` and before it sends `Remove`. Each kept file is whole, with its absolute path, its
contents and its permission bits, so it can go straight to `copy_in` of another container. Call it outside actor
turns: the files are too big for actor state.

The limits keep memory bounded whatever the folder holds: at most `max_bytes` of contents plus `max_files + 1` paths.
Entries come in the archive's order. A file goes into `files` when its size fits what is left of `max_bytes`;
otherwise it goes into `skipped` with its size ("5000 bytes, more than the 90 left") and the copy goes on, so one huge
file costs only itself. A file is whole or skipped, never cut. `max_files` bounds `files` and `skipped` together: once
it is reached, the next entry that would go into either stops the copy, and one last `SkippedFile` for `path` says
"more than N files; the rest was not read". A folder with exactly `max_files` entries gives no such entry.

```rust,ignore
let out = runtime.copy_out(&name, "/out", CopyLimits::new(100, 10 << 20)).await?;
for file in &out.files { store_artifact(run, &file.path, &file.contents, file.mode)?; }
for skipped in &out.skipped { warn!("{} not collected: {}", skipped.path, skipped.reason); }
```

(Not compiled from the guide; the E2E steps above run the same calls. Make it a doctest of `examples/` if it goes in.)

Errors: a relative `path`, one with a `..` part, `/`, or a zero limit is `InvalidSpec` before any call; a missing
container is `Conflict`, also one removed during the call; a missing `path` gives an empty `CopyOut`, not an error.
On Docker, an error during the archive stream fails the whole call with its kind (`DaemonUnavailable` for a lost
daemon, the only one worth a retry): there is never a half result.
**Caveats.** Order and so which files fit the limits first may differ between Docker engine versions.

## 2. Links and other kinds are never followed

**Where.** The same section.
**Text.** Only regular files are copied. A symbolic link, inside the folder or out of it, is skipped ("a symbolic link
to /etc/passwd, not followed"); so is a hard link, a device, a pipe or a socket, and an entry whose name is not under
`path` ("outside the folder"). A folder is not listed: the paths of its files show it. A program that wants a linked
file collected writes the file itself into the folder.

## 3. Where artifacts belong, and progress files

**Where.** The same section, and the container design advice where a run's output is planned.
**Text.** Files written to a tmpfs are gone once the container ended: Docker then shows the image's files under that
path, if any. Artifacts to collect after the end belong on the container's own file system or on a volume. A file
being written while it is read can come back half written: a program that updates a progress file writes it to a
temporary name in the same folder and renames it into place (`mv /work/.progress.tmp /work/progress.json`); the host
then reads `/work/progress.json` with small limits, such as `CopyLimits::new(1, 64 << 10)`, and always gets a whole
file.
**Caveats.** Dropping the `copy_out` future does not stop the read: Docker's archive is read on a blocking thread to its
end and then dropped. A file that grows, shrinks or vanishes while Docker builds the archive can come back short, or
be missing, without an error.

## 4. Files in the fake

**Where.** The container guide, the section on testing with `FakeRuntime`.
**Text.** `put_file(name, CopyFile)` adds a file as if the container's program wrote it, in any state; `copy_out`
returns it with what `copy_in` wrote, in the order Docker's archive walks a folder (part by part: `/out/a/c` before
`/out/a-b`), under the same limits and skip rules as Docker. On an ended container, files whose deepest mount is a
tmpfs are left out, as on Docker. `fail_next(Method::CopyOut, kind)` scripts a failure; `calls()` records
`Call::CopyOut`.
**Caveats.** The fake has no links: it holds only regular files. `put_file` replaces a file at the same cleaned path.

## Blocking

None.

## Status

Ran `cargo test -p cicada-containers --all-features` (lib and integration tests), `cargo clippy` on the crate and the
E2E crate, the three single-feature builds of `cicada-containers` with warnings denied, and the five new
`ContainerDockerTest` steps against Docker; the finishing checks and the gate are in the build report. Every section
above is proposed text; no protected prose is written yet.
