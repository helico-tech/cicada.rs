---
id: cicada-agent-skill.code.2
kind: code
change: cicada-agent-skill
round: 2
base: 30c20b48cd2d47fa2eeb1f65eb90e079b77723f9
head: 7a25c1076d4fed685c043d8b28268d16a039035f
previous: e83f62b22d5580edb708fc75c7e4a05547f459ed
requested: 2026-11-14T10:06:16+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done cicada-agent-skill.code.2 <file>`.

## Note from the requester

Version: 0.17.2

Round 2. Since round 1 the owner gave the README and hand-written docs to a dedicated docs session (AGENTS.md "Documentation ownership", docs/maintaining/maintenance.md; a local hook refuses other worktrees' commits of them) and main's docs were rebuilt around tasks. Owner decision: split. This branch keeps the code; the prose goes to the docs session through openspec/changes/cicada-agent-skill/docs-handoff.md.

The branch is rebuilt as one commit (7a25c10) on main 30c20b4; the round 1 head is no longer an ancestor, so review it whole against main (git diff main...feat/cicada-agent-skill).

Round 1 findings:
1. False exact match on a failed git probe (skills/cicada/scripts/cicada_source.py): fixed. revision_match is same/different only when git read both HEAD and the status; missing git or a failing HEAD/status read gives revision_match "unknown" and modified null (unversioned null when git is missing). SKILL.md says unknown/null is not proven and never a match. Tests for git missing from PATH and a fake git failing HEAD, then status.
2. Timeout wording in docs/guide/core.md: no longer on this branch (prose belongs to the docs session). The corrected text (crash or lost lease runs again; a timeout only under a retry policy covering it; blocking overtime never) is in docs-handoff.md.
3. README install fails without .claude/skills: no longer on this branch. The fixed steps (mktemp -d staging, mkdir -p .claude/skills, cp -R into it; update: diff -r, rm -rf the old folder, copy) are in docs-handoff.md, checked as documented in an empty project for a first install and an update.

Other changes from the split:
- The branch no longer edits README.md, docs/guide/*, docs/operations.md. AGENTS.md (test command, skills/cicada and package-skill.py map lines, skill rule) and CHANGELOG.md are kept; the hook does not protect them. The changelog entry has no links to sections not written yet.
- SKILL.md routes only to files and headings on the new main: actor design to docs/guide/core.md#design-an-actor; new routes to docs/how-to/install.md, docs/how-to/upgrading.md, docs/reference/stores.md, the HTTP guide's "Errors and limits", operations' "Shutdown and probes", "Streams and proxies", "Destructive controls".
- examples/tests/skill.rs coverage is stricter: a route to every .md under docs/guide, docs/how-to and docs/reference, except docs/how-to/releasing.md (releasing Cicada itself); no consumer route into docs/maintaining/ or releasing.md.
- tasks.md notes that the prose of tasks 1 and 5 went to the docs session.

Checks: fmt --check and clippy -D warnings pass; cargo test --workspace with PostgreSQL 2710 passed, 0 failed (lower than 2748 because main's docs rebuild cut README/guide Rust blocks from 49 to 12, so fewer doctests); the skill's Python suites and main's test_docs_maintenance.py pass; openspec validate cicada-agent-skill --strict passes.

Please check the round 1 finding 1 fix, the split (nothing of the agreed behaviour lost; the handoff is complete and correct, including fixes 2 and 3), and the new routes and coverage check.
