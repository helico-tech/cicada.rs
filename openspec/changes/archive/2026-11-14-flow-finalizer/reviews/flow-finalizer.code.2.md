---
id: flow-finalizer.code.2
kind: code
change: flow-finalizer
round: 2
base: 58c01805ba276dd6ef5918016b5b530cf3e9738d
head: 12bfe7095c78d3fcc4666adb2f975e683ef3fcf1
previous: 758ccaf2c01984e0534e7042f980717a3b9c3d24
requested: 2026-11-14T19:28:53+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-finalizer.code.2 <file>`.

## Note from the requester

Version: 0.18.5

Round 2: fixes only the round 1 blocker (commit 12bfe70, range 758ccaf..12bfe70). Round 1's report is .reviews/flow-finalizer.code.1.result.md.

Builder's report: `Containers::remove` now records the key as removed (a tombstone) before it returns; a create of that key already accepted but not finished, and any later create, makes nothing, so Ok from remove means no container of that key exists or can appear. The `create` doc says a create can be accepted, return an error such as a timeout, and still finish later. Module docs ("Both owners fence late work") and the SandboxMsg::Stop doc say the stopped flag only blocks later Starts, while in-flight creates are blocked by the runtime's tombstone, like Capacity::release. The docs handoff's cleanup guidance says the owner records the tombstone before it confirms. The test fake keeps removed keys and can accept a create that times out and completes later (`complete_creates()`). New test `a_create_in_flight_when_the_sandbox_stops_creates_nothing_when_it_completes_late`; it fails with the fake's tombstone line removed. The late-Start test is unchanged. No library code changed.

Checks: fmt --check and clippy -D warnings clean; cargo test -p cicada-examples 135 passed, cargo test -p cicada-actors 479 passed, 0 failed. The full workspace with PostgreSQL was not re-run for this round (only the example and the handoff changed); the release checks run it.
