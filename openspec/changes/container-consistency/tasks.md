# Tasks

Normal lane, a behaviour fix of two calls: 0.20.9, the patch after the unreleased 0.20.8 batch. It is released when
the whole container line, through `container-limits-and-kinds`, has landed (check the `release:` line of
`python3 scripts/status.py`, builder.md, Version). Follow `design.md`; the specs are `specs/container-runtime/spec.md`
and `specs/container-docker/spec.md`. Run the crate's tests with every feature:
`cargo test -p cicada-containers --all-features`.

- [x] 1. **One path rule for `copy_in`** (decision 1). Move `path_parts` and the choice of the deepest mount out of
  `crates/cicada-containers/src/docker/map.rs` into a private module compiled without features. Add two refusals with
  `InvalidSpec`: a path equal to a volume or bind mount target, and a path whose deepest mount is a tmpfs, on any root.
  `map::uploads` and the fake's `copy_in` (`src/fake.rs`) both call it; the backend takes the tmpfs targets from the
  inspected `HostConfig.tmpfs`, the fake from `spec.tmpfs`. The fake stores the cleaned-up path, so `/work/./a` and
  `/work/a` are one file, and keeps writing nothing on a refusal. Update the rustdoc of `ContainerRuntime::copy_in`
  (`src/runtime.rs`) and the `copy_in` note on `DockerRuntime` with the new cases. Verify: unit tests of the module
  (relative, `..`, `/`, empty and `.` parts, an exact target with and without a trailing `/`, a tmpfs target and a
  path below it, a volume below a tmpfs, the deepest mount); the fake's tests for an exact target on a read-only and
  a writable root, a tmpfs path on a writable root, `/`, and `/work/./a` then `/work/a` as one file; the map tests
  still pass; `cargo test -p cicada-containers --all-features`.
- [ ] 2. **A sweep names only what it removed** (decision 2). A pure `map::gone(resource, &error)`: 404 for every
  kind, 409 for a container only. The three loops of `DockerRuntime::sweep` (`src/docker/mod.rs`) leave a gone item,
  and a container another caller is removing, out of every list; a volume's 409 stays a skip. Update the rustdoc of
  `sweep` and of `Swept`'s lists. Verify: unit tests of `gone` for each kind and 404, 409 and another code;
  `cargo test -p cicada-containers --all-features`.
- [ ] 3. **`list` leaves out a container without a state** (decision 3). A pure `map::summary(&inspect_response)`
  returns `None` when Docker reports no state or an empty status; `DockerRuntime::list` skips it as it skips a
  container removed since the listing. `inspect`, `start`, `copy_in` and `stop` keep failing with `Other` for it.
  Update the rustdoc of `list` or the notes on `DockerRuntime`. Verify: unit tests of `summary` (a state, no state, an
  empty status); `cargo test -p cicada-containers --all-features`.
- [ ] 4. **E2E, changelog and docs handoff.** In `e2e/tests/suite/container_docker.rs`, extend the two `copy_in`
  steps: a path equal to the volume target `/work` fails with `InvalidSpec` and writes nothing, with a read-only root
  and with a writable one; on a writable root with a tmpfs at `/scratch`, `/scratch/c` fails with `InvalidSpec`. The
  0.20.9 section of `CHANGELOG.md` names under Fixed the refused paths (an exact mount target, a tmpfs path, `/` on the
  fake), the cleaned-up path on the fake, `Swept` without what was gone or being removed, and `list` without a
  container that has no state. `docs-handoff.md` carries the three rules for the container guide; nothing blocks. No
  edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `CICADA_E2E=1 cargo test -p cicada-e2e --
  --nocapture` (or the gate below); `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-consistency --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
