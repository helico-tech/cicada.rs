# Tasks

Normal lane, version 0.26.0 (unreleased). Follow `design.md`; the specs are `specs/inspector/spec.md` (removed) and
`specs/change-loop/spec.md`. ADR 0078 and the ADR status lines are already on `main`: no ADR task. Nothing shipped
has the inspector, so nothing is deprecated.

- [ ] 1. **Delete `cicada-inspector`.** Remove `crates/cicada-inspector`, its workspace member and dependency in
  `Cargo.toml`, and let `Cargo.lock` follow (decision 1). Verify: `cargo build --workspace`; `cargo tree --workspace`
  names no `cicada-inspector`.
- [ ] 2. **The tray without the inspector.** Remove the feature `inspector` and the optional dependency of
  `cicada-tray`; in `menu.rs` `InspectorItem`, `MenuModel`'s field `inspector` and the `close_inspector` parameter of
  `QuitRequest::begin`; in `appkit.rs` and `appkit/view.rs` the window, its runtime handle, the menu item and its
  action; the tests the design names (decision 1). Every other row keeps its order. Verify: `cargo test -p
  cicada-tray`; `cargo clippy -p cicada-tray --all-targets -- -D warnings`.
- [ ] 3. **`cicada_app::context_client` goes.** The function, its re-export in `lib.rs` and its tests (decision 4b);
  `Contexts`, `ContextFile`, `ContextEntry` and `check_server_url` stay. Verify: `cargo test -p cicada-app`;
  `cargo check -p cicada-app --features tui`.
- [ ] 4. **Gate scripts, changelog and docs handoff.** `scripts/ci/feature-builds.sh` and
  `scripts/ci/dependency-trees.sh` as decision 5 says; the 0.26.0 section of `CHANGELOG.md` without the inspector and
  `context_client` (decision 4); `docs-handoff.md` from the design's "Docs this breaks". No edit to `README.md`,
  `AGENTS.md`, `skills/` or the handwritten docs. Verify: `bash scripts/ci/dependency-trees.sh`;
  `python3 -m unittest discover -s scripts -p 'test_*.py'`; `grep -rn -i inspector CHANGELOG.md scripts crates`
  finds nothing outside the 0.25.0 and older sections.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate remove-inspector --strict`, commit, then `python3 scripts/gate.py run`.
  `cargo test -p cicada-examples --test skill` and `--test docs` fail until the docs session has removed the routes and
  sections the handoff names: report that as `Docs: blocked`, not as a failure to fix.
