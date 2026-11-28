# Tasks

Normal lane: in the unreleased batch (0.26.0; check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Follow `design.md`; the specs are `specs/container-runtime/spec.md` and `specs/container-docker/spec.md`.
Run the crate's tests with every feature: `cargo test -p cicada-containers --all-features`.

- [x] 1. **The spec field and the fake.** `NetworkSpec` gets `isolated: bool` (false in `new`), with a doc that says
  what it closes and that it needs `internal` (decision 1). `FakeRuntime::ensure_network` fails an isolated spec that
  is not internal with `InvalidSpec` and stores nothing, keeps `isolated` with the network, and fails with `Conflict`
  when an existing network's `internal` or `isolated` differs; the message names the flag (decisions 2 and 5). The
  rustdoc of the trait's `ensure_network` (an implementation honours `isolated` or fails with `Unsupported`) and of
  `ErrorKind::Conflict` and `Unsupported` names the new cases (decision 8). Verify: fake unit tests for the
  scenarios of `specs/container-runtime/spec.md`; `cargo test -p cicada-containers --all-features`.
- [x] 2. **The Docker backend.** In `docker/map.rs`, three pure functions (decision 6): the check of a spec against
  the agreed API version (`InvalidSpec` when isolated without internal, then `Unsupported` below 1.48 with a message
  naming the agreed version and Docker 28); the create request (both `gateway_mode_ipv4` and `gateway_mode_ipv6`
  options set to `isolated` for an isolated spec, no options otherwise); and `(internal, isolated)` read from an
  inspected network (isolated only when both options are `isolated`). `ensure_network` in `docker/mod.rs` calls the
  check first, before it looks for an existing network, then compares both flags with `Conflict` on a difference,
  as today on the create race too (decisions 3–5). Verify: map unit tests for each case, including API 1.47 and
  1.48, and an inspected network with only one of the two options; `cargo test -p cicada-containers
  --all-features`.
- [x] 3. **E2E.** In `e2e/tests/suite/container_docker.rs`, a test as decision 7 says: an isolated network is created
  and repeats with `Ok`; `docker network inspect` shows both options; each flag conflicts both ways; a listener
  started with the docker CLI on the host network is reached from a container on a plain internal network through
  its gateway address, and not from a container on the isolated network through the first address of its subnet.
  Label everything so the suite's sweep removes it. If host networking does not work on the E2E host, report it as
  a design question; do not drop the check. Verify: `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`.
- [x] 4. **Changelog and docs handoff.** The unreleased section of `CHANGELOG.md` gets an Added line: `isolated` on
  `NetworkSpec`, Docker 28 for it, and that a host's own `ContainerRuntime` written before it ignores the flag
  (decision 8). `docs-handoff.md` carries the paragraph and the upgrade step from "Docs this breaks" in the design;
  nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-network-isolation --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
