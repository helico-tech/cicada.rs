# Tasks

Normal lane, additive: 0.21.1, released together with `container-registry-auth` (check the `release:` line of
`python3 scripts/status.py`, builder.md, Version). Follow `design.md`; the specs are `specs/container-runtime/spec.md`
and `specs/container-docker/spec.md`. Run the crate's tests with every feature:
`cargo test -p cicada-containers --all-features`.

- [x] 1. **The type and the trait method.** In `crates/cicada-containers/src/runtime.rs`: `RuntimeInfo`
  (`#[non_exhaustive]`, public `arch` and `os`, `RuntimeInfo::new(arch, os)`), exported from `lib.rs`;
  `ContainerRuntime::info` after `sweep`, with a default that fails with `Unsupported` (decisions 1 to 3). Its rustdoc
  names Docker's `/version`, the spelling of `arch` (image platforms, no variant), bollard's 120 s timeout and
  `tokio::time::timeout` for a faster answer, and that a 5xx and a Docker below the API floor give `Other`, so a
  liveness check treats every error but `Unsupported` as "not here" (decision 4). The doc of `ErrorKind::Unsupported`
  names `info`. Verify: a unit test that a runtime without `info` compiles and fails with `Unsupported`;
  `cargo test -p cicada-containers --all-features`.
- [x] 2. **The fake.** `info` in `src/fake.rs` (decision 5): `amd64` on `linux` by default, `set_info(RuntimeInfo)`,
  `Call::Info` and `Method::Info` with `fail_next` and `fail_always`. Verify: tests for the default, `set_info`, a
  scripted `DaemonUnavailable` followed by an answer with two `Info` calls recorded, and a repeated call that leaves a
  created container `Created`; `cargo test -p cicada-containers --all-features`.
- [x] 3. **The Docker backend.** `DockerRuntime::info` in `src/docker/mod.rs` through `docker()` and bollard's
  `version()`, the map from `SystemVersion` in `src/docker/map.rs` (a missing or empty `Arch` or `Os` is `Other`),
  errors through `kind_of` (decision 4). Verify: map unit tests; a socket without a daemon gives `DaemonUnavailable`
  twice; `cargo test -p cicada-containers --all-features`.
- [ ] 4. **E2E, changelog and docs handoff.** In `e2e/tests/suite/container_docker.rs`: `info()` gives `os` `linux`
  and the `arch` that `docker version --format '{{.Server.Arch}}'` prints for the same context. A new section
  `## 0.21.1 — unreleased` in `CHANGELOG.md`, above 0.21.0, with a one-line summary, names `RuntimeInfo` and `info` under Added.
  `docs-handoff.md` carries `info`, `RuntimeInfo`, the spelling of `arch`, the timeout hint and the liveness rule
  (every error but `Unsupported` means "not here"); nothing blocks. No edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Verify: `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture` (or the gate below);
  `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-info --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
