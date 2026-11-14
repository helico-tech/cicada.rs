## Implementation

- [x] 1. Add the core guide's actor-design section: consistency/serialization boundaries, stable keys and cardinality,
  bounded state/work, messages, external-effect idempotency and state/contract evolution. Reuse or extend the existing
  checked examples; add Rust blocks only with compiled doctest coverage. Check relevant flow/replay/cancellation and
  operations routes for missing guidance and put any necessary clarification in its canonical guide. Preserve the
  difference between readable old state, equal contracts and compatible migrations. Run
  `cargo test -p cicada-examples --doc` and `cargo test -p cicada-examples --test docs`.
- [x] 2. Add skills/cicada/SKILL.md and its Python 3.11+ standard-library source resolver. Implement bounded, locked,
  offline discovery with RUSTUP_AUTO_INSTALL=0, explicit target/features including --no-default-features, filtered
  resolve IDs, exact package names, verified roots, source groups and distinct version/revision/dirty provenance.
  Include --metadata fixtures and manual fallback guidance. Make routing paths resolve from the discovered source,
  never the installed folder, and preserve authorized consumer choices of forks/paths/commits. Run helper tests for
  git/path/patch/mixed/self/none/no-docs/unavailable, equal-version different commits, dirty sources, inherited workspace
  dependencies, target exclusions, disabled defaults, absent locks/tools/toolchains and timeouts. Use temporary trees
  and fake command probes to assert no fetch/install/unlocked retry or project-file change; do not require access to
  a private consumer repository. A manual run in this checkout should report self.
- [x] 3. Add examples/tests/skill.rs and fixture/evaluation resources under examples/tests/skill. Verify frontmatter,
  version, routing paths/anchors and maintained domain/guide/example coverage, and invoke the resolver fixtures.
  Reject consumer runtime routes to proposals/internal loops while allowing the explicit self/contributor branch.
  Avoid line-count/exact-wording assertions and unsupported claims that tests establish semantic completeness. Add
  behavioral cases for new SQLite integration, duplicate payments, flow cancellation, offline old sources, moved
  headings, mixed/forked dependencies, HTTP auth, unshipped features and contract-changing deploys. Exercise a
  representative subset with an independent agent in temporary consumer projects and record decisions/artifacts and
  limitations; no external model service is a CI dependency. Run `cargo test -p cicada-examples --test skill` and docs
  checks; keep runtime behavior in any changed example tests on virtual time.
- [x] 4. Add scripts/package-skill.py and release-workflow integration. Package only the resolved tag tree's skill
  files plus generated SOURCE.json into cicada-skill-vX.Y.Z.tar.gz with portable cicada/ paths and deterministic tar/
  gzip metadata. Verify tag/workspace/skill versions and provenance before attachment to gh release create, preserving
  all existing release gates. Test in temporary Git repositories/tags: identical repeat output, dirty checkout ignored,
  annotated tag dereference, missing files, mismatched versions, safe member names and exact unpacked content. Do not
  test by packaging a historical Cicada tag that predates the skill, publish a release or move a real tag. Run the
  package tests through the skill test target and validate the edited workflow syntax.
- [x] 5. Update ADR 0041/index as needed, README installation/update guidance and operations Releasing. Document
  Python/tool prerequisites, source routing, exact provenance versus version equality, new/older/offline projects,
  lack of assets on old releases, staging/diffing before replacement and the absence of automatic updates. Add the
  skill version to release checks and use the version being released, not the design's historical version. Update
  changelog/version and confirm guides/examples remain the knowledge source. Run examples doctests/docs tests and
  the targeted skill checks. Do not install into the owner's global skills folder or add marketplace tooling.

Note (2026-11-14, the owner's split of documentation ownership): the prose of tasks 1 and 5 (the core guide's
actor-design section, the README's install and update steps, the Releasing steps) was written and reviewed here, then
handed to the docs session in [docs-handoff.md](docs-handoff.md). This branch keeps the code, AGENTS.md, the changelog
and the version.

## Finishing checks

- [x] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate cicada-agent-skill --strict`. Record unavailable optional checks. Confirm CI exercises helper,
  routing and packaging checks without network/model services, and verify a staged artifact in a temporary consumer
  directory outside the Cicada checkout. State which behavioral evaluations actually ran; structural checks alone
  do not establish that the skill makes correct decisions for every task.
