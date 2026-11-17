# Tasks

- [ ] 1. Correct the rustdoc and prepare delivery. Replace only the stale ActorSystem::nodes sentence with design.md's
  node-down-alert observability wording and authority boundary; keep the result description and runtime code intact.
  Inspect the diff and run `cargo doc -p cicada --no-deps`; add no wording or runtime tests. Write docs-handoff.md
  recording no guide change and the archive-time node-view Purpose correction. Prepare the normal next patch
  (currently 0.20.5), changelog and standard release pins/generated version metadata; no wire/contract semantics
  change. Verify affected docs/skill/version checks. The build loop corrects the spec Purpose when archiving.

- [ ] 2. Finishing. Run `openspec validate node-view-rustdoc --strict`. Commit implementation and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Full checks include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill checks, Node 26+ client-tests and Docker E2E. Record actual checks and limits.
  The builder does not push, tag, release or archive; the build loop owns landing/archive and obtains the owner's
  approval before publishing the patch. No extension of the docs-session release exemption is requested.
