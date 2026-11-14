## Implementation

- [x] 1. Update `.claude/skills/build-loop/builder.md`: describe the existing ownership boundary and its precedence
  over earlier tasks, require `docs-handoff.md` even for no impact, and keep its template here alone. Include version,
  source/public items and proving tests/examples, destination paths/headings as code spans, proposed substance,
  caveats, blocking checks/replacements or none, and truthful verification/docs status. Update it with behavior-changing
  fixes; review-request head identifies the source. Add the blocked-build report and skill metadata version step.
  Verify the additive/no-impact/fix-round walkthrough and `cargo test -p cicada-examples --test docs`.
- [x] 2. Update `.claude/skills/build-loop/SKILL.md`: record the concrete blocked head/paths, coordinate owner-prepared
  fixes, integrate only agreed exact blobs/deletions without merging, and check current owner-branch identity and
  compatibility with the base. Preserve unrelated prose and cover rename/deletion sides. Fold required fixes with
  breaking source so main has no intermediate known docs break. Keep tasks/checks pending until the combined checks
  pass, then use normal review and owner land/release approval. Recheck after fixes and every late rebase, including
  conflict-free ones; never resolve protected conflicts by blindly choosing a side. Add truthful Docs state in Present,
  archived handoff in Land, and the `docs/how-to/releasing.md` reference. Verify docs links and walk through breaking,
  unavailable-owner, changed-head, clean-rebase and release-pin cases against the existing hook's actual rules.
- [ ] 3. Reconcile the design-loop skill, both review briefs and `openspec/config.yaml`: design identifies blocking
  doc blocks/links/guarantee rows/routes, tasks hand off protected changes while retaining verification obligations,
  and code review checks both handoff and integrated owner content against its head. Explicitly accept walkthrough
  evidence for these process-only scenarios in the scenario-test rule; no prose-mirroring tests. Known failed docs
  checks block normal completed-build review and landing even when listed in the handoff. Verify with the docs test,
  `openspec validate --all --strict`, and a consistency walkthrough across the four briefs and config.
- [ ] 4. Reconcile ADR 0045/index and AGENTS.md pointers with the implemented workflow; write this change's own
  `docs-handoff.md`. Request the docs-owned maintenance procedure for named feature-head preparation, compatibility
  with current main and explicit coordination until adopted. Hand off architecture/maintenance/guide/guarantee prose
  instead of editing it. Keep the owner's queue order. Set the next patch version at build time, including skill
  metadata and required generated versions, and add a process-only changelog entry. Verify unchanged ownership hooks
  with `python3 -m unittest scripts/test_docs_maintenance.py`, plus the docs and skill tests and `python3 scripts/status.py`.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate build-loop-docs-handoff --strict`. Run PostgreSQL checks where configured and record any skips.
  Review the additive, blocking, absent-owner, revised-head (including a conflict-free late rebase) and release-pin
  walkthrough evidence. Confirm no ownership-hook or library behavior changes and no claim that unpublished feature
  discovery or docs-side adoption already exists. Follow normal review and owner-approved release gates.
