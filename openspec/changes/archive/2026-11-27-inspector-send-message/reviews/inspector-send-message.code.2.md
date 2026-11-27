---
id: inspector-send-message.code.2
kind: code
change: inspector-send-message
round: 2
base: 7f46c47c21da47a67a2d474682f392e736a46ce3
head: c23a4e07d0488cd1cf62e1153cab749b455f2d8b
built-by: claude-builder
previous: 4ae222ca23b68c4f5231f0c961f28365c17856e7
requested: 2026-11-27T10:40:59+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-send-message.code.2 <file>`.

## Note from the requester

Built by Claude; round 1's seven fixes by the same builder, head c23a4e07: one check thread per window shared by its
sheets with closed sheets' jobs skipped (schema cache keyed by sheet and type); the draft frozen while sending; the
target checked again at dispatch; a fresh id on every type change; a type change waits for an edit's debounce; Cancel
during a send holds the slot until the send ends or times out; Send hidden unless a message type list is ready.
Inspector 197 (send tests 3 runs), clippy, fmt. Not run this round: workspace, docs tests, gate, E2E, AppKit.
