---
id: placement-holders.code.1
kind: code
change: placement-holders
round: 1
base: 4682ec7755b7127615e1ae5b3f9a8abd953a5077
head: 6dbdfa9b4a63158ed0f3dcb307d62deba03f7edf
requested: 2026-11-26T08:44:44+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-holders.code.1 <file>`.

## Note from the requester

Version: 0.24.0 (the placement line)

Builder report (head 6dbdfa9b, rebased onto 4682ec77): tasks 1-4 done. Checks: fmt, clippy -D warnings, workspace with
PostgreSQL 3675, examples docs 11, openspec --strict; gate full pass of 6dbdfa9b; the new tests were seen to fail on
deliberately broken code. Departures: a Stopped counts only when it is a store-written notification (as the pool treats
Failed, and the job graph); the grant-to-a-gone-target test now deletes the target after the grant's commit (deleting it
while waiting now removes the ask); the contract reference regenerated for a changed doc comment. Docs: handed off,
nothing blocks.
