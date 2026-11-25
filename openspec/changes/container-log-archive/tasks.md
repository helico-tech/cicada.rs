# Tasks

Normal lane, additive: in the unreleased batch (check the `release:` line of `python3 scripts/status.py`, builder.md,
Version; after the 0.22.0 release it is a new patch). Builds on `container-output-store` (lands first) and `app-tasks`.
Follow `design.md`; the specs are `specs/container-outputs/spec.md` and `specs/container-actor/spec.md`. Run the crate's
tests with every feature: `cargo test -p cicada-containers --all-features`.

- [x] 1. **The label.** `ContainerActor` adds `cicada.container.prefix` = the settings' prefix to every spec it creates,
  after the job's `spec` (decision 1). Verify: an actor test with a job that sets its own labels;
  `cargo test -p cicada-containers --all-features`.
- [ ] 2. **The archiver and its settings.** `LogArchiver::new(runtime, store, prefix, settings)`, `ArchiveSettings`
  (interval, `page_lines` checked against 1 to `LOG_PAGE_MAX` with `InvalidArchive`, `max_parallel`) and
  `run(stop)` (decisions 2 and 3): rounds that never wait, one read per container at a time, `log_end` then
  `logs_since` pages appended with the guarded `append_logs`, empty unmoved pages not written, the drained set pruned
  to `list`'s names. `tracing` becomes a dependency of `cicada-containers`. Verify: the tests of the design's Tests
  section for rounds, restarts, two archivers and a hanging read; `cargo test -p cicada-containers --all-features`.
- [ ] 3. **Errors.** The rules of decision 4: a failed `list` retried; `Unsupported` drained and logged once;
  `Conflict` dropped; every other kind retried with the wait `interval × 2^(n−5)`, at most 60 s, after 5 failures;
  the store's `Conflict`, `Unavailable`, `Other` and `Invalid`. Verify: the error tests of the design;
  `cargo test -p cicada-containers --all-features`.
- [ ] 4. **Changelog and docs handoff.** The unreleased section of `CHANGELOG.md` names `LogArchiver`, `ArchiveSettings`
  and the label under Added. `docs-handoff.md` carries what the design's Docs section lists, the limits included;
  nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-log-archive --strict`, commit, then
  `python3 scripts/gate.py run`.
