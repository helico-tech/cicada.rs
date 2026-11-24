---
id: container-logs.code.1
kind: code
change: container-logs
round: 1
base: bfc56660e50b93be4d71805c198a9b8b3d7b3558
head: bbfdfef28348b1cf31fcc2ebfd0c3a77edf68155
requested: 2026-11-24T17:45:40+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-logs.code.1 <file>`.

## Note from the requester

Version: 0.20.9

Builder report (head bbfdfef): tasks 1-5 done. cicada-containers --all-features 112 passed; cargo test --workspace with PostgreSQL 3347; fmt, clippy, openspec strict, docs 11 pass; per-feature builds ok; ContainerDockerTest 19/19 incl. 4 new steps; gate.py run --e2e full and e2e pass of bbfdfef.
Departures: LogCursor::from_str accepts only the exact Display text; LogStream is an enum (no constructors); ErrorKind::Conflict and InvalidSpec rustdoc name logs_since. The fake's set_logs stores Out lines at the Unix epoch (logs returns the same text).
Docs: handed off, nothing blocks.
