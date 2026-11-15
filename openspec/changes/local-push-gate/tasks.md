## Implementation

- [x] 1. Implement the versioned check policy and shared `scripts/ci` commands, preserving every current CI category:
  Python tests, fmt, clippy with warnings denied, PostgreSQL workspace tests, core/app isolated-feature builds with
  warnings denied, app-alone tests and all terminal/driver/TLS/crypto dependency checks. Add full/docs/e2e dispatch,
  exact argv lists and controlled environment, stripping update/filter/compiler-wrapper overrides. Docs runs Python
  and examples doctest/docs/skill tests; E2E is unfiltered. Test policy/schema and environment handling with injected
  command runners, command parity against the old workflow, and a bad PG URL failing rather than silently skipping.
  Preserve existing shell error propagation. Run the targeted Python tests; do not invoke real publication tools.
- [x] 2. Implement run/status and tree-bound records: owned registered detached worktree with admin marker, cache
  boundary, external-config refusal, before/after source checks, inheritable flock/process groups, abandoned marker,
  atomic running/pass/fail records, redacted metadata and exact policy/runner validation. Implement conservative
  cumulative-diff classification and direct-base selection with no chains, per-kind failure blocking and E2E reuse.
  Test temporary repos, foreign paths, source mutation, symlink/mode/rename/non-Markdown inputs, 100-ancestor limit,
  tampered/missing records, forced failure and killed parent/live child. Confirm lock inheritance and recovery on the
  supported macOS/Linux platforms where available; record limits rather than bypassing unsupported flock. Recovery
  instructions must retain unusable evidence and require identified processes to be gone before marker removal.
  Run targeted Python tests; no destructive operation may touch the main/docs/release-local worktrees.
- [x] 3. Implement read-only pre-push validation and non-clobbering installation respecting core.hooksPath. Parse
  all ref updates, require main tip evidence/fast-forward, reject main deletion/unknown old objects, and require new
  version-tag checks/E2E on previously accepted named-remote main. Never qualify a tag from a proposed main update.
  Reject every existing remote version-tag mutation; preserve unrelated hooks and refs. Test real pushes to local
  bare remotes: missing/pass records, HEAD:main, force/deletion, unknown tracking, tag objects, multiple refs in both
  orders, and a server update hook rejecting only main. Prove the last candidate cannot leave a tag outside main.
  Test foreign-hook refusal and coexistence with docs hooks. Install only in temporary repos during building.
- [x] 4. Implement pinned resumable release and exact-commit release-check preflight/full modes. Hold the common
  lock without recursive acquisition; persist C/O, push explicit objects without force, accept main already containing
  C, and revalidate ancestry/remote state. Generate exact notes, verify skill SOURCE/hash before tag push, then inspect,
  create/resume and publish only a matching draft in origin's GitHub repository. Never clobber conflicting drafts or
  alter published Releases. Test with local bare remotes and fake gh/command adapters: dirty caller files untouched,
  HEAD/main races, changed C, raw annotated/lightweight mismatches, package failure before tag push, failures after
  each public step, uncertain API results, matching/mismatched drafts, missing/extra assets and published no-op.
  Verify all side effects remain bound to C and all waits/errors keep truthful recovery state. No real gh writes.
- [x] 5. Integrate manual-only CI using the shared policy, delete automatic release.yml, and update build briefs
  for committed-tree evidence, classified release/archive edits, ordinary owner approval and first-use rollout.
  Reconcile ADR 0046/index and mark only ADR 0016's remote-publication and failed-tag-deletion clauses superseded.
  Write the docs handoff for AGENTS.md, release and maintenance guides: commands, coverage, supported environment,
  immutable remote tags, explicit abandoned-marker recovery, non-hermetic/local enforcement limits and manual Linux
  coverage. Name no known blocking doc/route changes; recheck links. No claim that guidance is already written.
  Add the normal patch version/changelog/skill metadata and generated version updates at build time. Verify workflow
  triggers/command parity, Python tests, docs and skill tests, and queue status. The actual shared hook install and
  first local release belong to the owner's approved Land step, not this builder task; no interim bypass.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` with PostgreSQL and `openspec validate local-push-gate --strict`, using the new full gate
  on committed source so those checks and all preserved CI checks produce valid evidence. Run the actual unfiltered
  Docker E2E gate; skipped PG/E2E is not completion. Recheck evidence for the final committed tree after task/review
  edits. Confirm all temporary fault hooks and fake publication commands stay test-only, no secrets enter records,
  and no shared hook, origin ref, real Release or unrelated worktree was modified by the builder. Record unavailable
  checks and remain blocked rather than publishing unchecked. Follow normal code review and owner-approved Land.
