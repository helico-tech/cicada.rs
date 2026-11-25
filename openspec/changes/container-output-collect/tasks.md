# Tasks

Normal lane, **BREAKING**, in the 0.23.0 batch with `container-progress` (which lands first and opens the section;
check the `release:` line of `python3 scripts/status.py`, builder.md, Version). Follow `design.md` and ADR 0066
(`docs/adr/0066-container-outputs-at-the-end.md`); the specs are `specs/container-actor/spec.md` (its MODIFIED progress
requirement needs `container-progress` archived first) and `specs/container-outputs/spec.md`. Run the crate's tests
with every feature: `cargo test -p cicada-containers --all-features`.

- [x] 1. **Types and the store's rules.** `Outputs` (`Outputs::new(path)`, 64 MiB and 1000 files),
  `ContainerJob::outputs` (default `None`), `OutputsReport`, serde and `JsonSchema` on `SkippedFile`, the optional
  `outputs` of `ContainerEnd`, the phase `Collecting` and the state field `pending` with the validity rule;
  `ContainerActor::with_store`. The store's tighter input rules (decision 6) in the helpers, `MemoryOutputStore` and
  `check_output_store`. Update the test job's contract snapshot. Verify: wire, schema and store tests.
- [x] 2. **The collecting phase.** With a store, an end the container ran for goes to `Collecting` with `pending`;
  `Start`, `Stop` and `Remove` during it as decision 1 says; a 0.22 `Ended` state never collects; without a store,
  today's single commit. Verify: actor tests for each of these.
- [x] 3. **The log and the folder.** The log drained at most 20 pages per turn with the guarded append (decision 3);
  then one turn of `copy_out` and `put_file` per file, refused files skipped, the report built (decision 4); the turn
  that finishes records the end with `outputs`, sets `Ended`, sends it and the last progress. Verify: the store and
  report tests of the design's Tests section, a running archiver next to it, two containers with one slow.
- [ ] 4. **Errors, changelog and docs handoff.** The rules of decision 5 (retries, `max_errors` in `Collecting`,
  `InvalidSpec`/`Unsupported` at once, a gone container); the rustdoc changes of decision 1. The 0.23.0 section names
  the outputs under Added and the contract change under its **BREAKING** line; the handoff carries the design's Docs
  section and the rollback warning. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: the error
  tests; `cargo test -p cicada-examples --test docs` and `--doc` (report `Docs: blocked` if a guide block stops
  compiling).
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-output-collect --strict`, commit, then
  `python3 scripts/gate.py run`.
