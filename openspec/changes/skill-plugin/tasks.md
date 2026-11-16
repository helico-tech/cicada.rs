# Tasks

- [x] 1. Root marketplace and plugin. Add the two `.claude-plugin/` manifests with the exact fields and shapes in
  design decisions 1 and 2, the current workspace version, one `./` source and default skills/ discovery. Keep the
  archive layout and contributor loops unchanged. Verify with `claude --version`, then `claude plugin validate` on
  each explicit manifest and the repository root; record outputs and warnings for code review. This is review
  evidence, not a gate dependency or proof of private Git authentication, installation or interactive loading.

- [x] 2. Structural tests and release preflight. Extend `examples/tests/skill.rs` to check the manifest fields,
  types, names, source, versions and consumer-only component boundary in design decision 5. Use temporary fixtures
  for malformed/missing manifests, bad fields/types, extra entries, non-root sources, name/version mismatches,
  component overrides, every forbidden root path and symlinks; accept a second ordinary skill. Require real skill
  directories and regular SKILL.md files. Extend `scripts/release-check.sh` with a jq check of a string plugin
  version matching the release, failing before publication. Update `scripts/test_gate_release.py` fixtures and
  cover missing, malformed, absent/non-string and mismatched plugin versions, including no publication on failure.
  Verify `cargo test -p cicada-examples --test skill` (including existing resolver/archive tests) and
  `python3 -m unittest discover -s scripts -p 'test_gate_release.py'`. Do not change the gate policy or packager.

- [ ] 3. Portable skill installation guidance. Keep the helper's portable folder placeholder and explain replacing
  it with the actual shell-quoted installed path while running from the consumer project. Update SKILL.md section 7
  with plugin-managed updates, the staged archive alternative, comparison/preservation of edits before switching,
  duplicate-copy removal after comparison, and plugin-cache customization limits. Preserve resolved-source authority,
  missing SOURCE.json behavior and all existing routes. Verify `cargo test -p cicada-examples --test skill`; review
  both an archive and plugin installation example without installing a plugin or changing account settings.

- [ ] 4. Release metadata, ADR and docs handoff. Keep ADR 0049/index aligned with the implementation. Apply the next
  patch release through the normal workspace/lock/skill/plugin/OpenAPI version steps and changelog. Write
  `docs-handoff.md` for the installation guide's existing install/update sections and the release guide's version
  preparation/reset and review-validation instructions, with the exact topics and preserved anchors in the design.
  No Rust doc block, guide/example, skill route or guarantee row is expected to break; report any unexpected
  protected-prose failure and obtain the docs session's exact fix before landing. Do not edit README, AGENTS.md or
  handwritten docs; README release pins retain their existing exception. Verify examples doctests, docs links and
  skill tests with `cargo test -p cicada-examples --doc`, `cargo test -p cicada-examples --test docs` and
  `cargo test -p cicada-examples --test skill`. Repeat vendor validation on the final versioned tree for code review.

- [ ] 5. Finishing. Run `openspec validate skill-plugin --strict`. Commit the final implementation tree and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. The full gate must include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  independent feature builds, script/docs/skill tests and Docker E2E. Record CLI validation evidence and its limits
  alongside the gate result. Do not push, tag or release from the builder; publication belongs to the build loop
  after the owner's approval.
