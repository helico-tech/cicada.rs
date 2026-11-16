# Tasks

- [x] 1. Interactive skill and saved template. Add skills/actor-design/SKILL.md and assets/design-template.md with
  the designed source selection, sibling metadata/provenance checks, fallback behavior, confirmation/delegation,
  diagrams, safe document updates and resumption. Use the landed guide's nine step names/order. Mention the skill
  by name in cicada without a sibling source route. Keep knowledge in the resolved guide and review mode out of
  scope. Verify representative new/resumed conversations and source/sibling limitations against the design; record
  transcripts and remaining uncertainty for review rather than claiming model compliance from text inspection.

- [x] 2. Structural checks and behavioral cases. Extend examples/tests/skill.rs for both versions/names, explicit
  Source routes, template/guide step agreement and forbidden contributor routes, retaining all cicada coverage.
  Add examples/tests/skill/actor-design-evals.json for the designed staged cases and check shape/coverage. Exercise
  representative cases for premature advancement, delegated decisions, meaningful diagrams and preserved work.
  Verify `cargo test -p cicada-examples --test skill`. Manually check plugin discovery and sibling installation when
  a suitable harness is available; record unavailable evidence explicitly, never add a paid model CI dependency.

- [x] 3. Two-skill archive with historical compatibility. Add scripts/skill-archive-format and implement format 2
  in package-skill.py and shared archive helpers using explicit regular tagged blobs, both provenance files and safe
  deterministic members. Select legacy only when marker and actor-design tree are absent; retain legacy archive
  bytes and fixture behavior. Extend examples/tests/skill/test_package_skill.py for both layouts, fixed legacy
  digest, version/marker/missing-file failures, unsafe members and cwd independence. Verify those Python tests,
  relevant shared archive tests and `cargo test -p cicada-examples --test skill`.

- [x] 4. Release preflight and recovery. Require marker 2 and both skill versions in current preflight. Update
  gate.py's pinned-commit archive selection and verification to check every SOURCE.json and tagged file correctly.
  Extend scripts/test_gate_release.py and relevant release-preflight cases for old/new layouts, provenance/member
  failures, saved legacy digests and format-2 digests saved before a driver update. Never rewrite saved digests or
  immutable tags. Verify targeted script tests, including fixture archive compatibility and resumed publication.

- [x] 5. Handoff, ADR and release version. Reconcile ADR 0051/index and write docs-handoff.md for both-folder
  install/update, docs entry points and releasing-guide version/marker instructions, preserving existing anchors.
  Name the default artifact path, source limits, scope of each skill, manual evidence and legacy reproducibility.
  Integrate exact docs-owner fixes for any protected-prose route/doctest blockers. Apply the next patch version
  through workspace/lock, both skills, plugin, OpenAPI pins and changelog using normal release steps. Verify docs,
  doctests, skill and relevant release tests after integration; do not edit protected prose independently.

- [ ] 6. Finishing. Run `openspec validate actor-design-skill --strict`. Commit the implementation and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Its full checks include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill checks and Docker E2E. Record behavioral/manual checks and limits. The builder
  does not push, tag, release or archive; publication remains with the build loop.
