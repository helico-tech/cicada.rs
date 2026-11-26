---
id: placement-asks.code.1
kind: code
change: placement-asks
round: 1
base: cff593c4eb493593df5f1aad5bf10ce3a9279410
head: 2278afedea75b3809e0a84111e1b0fde681ccefb
requested: 2026-11-26T07:51:06+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-asks.code.1 <file>`.

## Note from the requester

Version: 0.24.0 (the placement line)

Builder report (head 2278afed, on main cff593c4): tasks 1-5 done. Docs: owner fixes integrated: docs/guide/actors.md
from the docs session's commit 139b0857 (byte-equal), folded into the commit that changes PlacementState::new (now four
arguments). Skill route added (actors.md#placement-pool, cicada.placement.json). Checks: examples doc 50 / docs 11 /
skill 18; workspace with PostgreSQL 3667; fmt, clippy -D warnings, openspec --strict; gate full pass of 2278afed.
Gap-fills to judge: poolFull = a place fits but the pool holds maxHolders and the line is full (lineFull when no place
fits and the line is full); holders are one top-level list naming their node; a Notify from anyone but the pool is
refused for good (the permit pool ignores it); maxWaiting and maxHolders required at create; one test dropped (a failed
Notify of a holder already told frees nothing: the helpers cannot order it). Prose still to write after landing:
architecture's cicada-actors row (handoff section 2).
