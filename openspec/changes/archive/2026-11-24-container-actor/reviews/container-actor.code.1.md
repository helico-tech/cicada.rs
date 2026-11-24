---
id: container-actor.code.1
kind: code
change: container-actor
round: 1
base: 4b6093669b02371c52cf0902197aa6b7ad39d65c
head: 9e531e3b1ffa39231b3f70d714e113b7a4245eb9
requested: 2026-11-24T12:07:29+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-actor.code.1 <file>`.

## Note from the requester

Version: 0.20.7 (the unreleased batch; no bump)

Builder report (head 9e531e3): tasks 1-6 done. cargo test --workspace 3199 passed (PostgreSQL skipped; the gate ran them); cicada-containers 35 unit + 19 actor + 6 doc tests; skill 18; resolver Python 27; docs 11; fmt, clippy -D warnings (also --features docker), openspec strict pass; gate.py full pass for 9e531e3. E2E not run (release).
Choices where the design is silent: the end message uses the poll's retry policy (7 attempts, 1-32 s); a reason keeps its first 1 KiB; a Start after the end sends the saved end to that Start's own target every time, and a later successful send clears `undelivered`; test helpers from cicada-axum as a dev-dependency only.
Docs: handed off, nothing blocks.
