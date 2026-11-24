---
id: job-graph-cancel.code.1
kind: code
change: job-graph-cancel
round: 1
base: 729502b33885fc602ff981418fb5a7d330c85be1
head: b0e3662ff7c1525ff4ef32081194fa381e3c993d
requested: 2026-11-24T15:39:46+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done job-graph-cancel.code.1 <file>`.

## Note from the requester

Version: 0.20.8 (the unreleased batch)

Builder report (head b0e3662): tasks 1-6 done. fmt, clippy clean; cargo test --workspace with PostgreSQL 3299 passed; job_graph 39; wire_formats 28; docs 11; skill 18; openspec strict valid; gate.py full pass of b0e3662. E2E at release. No departures. Added Job::with_cancel_type; a missing cancelType is written as null (like output); the cancelled-status doc text no longer says "not set by this release", so JobGraphEnd's descriptions changed (fields not); dispatch/report/send_cancel share one send helper. 12 new tests incl. a SendCancel sent by a client.
Docs: handed off, nothing blocks.
