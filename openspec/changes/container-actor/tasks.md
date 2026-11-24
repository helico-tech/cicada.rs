# Tasks

Normal lane, additive: the version of the unreleased batch (0.20.7). Follow `design.md` and ADR 0057; the spec is
`specs/container-actor/spec.md`. Runtime behaviour is tested on virtual time (`#[tokio::test(start_paused = true)]`
with `TestActors` and `FakeRuntime`).

- [x] 1. **Types, settings and text.** In `crates/cicada-containers`: the dependency on `cicada` (with `serde`,
  `serde_json`, `schemars`), `ContainerJob`, `ContainerActor::new(job, runtime, settings)`, `ContainerSettings` with
  its defaults and the prefix rule (decisions 5, 7), the messages `Start`, `Stop`, `Remove`, `Check` and the internal
  `Poll`, `ContainerState` and `ContainerEnd` (decisions 1-3), and the text clean-up (decision 8). Verify: unit tests
  of the name rule, the settings and the text caps (NUL, lines, bytes on a character boundary);
  `cargo test -p cicada-containers`.
- [x] 2. **The poll.** The goal and phase steps of decision 4, the armed poll and its re-arming (decision 6), errors in a
  row with backoff, permanent create errors, the deadline, and the end saved and sent in one commit, with a refused send
  saved as undelivered (decision 3). Verify: `cargo test -p cicada-containers` (unit tests of the step table where it
  helps); `cargo clippy -p cicada-containers --all-targets`.
- [x] 3. **Virtual-time tests.** `crates/cicada-containers/tests/actor.rs` with the dev-dependencies of "Contract and
  tests", covering every scenario of the spec and the test list of the design. Verify:
  `cargo test -p cicada-containers --test actor`.
- [ ] 4. **Contract and resolver.** A test job's contract pinned in `crates/cicada-containers/tests/contracts/` with
  samples of every message, each phase's state and an end payload (`CICADA_UPDATE_CONTRACTS=true` writes it).
  `cicada-containers` in `PACKAGES` of `skills/cicada/scripts/cicada_source.py`, with its Python test. Verify:
  `cargo test -p cicada-containers`, `cargo test -p cicada-examples --test skill`.
- [ ] 5. **Changelog and docs handoff.** The `0.20.7 — unreleased` changelog section names the container actor and
  ADR 0057. `docs-handoff.md` from "Docs it breaks"; nothing blocks. Verify: `cargo test -p cicada-examples --test docs`.
- [ ] 6. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-actor --strict`, commit, then `python3 scripts/gate.py run`
  (no E2E: the actor runs on the fake; E2E runs at the release).
