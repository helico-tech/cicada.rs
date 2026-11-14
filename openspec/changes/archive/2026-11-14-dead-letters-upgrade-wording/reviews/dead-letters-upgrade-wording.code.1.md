---
id: dead-letters-upgrade-wording.code.1
kind: code
change: dead-letters-upgrade-wording
round: 1
base: a5980e7ee87b76cc48d778e08d0e3ff7b25476bb
head: ea71d289bc7e48017a7531f52765ff16d0295162
requested: 2026-11-14T20:14:55+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done dead-letters-upgrade-wording.code.1 <file>`.

## Note from the requester

Version: 0.18.7 (text only)

Builder's report:

Tasks 1–4 done. A marked correction note in the archived deep-failure-notices design.md (after the export-conversion paragraph; it also scopes the Proofs wording) and an indented note under its task 3 in tasks.md: added lines only (8 and 3), checkbox count still 6/6 ticked. Only the schema sentence of check's rustdoc in examples/src/dead_letters_upgrade.rs changed; no code. Version 0.18.7 in Cargo.toml, Cargo.lock, skills/cicada/SKILL.md and docs/openapi.json; correction-only changelog entry; docs-handoff.md written.

Checks: dead_letters_upgrade 5 passed; the payload-fields test 1 passed; cargo doc -p cicada-examples --no-deps no warnings; fmt --check and clippy -D warnings clean; openspec validate --strict valid; cargo test --workspace with PostgreSQL 2905 passed, 0 failed. After rebasing onto a docs-only main commit: --doc 28, --test docs 11, --test skill 8 passed. status.py order unchanged; the spec delta has all 6 scenarios.

Departures: none. Docs: handed off, no blockers; one optional suggestion (step 5 of the 0.17.0 upgrade in docs/how-to/upgrading.md lists three of check's four checks).
Breaking: none.
