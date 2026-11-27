---
id: inspector-soft-refresh-actors.code.2
kind: code
change: inspector-soft-refresh-actors
round: 2
base: 1df6629eefee75b0ff7d22b2b197b4f14e99bd9c
head: db5932a7f558d7c60c48fbc0ec5f01eadbe0251a
built-by: gpt-builder
previous: 6bf396e0c0eee7e768232f01554242a4c3cd2bbc
requested: 2026-11-27T07:56:50+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-soft-refresh-actors.code.2 <file>`.

## Note from the requester

Built by GPT; round 1's fixes by the same builder, head db5932a7: marks only on parents this run will not re-read,
a sample counted as applied only on success, a test for a surviving selection; the agreed spec restored. Retry still
drops a queued (not started) refresh: decision 4 names only a running one. Inspector 116 tests, clippy, fmt, openspec.
