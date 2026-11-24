# Docs handoff for container-make-dirs

Version: 0.20.9. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` can make empty folders with a mode in a created container, so a program that does not run as
root can write there. Additive; no README or guide block, link, guarantee row or skill route names the container
runtime.

- `ContainerRuntime::make_dirs(name, dirs: &[CopyDir]) -> Result<(), ContainerError>`, next to `copy_in`.
- `CopyDir { path: String, mode: u32 }`, `non_exhaustive`, exported from the crate root; `CopyDir::new(path)` gives
  mode 0o755.
- The method has a default that fails with `Other`: a host's own runtime compiles unchanged.
- `FakeRuntime` keeps the folders it makes with their modes. `Call::MakeDirs { name, dirs }` and `Method::MakeDirs`
  record and script the call.

Proved by the unit tests in `crates/cicada-containers/src/dirs.rs` (the named-folder check, the walk: missing parts, a
file on the way, absolute and relative links, `..` stopping at the root, a link to no folder, eight links in the walk
and a ninth; named modes in either order, the path rule on the folders to make), `src/fake.rs`
(`make_dirs_makes_a_folder_and_its_parents_and_a_repeat_makes_nothing`,
`an_existing_folder_is_left_alone_and_gets_a_new_child`,
`a_file_at_the_path_or_a_parent_is_a_conflict_and_nothing_is_made`,
`a_named_parent_keeps_its_own_mode_in_either_order`, `one_path_named_twice`,
`a_bad_mode_and_a_late_or_missing_container`, `make_dirs_follows_the_path_rule_of_copy_in`,
`a_scripted_failure_of_make_dirs_is_recorded_and_makes_nothing`,
`a_runtime_without_logs_since_copy_out_or_make_dirs_compiles_and_fails_with_other`) and `src/docker/map.rs` (the
archive stat's folder and link bits, the folder tar parents first with modes and uid/gid 0, one tar per mount with a
read-only root), and by `ContainerDockerTest` in `e2e/tests/suite/container_docker.rs`
(`make_dirs_gives_user_1000_a_folder_and_leaves_existing_ones_alone`,
`make_dirs_refuses_a_file_and_a_path_below_a_file_and_a_late_call`,
`make_dirs_on_a_read_only_root_makes_only_on_a_mount`). The rustdoc of `make_dirs`, `CopyDir` and `DockerRuntime` says
the same.

## 1. Preparing writable folders before start

**Where.** The container guide, when the docs session writes it (no guide covers `cicada-containers` yet): the
runtime section, next to `copy_in`.
**Text.** A program that runs as a user other than root cannot write to a folder root made, or to one that does not
exist. Before `start`, `make_dirs(name, dirs)` makes the folders it needs: each missing named folder with its mode,
each missing parent with 0o755, owned by root. Mode 0o777 lets any user write.

```rust,ignore
let dirs = [CopyDir { mode: 0o777, ..CopyDir::new("/out") }, CopyDir::new("/tmp/out")];
runtime.make_dirs(&name, &dirs).await?;   // the container is Created
runtime.start(&name).await?;
```

(Not compiled from the guide; `make_dirs_gives_user_1000_a_folder_and_leaves_existing_ones_alone` runs the same calls
with a program of user 1000 that writes `/out/a`. `CopyDir` is `non_exhaustive`: outside the crate set the mode on a
`CopyDir::new(..)` binding, `let mut dir = CopyDir::new("/out"); dir.mode = 0o777;`. Make it a doctest of
`examples/` if it goes in.)

Calling it again makes nothing, as everything exists then: a replayed turn is safe.

## 2. What exists is never touched

**Where.** The same section.
**Text.** Only what is missing is made. An existing folder keeps its owner and mode, also when `dirs` names it with
another mode: `/tmp` keeps 1777, and an image's `/run/postgresql` keeps its owner `postgres` and mode 3777. So does
an existing parent. A folder named in `dirs` gets its own mode also when it is a parent of another named folder, in
any order. The same path named twice with one mode is made once; with two modes it is `InvalidSpec`.

A symbolic link at a named path or a parent counts as the folder it points to: in `postgres:18`, `/var/run` links to
`/run`, so `/var/run/app` makes `/run/app` and the link stays. The path rules below apply to the link's target. The
walk of one named folder follows at most 8 links, counting every link on its way. Docker resolves a chain of links
itself (`/tmp/l1 -> l2 -> /tmp` comes back as `/tmp`), so a chain counts as one link there, and a link loop fails with
`Other` from Docker rather than `Conflict`.

## 3. Errors and limits

**Where.** The same section.
**Text.** Every check is done before anything is made, so a refused call makes nothing.

- `Conflict`: a file at a named path or a parent (`/etc/passwd`, or `/etc/passwd/x` below it), a link to anything but
  a folder (a missing target too), more than 8 links in the walk, and a container that is missing, running or ended.
- `Other`: on Docker, a link loop at a named path or a parent (Docker's stat fails on it), and Docker's other
  refusals.
- `InvalidSpec`: a mode above 0o7777, one path with two modes, and every path `copy_in` refuses: relative, with a `..`
  part, `/`, a volume or bind mount target, at or below a tmpfs, on a read-only mount, and with `read_only_root` a
  path on no volume or bind mount.

**Caveats.**
- The mode of a volume's own root folder cannot be set, as a mount target is refused: make a folder inside the
  volume (`/work/cache`), or set the mode in the image.
- An existing folder keeps its mode and owner even when the caller asked for another mode: `make_dirs` never changes
  what exists.
- A folder another API caller makes between the check and the write is not seen; the container has no process yet,
  so nothing else can race.
- On Docker each named folder costs one archive stat per part of its path, and one upload per call (one per mount
  with a read-only root).

## Blocking

None.

## Status

Ran `cargo test -p cicada-containers --all-features`, the three new E2E steps
(`CICADA_E2E=1 CICADA_E2E_ONLY=ContainerDockerTest::make_dirs`) and `cargo test -p cicada-examples --test docs`; the
finishing checks and the gate are in the change's last task. All prose above is proposed text; none is written into
the guides yet.
