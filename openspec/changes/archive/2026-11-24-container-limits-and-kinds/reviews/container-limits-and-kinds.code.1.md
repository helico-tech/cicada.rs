---
id: container-limits-and-kinds.code.1
kind: code
change: container-limits-and-kinds
round: 1
base: e06059d39cb0d5799540ed2daa7a7db1a0117b0c
head: fd3fbc792e2f2f37d5ee8cca92b674a4c8e44800
requested: 2026-11-24T20:28:20+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-limits-and-kinds.code.1 <file>`.

## Note from the requester

Version: 0.21.0 (BREAKING; the batch heading renamed from 0.20.9 with a rewritten summary)

Builder report (head fd3fbc79): tasks 1-5 done. cargo test --workspace with PostgreSQL 3416; cicada-containers 134 unit, 39 actor; docs 11; skill 18; clippy, fmt, openspec strict clean; gate.py full pass of fd3fbc79. E2E at release.
Departures: spec deltas changed from Other to Unsupported for the trait defaults (design's post-review section) plus runtime deltas for the error kinds and the three default methods; the scenario "A container gone after create" rewritten for the new inspect-first order; the unreleased container-inputs changelog entries corrected (Unsupported, the old inspect after create).
Docs: handed off, nothing blocks.
