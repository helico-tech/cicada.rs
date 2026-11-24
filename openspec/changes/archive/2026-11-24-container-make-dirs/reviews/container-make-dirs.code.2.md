---
id: container-make-dirs.code.2
kind: code
change: container-make-dirs
round: 2
base: 9e220888599dede46a8352081a47b4c2493dcb60
head: 4ddb33043951c1eb3c77427b94d5d5cf4e94bb8c
previous: 0ffdbfe3649577efb75c6d0b547411f0224e43a7
requested: 2026-11-24T19:19:59+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-make-dirs.code.2 <file>`.

## Note from the requester

Version: 0.20.9

Round 2 (head 4ddb330): fix (a) of round 1 — docs only. make_dirs and DockerRuntime rustdoc say the walk follows at most 8 links in the walk (a guard), Docker resolves a chain itself (one link) and a loop fails with Other from Docker; MAX_LINKS doc and one test name; the Docker spec delta's link rule moved into its own requirement "Links on Docker are followed to a folder" (500-char limit); handoff and changelog match. Checks: fmt; cicada-containers 128+22+6; clippy; openspec strict; docs 11; gate.py full pass of 4ddb330. E2E evidence is from 0ffdbfe (only comments changed since).
