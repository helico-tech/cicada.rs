---
id: container-make-dirs.code.1
kind: code
change: container-make-dirs
round: 1
base: 9e220888599dede46a8352081a47b4c2493dcb60
head: 0ffdbfe3649577efb75c6d0b547411f0224e43a7
requested: 2026-11-24T19:09:00+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-make-dirs.code.1 <file>`.

## Note from the requester

Version: 0.20.9 (becomes 0.21.0 later in the batch)

Builder report (head 0ffdbfe): tasks 1-5 done. cicada-containers --all-features 128+22+6; cargo test --workspace with PostgreSQL 3391; fmt, clippy, openspec strict, docs 11; gate.py run --e2e full pass and E2E 58 passed on 0ffdbfe.
Departure, accepted by the change loop: the walk stats one part at a time from the root instead of the folder then upward (design decision 4): walking upward finds an existing /var/run/postgresql without seeing that /var/run is a link, so the tar would pass through a link, which decision 5 forbids; going down, every statted path is link-free. Cost one stat per part; Docker is never asked about a path below a file. Shared module src/dirs.rs (fake and Docker); paths::place_as for the error text.
Docs: handed off, nothing blocks.
