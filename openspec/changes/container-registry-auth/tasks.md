# Tasks

Normal lane, additive: 0.21.1, together with `container-info` (check the `release:` line of
`python3 scripts/status.py`, builder.md, Version). Follow `design.md`; the spec is `specs/container-docker/spec.md`.
Run the crate's tests with every feature: `cargo test -p cicada-containers --all-features`.

- [x] 1. **Reading the config.** A new `src/docker/auth.rs` (decisions 1, 3, 4 and 7): the folder (`docker_config`,
  else `DOCKER_CONFIG`, else `$HOME/.docker`), `config.json` read with `tokio::fs`, the registry of an image (Docker
  Hub as `https://index.docker.io/v1/`), key normalisation, the source by the CLI's order (`credHelpers`,
  `credsStore`, `auths`; `auths` unused when a helper is configured), `auth` decoded with `base64` 0.22 (a workspace
  dependency, only in the `docker` feature) and split at the first `:`, `identitytoken`. A missing, unreadable or
  broken file gives no credentials. Verify: the unit tests of the design's Tests section for these parts;
  `cargo test -p cicada-containers --all-features`.
- [x] 2. **Running a helper.** In `auth.rs` (decision 5): `docker-credential-<name> get` through `tokio::process`, found
  on `PATH`, the server key on stdin, killed after its limit (10 s in the backend; the runner takes the limit as an
  argument), its JSON answer read (`Username`, `Secret`; `<token>` makes an identity token). A missing helper, a
  non-zero exit, no JSON or the limit give no credentials and a reason (`not found`, `exited with <code>`,
  `timed out`, `gave no JSON`). Its output never leaves the function. Verify: helper scripts in a temp folder with a
  short real-time limit: an answer, `<token>`, a non-zero exit, no JSON, a sleep over the limit;
  `cargo test -p cicada-containers --all-features`.
- [x] 3. **The pull.** In `src/docker/mod.rs` (decisions 2 and 6): `DockerRuntime::docker_config(self, dir) -> Self`;
  `pull_if_missing` looks up the credentials only when it pulls and passes them to `create_image`; a failed pull's
  message ends with where the credentials came from or why there were none (`(credentials from …)`,
  `(no credentials for <registry>)`, `(no credentials: docker-credential-<name> <reason>)`); the error kinds stay as
  they are (decision 8). The `DockerRuntime` rustdoc that says a private image must be local gives the credential
  rules, the 10 s limit and `docker_config` instead. Verify: a unit test that a failed pull's message holds the
  source and never a credential; `cargo test -p cicada-containers --all-features`.
- [ ] 4. **E2E, changelog and docs handoff.** In `e2e/tests/suite/container_docker.rs`, the E2E case of the design's
  Tests section (a `registry:2` with htpasswd, a push with `docker --config <tempdir>`, a create with an `auths`
  entry, a failed create with an empty folder whose message says `no credentials for localhost:<port>`), cleaned up
  also after a failure; if the daemon cannot reach the registry, report it. The `## 0.21.1 — unreleased` section of
  `CHANGELOG.md` (made by `container-info`; make it if it is missing) names the pull with the machine's Docker login
  and `docker_config` under Added, and its summary line covers both changes. `docs-handoff.md` carries the
  credential rules (the file, the order, a failing helper, the 10 s limit, `docker_config`, `docker login` as the
  user the host runs as); nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture` (or the gate below);
  `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `cargo deny check advisories`, `openspec validate container-registry-auth --strict`,
  commit, then `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
