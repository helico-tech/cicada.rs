---
id: container-registry-auth.code.1
kind: code
change: container-registry-auth
round: 1
base: e39fd91222da62d8b98a3fcb4b101672a2812542
head: 944c2d7f4b59dd412047cc02d131eb337a68b7bf
requested: 2026-11-25T09:54:30+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-registry-auth.code.1 <file>`.

## Note from the requester

Version: 0.21.1

Builder report: tasks 1-4 done on feat/container-registry-auth (head 944c2d7f). Task 5: fmt, clippy, workspace tests with
PostgreSQL (3437 passed), cicada-containers all features (200), docs tests (11), cargo deny and openspec validate --strict
pass; full E2E suite run directly: 60 passed on OrbStack (the test registry was reachable on localhost). gate.py run failed
twice on the unrelated a_node_warns_at_start_when_its_pool_is_smaller_than_its_concurrency_plus_4, which fails 3 of 3 runs
on main e39fd912 with the gate's profile; the change pool-warning-test-race fixes it and lands first, then this branch is
rebased and gated. Do not block on the missing gate evidence. Departures: an uppercase first part of a reference counts
as a registry host (as Docker does); a helper killed by a signal gives the reason "exited with a signal". Docs: handed off,
nothing blocks.
