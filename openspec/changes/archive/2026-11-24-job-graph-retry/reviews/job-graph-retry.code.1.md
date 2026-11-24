---
id: job-graph-retry.code.1
kind: code
change: job-graph-retry
round: 1
base: 702cdb63b5511e73d2c8e04af620ce4d25c2ddea
head: 1437b0adb727221c49c6b375126cfedf509d5dd2
requested: 2026-11-24T16:16:55+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done job-graph-retry.code.1 <file>`.

## Note from the requester

Version: 0.20.8 (the unreleased batch)

Builder report (head 1437b0a): tasks 1-6 done. fmt, clippy clean; cargo test --workspace with PostgreSQL 3317 passed (job_graph 54, wire_formats 28); docs 11; skill 18; openspec strict valid; gate.py full pass of 1437b0a. E2E at release. Retry checks the job, then reopen(i) and the existing settle. 15 new tests.
Departures: the run's contract file has Retry and `ends` but not `end` (JobGraphEnd is declared in the host's family; its schema and samples are checked in wire_formats.rs). "Retry before the cancel notice ran" is set up by (deliverAt, seq) ordering (G5). JobGraphEnd.end and JobGraphState.ends are new public fields (unreleased; ship in 0.20.8).
Docs: handed off, nothing blocks.
