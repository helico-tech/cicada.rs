# Docs handoff for container-consistency

Version: 0.20.9. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

Three fixes in `cicada-containers`, so the fake and the Docker backend give the same answer. No new public item, no
new method, field or error kind; no README or guide block, link, guarantee row or skill route names these calls.

- `ContainerRuntime::copy_in` follows one path rule on every runtime (a private module, `src/paths.rs`, that both
  call). A path fails with `InvalidSpec`, before any write, when it is relative, has a `..` part, names no file once
  empty and `.` parts are dropped (`/`), equals the target of a volume or bind mount (with any root), lies at or below
  a tmpfs with no deeper volume or bind mount (with any root), lies on a read-only mount, or, with `read_only_root`,
  lies on no volume or bind mount. `/work/./a` and `/work/a` are one file.
- `sweep`: `Swept` names only what this sweep removed or skipped. Something already gone, or a container another
  caller is removing, is in no list.
- `DockerRuntime::list` leaves out a container whose state Docker does not report; `inspect`, `start`, `copy_in` and
  `stop` of that name fail with `Other`.

Proved by the unit tests in `crates/cicada-containers/src/paths.rs`, `src/fake.rs` (`copy_in_refuses_a_mount_target_on_any_root`,
`copy_in_refuses_a_tmpfs_path_and_the_root_path_on_a_writable_root`, `copy_in_keeps_one_file_under_two_spellings`)
and `src/docker/map.rs` (`a_sweep_counts_404_as_gone_and_409_only_for_a_container`,
`list_leaves_out_a_container_without_a_state`), and by `ContainerDockerTest` in `e2e/tests/suite/container_docker.rs`
(the two `copy_in` steps). The rustdoc of `copy_in`, `sweep`, `Swept` and `DockerRuntime` says the same.

## 1. Where `copy_in` may write

**Where.** The container guide, when the docs session writes it (no guide covers `cicada-containers` yet): the
section on `copy_in`.
**Text.** Copy files only into a created container. A path must be absolute and name a file; `/work/./a` is
`/work/a`. Write below a mount target, never onto it: `/work` itself is refused when a volume or bind mount sits
there, as a file cannot replace a mount point. Never write at or below a tmpfs (unless a volume or bind mount sits
deeper): the tmpfs is mounted at start, over the file, so the call refuses it on any root. With `read_only_root`, a
file must lie below a writable volume or bind mount. Every refusal is `InvalidSpec` and writes none of the files.
**Caveats.** The fake and Docker refuse the same paths from 0.20.9; a host that wrote a mount target or a tmpfs path
on the fake before must move those files below a mount.

## 2. What a sweep reports

**Where.** The container guide, the section on `sweep`.
**Text.** `Swept` names what this sweep removed, per kind, and what it skipped with the reason. Something another
caller removed first, or a container another caller is still removing, is in no list, so two sweeps that race never
both name the same item.
**Caveats.** Do not count on `Swept` to list everything that carried the label; list or inspect when that matters.

## 3. What a list leaves out

**Where.** The container guide, the section on `list` (or the Docker backend's notes).
**Text.** `list` returns the containers that carry the label and whose status it can report. On Docker it leaves out
a container removed while it lists, and one whose state Docker does not report; `inspect` of such a name fails with
`Other`.
**Caveats.** A missing name in `list` is not proof the container is gone; `inspect` it by name.

## Blocking

None.

## Status

`cargo test -p cicada-containers --all-features` and `cargo test -p cicada-examples --test docs` pass; the finishing
checks and the gate are in the build report. All text above is proposed; no protected file is edited.
