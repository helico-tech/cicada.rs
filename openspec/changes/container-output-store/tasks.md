# Tasks

Normal lane, additive: in the unreleased batch (0.22.0; check the `release:` line of `python3 scripts/status.py`,
builder.md, Version). Follow `design.md`; the spec is `specs/container-outputs/spec.md` (a new capability). Run the
crate's tests with every feature: `cargo test -p cicada-containers --all-features`.

- [x] 1. **The trait and its types.** A new module in `crates/cicada-containers/src/` with `OutputStore` (async,
  `Send + Sync + 'static`, the seven methods of the design), `LogEnd`, `StoredLine`, `StoredFile`, `OutputError` and
  `OutputErrorKind` (`Invalid`, `Conflict`, `Unavailable`, `Other`; `is_retryable` for `Unavailable` only), and the
  public `check_container_name` and `clean_output_path` (decisions 1 to 6). Serde for `LogLine` and `LogStream`
  (decision 7). Rustdoc states the one-write rule, the input rules and what a store promises that the check cannot
  prove (decision 8). Export from `lib.rs`. Verify: unit tests of the two input helpers and the serde form;
  `cargo test -p cicada-containers --all-features`.
- [x] 2. **The in-memory store.** `MemoryOutputStore` behind `testing`: the rules of decisions 2 to 6, the guarded
  append under one lock, the input rules before anything else. Verify: its own tests;
  `cargo test -p cicada-containers --all-features`.
- [x] 3. **The conformance check.** `check_output_store(store)` behind `testing`, with its own container names, covering
  the design's Tests section, and panicking with the rule it broke; it passes on `MemoryOutputStore`, and a
  deliberately broken store in a test makes it panic. Verify: `cargo test -p cicada-containers --all-features`.
- [ ] 4. **Changelog and docs handoff.** The 0.22.0 section of `CHANGELOG.md` names `OutputStore`, its types, the
  helpers, `MemoryOutputStore`, `check_output_store` and serde on `LogLine` under Added. `docs-handoff.md` carries
  the trait, the one-write rule, the input rules and the check for `docs/guide/containers.md`; nothing blocks. No edit
  to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-output-store --strict`, commit, then
  `python3 scripts/gate.py run`.
