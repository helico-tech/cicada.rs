# Tasks

Normal lane: in the unreleased batch (0.22.0; check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Follow `design.md`; the specs are `specs/container-runtime/spec.md`, `specs/container-docker/spec.md` and
`specs/container-actor/spec.md` (modified requirements). The fake gets nothing new (decision 5). Run the crate's tests
with every feature: `cargo test -p cicada-containers --all-features`.

- [ ] 1. **The kind and the Docker backend.** `logs` and `logs_since` of `DockerRuntime` map Docker's 501 to
  `Unsupported` with Docker's message; every other error keeps its mapping (decisions 1 and 2). The rustdoc of the
  trait's log calls, of `ErrorKind::Unsupported` and of `DockerRuntime` (which promises `Other` today) says the new
  kind. Verify: map unit tests (a 501 of a log read is `Unsupported`, a 500 stays `Other`);
  `cargo test -p cicada-containers --all-features`.
- [ ] 2. **The actor.** Its log read for an end treats `Unsupported` as final: the end is recorded at once with
  `(no logs: <message>)`, not counted as an error, the read not retried; the rest of the end (such as the removal
  poll of a goal `Remove`) as today; other kinds keep the count (decision 3). Verify: actor tests with the fake
  scripted by `fail_always(Method::Logs, ErrorKind::Unsupported)`, and `Other` still retried;
  `cargo test -p cicada-containers --all-features`.
- [ ] 3. **E2E, changelog and docs handoff.** In `e2e/tests/suite/container_docker.rs`: a container made with
  `docker create --log-driver none` through the CLI helper gives `Unsupported` from `logs` and `logs_since`, created
  and after it ran. The unreleased section of `CHANGELOG.md` names the kind under Changed, with the old promise of
  `Other` (decision 4). `docs-handoff.md` carries the kind and that a follower stops at `Unsupported`; nothing blocks.
  No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `CICADA_E2E=1 cargo test -p cicada-e2e --
  --nocapture` (or the gate below); `cargo test -p cicada-examples --test docs`.
- [ ] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-logs-unreadable --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
