---
id: container-actor.code.2
kind: code
change: container-actor
round: 2
base: 4b6093669b02371c52cf0902197aa6b7ad39d65c
head: 1a073a700331c5a49f6b281ccad5d2451da2eb53
previous: 9e531e3b1ffa39231b3f70d714e113b7a4245eb9
requested: 2026-11-24T12:18:34+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-actor.code.2 <file>`.

## Note from the requester

Version: 0.20.7

Round 2 (head 1a073a7). Fixed: (1) blocking: Start runs the state decoder's message-type check before saving; empty or >200 chars fails the turn for good, state unchanged; test shows a normal Start still works after. (2) note 1: a successful inspect resets errors (running and stopping); test with two failed inspects then a failed log read keeps the real logs. (3) handoff says a Start after the end resends the saved end; target treats repeats per key as one. Contract test.container.json rewritten: description only, hash unchanged. Checks: cicada-containers 35 unit, 21 actor, 6 doc; clippy, fmt clean; gate.py full pass of 1a073a7.
