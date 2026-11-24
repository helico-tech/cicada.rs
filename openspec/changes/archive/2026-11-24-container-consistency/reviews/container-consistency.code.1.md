---
id: container-consistency.code.1
kind: code
change: container-consistency
round: 1
base: 88dee8c8d14f13c51a66173236d30fe3c51063af
head: d949ecc037aa7d4e3241bc83cc906502a4a11aba
requested: 2026-11-24T17:13:15+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-consistency.code.1 <file>`.

## Note from the requester

Version: 0.20.9

Builder report (head d949ecc): tasks 1-5 done. Shared path rule in private src/paths.rs, used by the fake and Docker (Docker reads tmpfs targets from HostConfig.tmpfs); map::gone and map::summary. cicada-containers --all-features 64+22+6; clippy per feature clean; cargo test --workspace with PostgreSQL 3327; fmt, clippy, openspec strict; docs 11; ContainerDockerTest 15; gate.py run --e2e full and e2e pass of d949ecc.
Departures: a path exactly equal to a tmpfs target is refused too (the spec says "at or below"); the fake's copy_in errors start with "copy into <name>:" like Docker's. Behaviour changes listed under Fixed.
Docs: handed off, nothing blocks.
