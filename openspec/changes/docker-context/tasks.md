# Tasks

Normal lane, additive: 0.21.2 (raise the patch number; check the `release:` line of `python3 scripts/status.py`,
builder.md, Version). Follow `design.md`; the spec is `specs/container-docker/spec.md`. Run the crate's tests with
every feature: `cargo test -p cicada-containers --all-features`.

- [x] 1. **The resolver.** A new `src/docker/context.rs` (decisions 1 to 4 and 6): a pure function of `DOCKER_HOST`,
  `DOCKER_CONTEXT` (empty counts as unset), the config folder (`auth::config_dir(None)`) and whether the local socket
  exists, giving the endpoint to use or the error; the meta path `contexts/meta/<sha256 hex>/meta.json` and
  `Endpoints.docker.Host`; the TLS check (`SkipTLSVerify`, `contexts/tls/<sha256>/docker`); the schemes `unix`, `tcp`,
  `http` only; every error naming the context and where its name came from. `dep:sha2` joins the `docker` feature in
  `crates/cicada-containers/Cargo.toml`. Verify: the unit tests of the design's Tests section for the resolver;
  `cargo test -p cicada-containers --all-features`.
- [x] 2. **connect().** In `src/docker/mod.rs` (decision 5): `connect()` gathers the inputs, calls the resolver, and
  connects a context's endpoint through the code of `connect_to`; `DOCKER_HOST` and the local socket go as today. A
  unix endpoint whose socket file is missing is `DaemonUnavailable`, naming the context and the endpoint. The rustdoc
  of `DockerRuntime`, `connect()` and `connect_to` gives the order and the TLS refusal instead of "contexts are not
  read". Verify: a unit test for the missing socket of a context; `cargo test -p cicada-containers --all-features`.
- [ ] 3. **E2E, changelog and docs handoff.** In `e2e/tests/suite/container_docker.rs`, the helper `connect()` calls
  `DockerRuntime::connect()` alone; the CLI helper of the registry test keeps `-H` with the context's host; the
  helper's doc comment says what the design's Tests section says about a machine with both a socket and another
  context. A new section `## 0.21.2 — unreleased` in `CHANGELOG.md`, above 0.21.1, with a one-line summary, names the
  context order and the TLS refusal under Changed. `docs-handoff.md` carries the order, the TLS refusal and
  `connect_to` for other setups; nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture` (or the gate below) on this machine, which has no
  `/var/run/docker.sock`; `cargo test -p cicada-examples --test docs`.
- [ ] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate docker-context --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
