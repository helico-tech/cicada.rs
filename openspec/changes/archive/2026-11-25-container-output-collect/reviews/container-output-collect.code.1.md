---
id: container-output-collect.code.1
kind: code
change: container-output-collect
round: 1
base: 7faf9e7f25e5687d4d99fa2bc3633d81698c75e6
head: 1bd64ade94c2c1566dbb2a542c0242a19619618e
requested: 2026-11-25T22:18:47+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-output-collect.code.1 <file>`.

## Note from the requester

Version: 0.23.0 (BREAKING)

Builder report (head 1bd64ade, rebased onto 7faf9e7f): tasks 1-5 done. Checks: workspace with PostgreSQL 3600; new
tests/outputs.rs 17, tests/actor.rs 59; examples doc 45 / docs 11 / skill 18; fmt, clippy -D warnings, openspec --strict;
gate full pass of 1bd64ade. No design departures. Consequence named in changelog and handoff: owner types whose messages
take ContainerEnded also get a new contract hash (the end gains outputs). Breaking notes: new state pending and phase
collecting; ContainerEnd literal needs outputs: None; exhaustive Phase matches need Collecting; the store refuses names
over 200 and control characters; no rollback to 0.22 while an actor is collecting. Docs: handed off, nothing blocks.
