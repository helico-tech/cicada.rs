# Tasks

Normal lane, additive: 0.20.9, with the rest of the container line, released when the line, through
`container-limits-and-kinds`, has landed (check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Builds on `container-consistency` (the shared path rule) and `container-logs` (the default-method pattern),
which must have landed first. Follow `design.md`; the specs are `specs/container-runtime/spec.md` and
`specs/container-docker/spec.md`. Run the crate's tests with every feature:
`cargo test -p cicada-containers --all-features`.

- [ ] 1. **Types and the collector.** In `crates/cicada-containers/src/`: `CopyLimits` (with `new`), `CopyOut` and
  `SkippedFile`, all `non_exhaustive` (decisions 1-2), exported from `lib.rs`; the private collector of decision 7:
  entries in order (path, kind, size, a way to read the contents), the byte budget with skip and go on, the file
  count stop only when another countable entry follows, regular files only, links and other kinds and names outside
  `path` skipped with a reason (decision 3), a kept file read into a buffer of exactly its size. Remove the `allow(dead_code)` on `src/paths.rs` once `copy_out` uses the module in every build (from the
  container-consistency review). Verify: unit tests
  of the collector (budget skip, the stop with its last entry, exactly at the limit without it, links, a device, a
  name outside the folder, a file never cut); `cargo test -p cicada-containers --all-features`.
- [ ] 2. **The trait and the fake.** `ContainerRuntime::copy_out` with a default that fails with `Other` (decision 9);
  its rustdoc names the limits and the memory bound, the link rule, `InvalidSpec` for a bad path or a zero limit,
  `Conflict` and the empty result, the tmpfs rule, the rename advice and the Limits section of the design. The
  fake (decision 8): `put_file`, `copy_out` through the shared path parsing and the collector, the walk order part
  by part, files written to a tmpfs left out once the container ended, `Call::CopyOut` and `Method::CopyOut`.
  Verify: a unit test that a runtime without `copy_out` compiles and fails with `Other`; the fake's tests for the
  scenarios of the runtime spec (folder, one file too big, too many files, exactly at the limit, zero limits, what is
  not there, a file the program wrote, a tmpfs after the end) and a scripted failure;
  `cargo test -p cicada-containers --all-features`.
- [ ] 3. **The Docker backend.** `DockerRuntime::copy_out` in `src/docker/mod.rs`, the archive reader apart so it can
  be tested on an in-memory tar (decision 6): check the path and limits first; find the container (`Conflict`);
  `download_from_container` through `StreamReader` and `SyncIoBridge` into `tar` in `spawn_blocking`; the first
  bollard error kept in a side slot and mapped back with `kind_of`; a 404 then a second inspect (empty result, or
  `Conflict` when the container is gone); any other error fails the whole call with no partial result. Add
  `tokio-util` with `io` and `io-util` to the feature `docker` (no new crate in `Cargo.lock`). Verify: unit tests of
  the reader (a long PAX name, a big skipped entry never read into memory, a stream error mid-file coming back as
  `DaemonUnavailable`, a 404 first chunk); `cargo test -p cicada-containers --all-features`;
  `cargo tree -p cicada-containers --features docker` shows no new crate.
- [ ] 4. **E2E, changelog and docs handoff.** In `e2e/tests/suite/container_docker.rs`, the cases of the design's
  Tests section, including the container spec's scenarios that a real daemon can stage (artifacts on the root and a
  volume after the end, a progress file while running, a tmpfs after the end). The 0.20.9 section of `CHANGELOG.md`
  names `copy_out`, `CopyLimits`, `CopyOut`, `SkippedFile` and the fake's `put_file` under Added. `docs-handoff.md`
  carries the call, the limits and the memory bound, the link rule, the tmpfs note, the rename advice and the
  stated limits for the container guide; nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs.
  Verify: `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture` (or the gate below);
  `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-copy-out --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
