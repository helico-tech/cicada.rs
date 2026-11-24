# Proposal

## Why

Factory (cicada-code-factory) wants to move its container runs onto `cicada-containers` 0.20.7 and drop its own
`crates/containers` with its `docker` CLI backend (request from the Factory session, 2026-11-24; reference: Factory's
`docs/design/containers.md` and `crates/containers/src/runtime.rs`). It would be the third consumer (ADR 0056). Nothing can copy files out of a container: Factory collects artifacts after a run, and reads a small progress file
the program overwrites while it runs.

## What Changes

- `ContainerRuntime::copy_out(name, path, limits)` returns `CopyOut { files: Vec<CopyFile>, skipped: Vec<Skipped {
  path, reason }> }` for the files under `path` (or the one file), on a created, running or ended container; a
  missing path gives an empty result. A file is whole or skipped, never cut. Limits on the number of files and the
  bytes keep memory bounded whatever the folder holds; what does not fit is listed in `skipped` (as `Swept.skipped`),
  so one huge file does not cost a run all its artifacts (Factory's issue 0006). Symlinks out of the folder are not
  followed.
- The Docker backend reads Docker's archive and unpacks it in memory; the fake returns what it holds.

Factory's review (2026-11-24) adds: `copy_out` must work after the container has exited and before `Remove` (when
artifacts are collected), and its rustdoc says a tmpfs folder is empty once the container has ended: artifacts belong
on the container's own file system or a volume.

## Out of scope

Copying into the store. A separate `read_file`: one file with small limits is `copy_out`. The actor (the host copies
after `ContainerEnded` and before it sends `Remove`).

## Order

After `container-logs`, before `container-make-dirs`.

## Impact

The trait (an added method with a default that fails with `Other`, so no implementation outside this repo breaks;
design decision 9), the fake, the Docker backend and its E2E class. A patch.
