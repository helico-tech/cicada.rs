# Tasks

Normal lane, additive: in the unreleased batch (check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Follow `design.md`; the spec is `specs/container-outputs/spec.md`. Run the crate's tests with every feature:
`cargo test -p cicada-containers --all-features`.

- [ ] 1. **The watch and its settings.** `OrphanWatch<J>::new(runtime, prefix, settings, on_orphan) -> Result`,
  `OrphanSettings` (interval 5 minutes, zero refused) and `run(system, stop)` (decision 1). Verify: the refusal test;
  `cargo test -p cicada-containers --all-features`.
- [ ] 2. **Rounds and the alert.** List by `PREFIX_LABEL`, the address from `J::TYPE` and the name without the prefix,
  `ActorSystem::get_actor`; an orphan after two rounds in a row without its actor, reported once; the set pruned;
  names without the prefix skipped with one warning (decision 2). Verify: the tests of the design's Tests section.
- [ ] 3. **Errors.** A failed `list` or `get_actor` logged and retried, not counted; nothing ends `run`; the limits in
  the rustdoc (decision 3). Verify: the error tests.
- [ ] 4. **Changelog and docs handoff.** The unreleased section names `OrphanWatch` and `OrphanSettings` under Added;
  the handoff carries the design's Docs section and the limits. No edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-orphan-alert --strict`, commit, then
  `python3 scripts/gate.py run`.
