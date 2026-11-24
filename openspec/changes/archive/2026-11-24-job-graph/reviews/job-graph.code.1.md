---
id: job-graph.code.1
kind: code
change: job-graph
round: 1
base: e1fffcffc0ab39fe29b3f1cb6e95567cc9842b1b
head: d59e6213cbfef1cc92598efe2e6d5759bc689ba6
requested: 2026-11-24T14:10:53+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done job-graph.code.1 <file>`.

## Note from the requester

Version: 0.20.8 (the unreleased batch; no bump)

Builder report (head d59e621): tasks 1-6 done. fmt, clippy -D warnings clean; cargo test --workspace 3274 passed (PostgreSQL in the gate); --test job_graph 19; lib unit 122; wire_formats 28; docs 11; skill 18; openspec strict valid; gate.py full pass of d59e621. E2E at release. No departures. Names chosen: Job::new(...).with_needs, JobEntry, JobAttempt, JobOutcome, JobGraphStatus, InvalidJobGraph, JobReport (names the last attempt by number).
Note: the committed design has run status `running | succeeded | failed | cancelled` (cancelled set only by job-graph-cancel).
Docs: handed off, nothing blocks.
