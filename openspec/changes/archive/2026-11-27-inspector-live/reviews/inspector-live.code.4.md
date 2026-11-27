---
id: inspector-live.code.4
kind: code
change: inspector-live
round: 4
base: 7818622f21ef549122f6e01ab2b2331fe0aae0ee
head: ce35363ff648b568375c85a35546089ab92f4b89
built-by: claude-builder
previous: 5266fc2f957fe065a8652f928068aac0137e032d
requested: 2026-11-27T09:06:24+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-live.code.4 <file>`.

## Note from the requester

Built by Claude; round 3's two fixes by the same builder, head ce35363f: a context switch counts as a new visit, taken
under the live refresh check's lock before the old reads are cancelled; a 404 actor lookup cancels the pane's other
reads. Also the Outcomes count wait checks whether the task finished. A side effect fixed: the pane's slot requests made
`biased` so `actor_refresh_shares_four_slots_…` keeps its order. Inspector 158 tests (6 runs), clippy, fmt, docs tests,
openspec. Not run this round: the workspace tests, gate, E2E, AppKit.
