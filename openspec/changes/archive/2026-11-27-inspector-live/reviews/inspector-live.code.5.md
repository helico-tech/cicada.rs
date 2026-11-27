---
id: inspector-live.code.5
kind: code
change: inspector-live
round: 5
base: 7818622f21ef549122f6e01ab2b2331fe0aae0ee
head: 3a20bafc1d260664f014ed34d8df0787f6b601cb
built-by: claude-builder
previous: ce35363ff648b568375c85a35546089ab92f4b89
requested: 2026-11-27T09:15:41+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-live.code.5 <file>`.

## Note from the requester

Built by Claude; round 4's fix by the same builder, head 3a20bafc: a context switch is one Switch command carrying
source, page, prefix and visit (Live::follow; Session::live_target under the switch's locks); a two-server test on real
/changes streams. Notes: the gone-actor test holds the 404 until the mailbox arrived; a count task clears its own slot
by task id and wakes waiters (no test). Inspector 160 (4 runs), workspace with PostgreSQL 4,045, clippy, fmt, openspec.
