---
id: inspector-live.code.3
kind: code
change: inspector-live
round: 3
base: 7818622f21ef549122f6e01ab2b2331fe0aae0ee
head: 5266fc2f957fe065a8652f928068aac0137e032d
built-by: claude-builder
previous: 52c4ed3bef16ce94bb91b9f0fbb992c37dbcd49e
requested: 2026-11-27T08:56:18+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-live.code.3 <file>`.

## Note from the requester

Built by Claude; round 2's four fixes by the same builder, head 5266fc2f: Types shows the refreshed type, menu and JSON
together only while the refresh is current (one existing test's expectation changed for that); a pause counter whose
gate only the live task reopens; a visit counter per page switch; the ⌘R wait covering the whole reload and dropped on
a page switch. Inspector 156 tests (3 runs), workspace with PostgreSQL 4,041, clippy -D warnings, fmt, docs tests,
openspec. Not run: gate, E2E, AppKit.
