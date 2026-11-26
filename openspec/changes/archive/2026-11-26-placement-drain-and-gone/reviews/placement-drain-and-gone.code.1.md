---
id: placement-drain-and-gone.code.1
kind: code
change: placement-drain-and-gone
round: 1
base: 86a0546200c1a3032f39311899bb857d9ada571e
head: 0e6c440591ad266c0787fe7cdbb95ea4a9974740
requested: 2026-11-26T10:26:48+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-drain-and-gone.code.1 <file>`.

## Note from the requester

Version: 0.24.0 (the placement line)

Builder report (head 0e6c4405, on main 86a05462): tasks 1-4 done. Checks: cicada-actors (placement 40 tests, 7 new;
wire 31), workspace 3683, examples docs 11 / doc 50, fmt, clippy -D warnings, openspec --strict; gate full pass of
0e6c4405. No departures; `draining` is optional when decoded (false) and always written. PlacementAnswerKind gains Gone
(exhaustive host matches need an arm; the handoff says so). Docs: handed off, nothing blocks.
