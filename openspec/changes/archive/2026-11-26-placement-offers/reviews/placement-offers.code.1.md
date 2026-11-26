---
id: placement-offers.code.1
kind: code
change: placement-offers
round: 1
base: ef1b6dc84f125eb1244addf0ce2db849eb8908cc
head: 757adf4fef612c5202c0e393777d132c3b0ec245
requested: 2026-11-26T03:31:03+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-offers.code.1 <file>`.

## Note from the requester

Version: 0.24.0 (the placement line's release; nothing released from it yet)

Builder report (head 757adf4f): tasks 1-5 done. placement.rs with checked state, offers, silence and alerts; contract
docs/reference/contracts/cicada.placement.json, samples, wire tests; a new 0.24.0 unreleased section and 0.24.0
everywhere. Checks: fmt, clippy -D warnings, workspace with PostgreSQL 3646 (9 actor, 4 unit, 2 doc, 3 wire tests new),
openspec --strict; gate full pass of 1b33f1c2 (task 5 ticked after, in 757adf4f). Open choices: an Alert from another
sender is refused for good like Check; armed holds only the check time and a check arms the next only when armed has
passed (one chain when Rearm makes two); cost: a check made due early stops the chain until Rearm (documented); node and
instance ids must not be blank; the skill route to actors.md#placement-pool is left to the docs session (heading missing).
Docs: handed off, nothing blocks.
