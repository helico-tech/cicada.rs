---
id: container-inputs.code.1
kind: code
change: container-inputs
round: 1
base: 3b117ed7043e9ac0646792939a002a3cf7f785eb
head: 3952e93ffc1300bcfee3f2b0ed797c085250dea2
requested: 2026-11-24T19:43:49+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-inputs.code.1 <file>`.

## Note from the requester

Version: 0.20.9 (becomes 0.21.0 with container-limits-and-kinds)

Builder report (head 3952e93f): tasks 1-5 done. fmt, clippy clean; cargo test --workspace 3405 passed; cicada-containers all features 131 unit, 32 actor, 7 doc; docs 11; openspec strict valid; gate.py full pass of 3952e93f. E2E at release. No departures; open choices: a container missing right after its create counts as a Conflict error with its own message; when the second inspect after a Conflict fails, the actor counts the original Conflict. 10 new actor tests.
Docs: handed off, nothing blocks.
