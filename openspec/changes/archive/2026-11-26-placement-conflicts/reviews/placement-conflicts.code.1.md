---
id: placement-conflicts.code.1
kind: code
change: placement-conflicts
round: 1
base: 5f0277af5bf7de8d91b416013b4a18073150e7fe
head: 505dc19f2af38031cb6395c5591d77f4cac55d8d
requested: 2026-11-26T12:09:53+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-conflicts.code.1 <file>`.

## Note from the requester

Version: 0.24.0 (the placement line)

Builder report (head 505dc19f, on main 5f0277af): tasks 1-4 done. Checks: workspace with PostgreSQL 3688 (placement 44,
4 new; a decode unit test; wire samples), examples docs 11 / doc 50, fmt, clippy -D warnings, openspec --strict; gate
full pass of 505dc19f. Departures: extra decode checks (a mark's instance out of bounds, equal to the recorded one, or an
`at` too late to add a silence time); the renewal test now renews with the same instance (a second instance is a
conflict now); placement doc paragraphs over 120 columns rewrapped and the contract reference regenerated. Until
placement-member, a restart within the silence time looks like a conflict. Docs: handed off, nothing blocks.
