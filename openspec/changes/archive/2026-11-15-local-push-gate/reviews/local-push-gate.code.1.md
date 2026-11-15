---
id: local-push-gate.code.1
kind: code
change: local-push-gate
round: 1
base: 48959e3d979534985bb13e255871b66d13aed730
head: 18ffa1bf25177b7e5bf42bc64f93c783b697001e
requested: 2026-11-15T09:20:24+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done local-push-gate.code.1 <file>`.

## Note from the requester

Version: 0.18.9 (process and tooling; no library code)

Build-loop safety rules for this build: the hook was never installed in the shared repository and no evidence was written to the real common git dir (verified by the build loop: core.hooksPath unset, only the docs hooks pre-commit/pre-merge-commit present, no records); nothing pushed, tagged or released. All hook, push and release tests use temporary repositories, bare remotes and a fake gh.

Builder's report:

Tasks 1–6 done. scripts/gate.py with a versioned full/docs/e2e policy (scripts/gate-policy.json), tree-bound evidence from an owned worktree, the pre-push hook, release-check pinned to the checked-out commit with a --before-push mode and a resumable release driver; ADR 0046 supersedes ADR 0016's workflow and tag-deletion clauses; build and review briefs updated; external Cargo/toolchain configuration refused before release-check runs in the owned worktree.

Checks: full gate (`gate.py run --e2e` on 58290e4, in a temporary clone): 60 Python tests and 3055 Rust tests passed, 0 failed, PostgreSQL included; every feature-alone build and dependency-tree check passed; 28 E2E tests passed in Docker. Final head 18ffa1b changes only Markdown: docs checks fresh over that full pass (60 Python, 47 Rust passed), E2E pass reused. Run directly: fmt, clippy -D warnings clean, --test docs/--test skill 19 passed, openspec validate --strict valid. With a wrong CICADA_PG_URL the store tests fail rather than skip (2 failed). Not checked: Linux (macOS only).

Installation after landing (needs the owner's yes): `python3 scripts/gate.py install-hook` once in the main checkout; releases then run `python3 scripts/gate.py release vX.Y.Z`.

Open points from the builder: (1) the review loop's `.taken` files are not Markdown, so every release commit that archives reviews needs a full gate run rather than the cheaper docs checks; (2) uploading a missing asset to an existing draft calls `gh release upload`; not confirmed against real GitHub that it finds drafts (only matters on resume; normal creation attaches the file in the same call).

Breaking: none for library users. For maintainers: CI no longer runs on push, and a version tag on GitHub can no longer be deleted or moved through the gate.
Docs: handed off, no blockers (AGENTS.md, docs/how-to/releasing.md, a gate section, docs/maintaining/maintenance.md).
