# Tasks

Normal lane, additive: in the unreleased batch with `docker-context` (check the `release:` line of
`python3 scripts/status.py`, builder.md, Version). Follow `design.md`; the spec is `specs/container-actor/spec.md`.
Run the crate's tests with every feature: `cargo test -p cicada-containers --all-features`.

- [ ] 1. **The constant.** `ContainerJob::MAX_TURN: Option<Duration> = None` in `crates/cicada-containers/src/actor/`,
  and `const MAX_TURN: Option<Duration> = J::MAX_TURN` in `impl Actor for ContainerActor<J>` (decisions 1 and 2). Its
  rustdoc names the create step and the stop's `stop_grace` as the long turns, says to set the limit above both, and
  what a timeout does (decision 3). Verify: a unit test that the actor's `MAX_TURN` is the job's, and `None` without
  it; `cargo test -p cicada-containers --all-features`.
- [ ] 2. **Behaviour on virtual time.** With `TestActors` and a test runtime that wraps the fake and sleeps in
  `create`: a step longer than the node's default but shorter than the job's limit ends `done`; a job limit below the
  step ends it `failed` (`timeout`) and the retry makes no second container; a zero `MAX_TURN` is refused at
  registration. Verify: `cargo test -p cicada-containers --all-features`.
- [ ] 3. **Changelog and docs handoff.** The unreleased section of `CHANGELOG.md` (made by `docker-context`; make it
  if it is missing) names `ContainerJob::MAX_TURN` under Added. `docs-handoff.md` carries `MAX_TURN`, when to set it
  and what a timeout does; nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --test docs`.
- [ ] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-job-max-turn --strict`, commit, then
  `python3 scripts/gate.py run`.
