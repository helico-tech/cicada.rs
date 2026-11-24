# Tasks

Normal lane, additive: 0.20.9, with the rest of the container line, released when the line, through
`container-limits-and-kinds`, has landed (check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Builds on `container-consistency` (the shared path rule, landed) and follows `container-logs` and
`container-copy-out` in the order (the default-method pattern). Follow `design.md`; the specs are
`specs/container-runtime/spec.md` and `specs/container-docker/spec.md`. Run the crate's tests with every feature:
`cargo test -p cicada-containers --all-features`.

- [ ] 1. **The type and the trait method.** In `crates/cicada-containers/src/`: `CopyDir` (`non_exhaustive`,
  `CopyDir::new(path)` with mode 0o755), exported from `lib.rs`; `ContainerRuntime::make_dirs` with a default that
  fails with `Other` (decision 1). Its rustdoc names the rules of decisions 2, 3 and 5 and the design's limits (a
  volume root's mode, an existing folder keeps its mode and owner, the race with another caller). Verify: a unit test
  that a runtime without `make_dirs` compiles and fails with `Other`; `cargo test -p cicada-containers --all-features`.
- [ ] 2. **The fake.** `make_dirs` in `src/fake.rs` (decision 6): the checks of decision 3 (state, shared path rule,
  mode above 0o7777) and decision 2 (a held file at the path or a parent is `Conflict`; existing folders, `/` and the
  parents of held entries are left alone; a named mode beats the implied 0755 in any order; one path with two modes
  is `InvalidSpec`, with one mode it is made once), all before any change; made folders kept with their modes;
  `Call::MakeDirs` and `Method::MakeDirs`. The fake has no links. Verify: the fake's tests for every scenario of the
  runtime spec (a folder made twice, an existing folder left alone with a new child, a file at the path and at a
  parent, a named parent in either order, one path named twice, a bad mode, a late call) and a scripted failure;
  `cargo test -p cicada-containers --all-features`.
- [ ] 3. **The Docker backend.** `DockerRuntime::make_dirs` in `src/docker/mod.rs`, pure parts in `src/docker/map.rs`
  (decisions 4 and 5): find the container (`Conflict` unless `Created`); check paths, modes and overlaps; ask
  `get_container_archive_info` for each named folder; on 404, or any other error, walk its parents upward until one
  exists (a parent that is a file is `Conflict`; with no such parent a non-404 error maps by `kind_of`); follow a
  link to its target, relative ones against the link's folder, cleaned, up to 8 hops (a folder counts as existing,
  anything else and a ninth hop are `Conflict`); a 404 whose container is gone (a second inspect) is `Conflict`;
  apply the path rule to the resolved paths; then upload a tar of only the missing folders under their resolved
  paths, parents first, uid and gid 0, through the uploads of `copy_in`. Verify: map unit tests (the tar holds only
  the missing folders, parents first, with their modes and resolved paths; the folder bit and a link target of a
  stat; link resolution: absolute, relative, `..` stopping at `/`, a ninth hop; grouping per mount with a read-only
  root); `cargo test -p cicada-containers --all-features`.
- [ ] 4. **E2E, changelog and docs handoff.** In `e2e/tests/suite/container_docker.rs`, the cases of the design's
  Tests section: `/out` at 0777 written by a program running as user 1000; `/tmp/out` leaves `/tmp` at 1777;
  `/run/postgresql` keeps the owner and mode it had before the call; `/var/run/app` makes `/run/app` through the
  image's link and `/var/run` stays a link; a file at the path, and `/etc/passwd/x` below a file, are `Conflict`
  (this pins Docker's answer for a path below a file); a read-only root with a volume at `/work`; `make_dirs` after
  start is `Conflict`. The 0.20.9 section of `CHANGELOG.md` names `CopyDir` and `make_dirs` under Added.
  `docs-handoff.md` carries `CopyDir`, `make_dirs`, its rules (existing folders untouched, links followed to a
  folder, named modes, one mode per path) and the limits for the container guide; nothing blocks. No edit to
  `README.md`, `AGENTS.md` or the handwritten docs. Verify: `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`
  (or the gate below); `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-make-dirs --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
