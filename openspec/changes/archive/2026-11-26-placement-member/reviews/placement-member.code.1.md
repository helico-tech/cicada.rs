---
id: placement-member.code.1
kind: code
change: placement-member
round: 1
base: 4b265079b67c852c324cc560604f97bad81a48c2
head: 1eb972ad051b0bad861c1c7ea3501547b671738f
requested: 2026-11-26T14:22:50+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-member.code.1 <file>`.

## Note from the requester

Version: 0.24.0 (the placement line)

Builder report (head 1eb972ad): tasks 1-3 done; task 4's checks passed except the gate (its run was killed by the
builder's own loop limit; the orchestrator gates main after the merge). Checks: fmt, clippy -D warnings, openspec
--strict; workspace 3698 (PostgreSQL tests skipped); examples docs 11 / doc 50; the 7 new member tests passed 3 runs in a
row. Choices: code in placement/member.rs; zero refused by MemberSettings::with_interval (InvalidPlacement);
InvalidIdentity also covers write failures; cicada-actors gains tracing and uuid; the contract reference changed only
in the description of `instance`. Docs: handed off, nothing blocks. Do not block on the missing gate evidence.
