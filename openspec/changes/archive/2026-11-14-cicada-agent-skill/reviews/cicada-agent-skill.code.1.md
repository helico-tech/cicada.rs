---
id: cicada-agent-skill.code.1
kind: code
change: cicada-agent-skill
round: 1
base: c35eb588c186d51d223461b662954180d6d9eb91
head: e83f62b22d5580edb708fc75c7e4a05547f459ed
requested: 2026-11-14T09:40:28+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done cicada-agent-skill.code.1 <file>`.

## Note from the requester

Version: 0.17.2

Builder's report:

Tasks (all done):
1. Core guide: new section "Design an actor type" (actor boundary, stable keys, small state, messages, idempotency keys for outside calls, durable waits, why reading old state is not an equal contract); no new Rust blocks. Added the two examples missing from the guide's table; "fifteen" -> "seventeen" there and in the README.
2. skills/cicada/SKILL.md (127 lines) and its read-only helper skills/cicada/scripts/cicada_source.py.
3. examples/tests/skill.rs, the Python tests and evals.json in examples/tests/skill/. The Markdown reading code of docs.rs moved to examples/tests/markdown/mod.rs, shared by both checks. An independent agent ran 3 of the 9 evaluation cases; results in openspec/changes/cicada-agent-skill/evaluation.md.
4. scripts/package-skill.py; release.yml packages the skill in the verify job and attaches the archive to the GitHub Release.
5. README install/update section, Releasing steps in docs/operations.md, a skill version check in release-check.sh, AGENTS.md and the changelog. ADR 0041 unchanged.
6. Finishing checks.

Checks: fmt --check and clippy -D warnings pass; cargo test --workspace with PostgreSQL 2748 passed, 0 failed; openspec validate --strict passes; skill target 8/8 and docs target 11/11 after the last SKILL.md edit; Python: 24 resolver tests and 8 packaging tests pass; a packaged archive was unpacked and installed in three temporary consumer projects outside the checkout. Not run: real GitHub Actions (only YAML parse checked).

Departures / notes:
- Python 3.11+ is now needed to run cargo test --workspace; the skill test fails rather than skips when it is missing.
- OUTPUT of package-skill.py is taken as a folder.
- Fixed five small routing gaps the evaluation found in SKILL.md.
- The core guide has no recipe for testing "the outside call happened, then the node died"; belongs to a later change.

New concepts: consumer skill (routes an agent to the guides and code of the Cicada source the project resolves); source report (helper JSON per source group: version, full Cargo source, commit, local changes; version_relation compares versions, revision_match compares commits; an equal version is not proof of the same code).
Breaking: none.
