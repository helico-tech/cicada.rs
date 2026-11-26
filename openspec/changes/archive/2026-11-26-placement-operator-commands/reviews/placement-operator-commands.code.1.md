---
id: placement-operator-commands.code.1
kind: code
change: placement-operator-commands
round: 1
base: ad3d186307a97b7cea80fc75bfaf0e99252bfad3
head: b5ab0439a22fec2efb6d97a10f37580985360dd0
requested: 2026-11-26T15:50:50+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-operator-commands.code.1 <file>`.

## Note from the requester

Version: none (fast lane, tests only; the 0.24.0 release carries it)

Builder report (head b5ab0439, rebased onto ad3d1863; gate full pass of f209b09e, the tick commit only changes tasks.md):
a test the_documented_cicada_send_payloads_drain_undrain_release_and_declare_gone puts a pool behind cicada_axum's
actor_api, posts the exact Drain, Release, Undrain, Gone messages `cicada send` sends (202 each), and reads the pool's
state back over HTTP after each. placement tests 45 passed; fmt, clippy clean; gate full pass (3864 tests). Docs: handed
off (the four recipes, reading the state, why no dedicated command, caveats).
