# Tasks

Normal lane, in the **BREAKING** 0.23.0 batch (a state field; check the `release:` line of `python3 scripts/status.py`,
builder.md, Version). Builds on `container-output-collect` (`ContainerActor::with_store`; lands first). Follow
`design.md`; the spec is `specs/container-actor/spec.md`. Run the crate's tests with every feature:
`cargo test -p cicada-containers --all-features`.

- [x] 1. **The setting and the state.** `ContainerSettings::output_retention: Option<Duration>` (default `None`); the
  state field `outputs_until: Option<Timestamp>`, valid only in `Removed`, capped at the largest instant; the contract
  snapshot updated. Verify: wire and validity tests.
- [x] 2. **The due poll.** The removal step saves `outputs_until` and arms a poll (with a store and a retention);
  `work_left` is true in `Removed` while `outputs_until` is set, so `Check` re-arms it; the poll calls
  `OutputStore::remove` and clears it (decisions 1 and 3). Verify: the deletion after the retention, a lost poll and
  `Check`, a restart, a retention unset later.
- [x] 3. **Errors.** `Unavailable` and `Other` retry with the backoff without limit; `Invalid` clears with a warning;
  no store at the due time clears with a warning (decision 3). Verify: the error tests of the design.
- [x] 4. **Changelog and docs handoff.** The 0.23.0 section names `output_retention` under Added and the state field
  in its BREAKING notes; the handoff carries the design's Docs section. No edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc`.
- [x] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-output-retention --strict`, commit, then
  `python3 scripts/gate.py run`.
