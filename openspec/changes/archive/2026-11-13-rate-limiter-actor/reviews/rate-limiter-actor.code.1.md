---
id: rate-limiter-actor.code.1
kind: code
change: rate-limiter-actor
round: 1
base: df5ece01ec8c74034c4375be02bb3f966ad3acd1
head: b97719c27e3496bff281a2bc1ceec3fe5eb4aced
requested: 2026-11-13T10:00:47+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done rate-limiter-actor.code.1 <file>`.

## Note from the requester

Version: 0.16.2

Builder report (round 1)

rate-limiter-actor (0.16.2) is built on branch feat/rate-limiter-actor, worktree .claude/worktrees/agent-ae3d9760f74379f26. All 5 tasks done, in 5 commits on top of main. Nothing pushed, merged, tagged or archived.

Tasks
1. Done. Ref, checked state, Submit with its typed helper, and the slot arithmetic, in crates/cicada-actors/src/rate_limiter.rs.
2. Done. 20 tests in tests/rate_limiter.rs on virtual time, plus one store-failure test in tests/store_failure.rs.
3. Done. Contract written to docs/reference/contracts/cicada.rate-limiter.json; sample service hosts the type with no startup instance.
4. Done. Example examples/src/rate_limit.rs with its tests, a "Rate limiter" section in the actors guide, tables, README, architecture row, AGENTS map, changelog and version.
5. Done. Finishing checks pass.

Checks
- cargo fmt --all --check: clean.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2108 passed, 0 failed.
- openspec validate rate-limiter-actor --strict: valid.
- Not run: the Docker end-to-end suite (CICADA_E2E=1).

Beyond the design, and open questions
- Added a sample-service state sample and a 62/63-level payload depth test, outside the listed tasks, following the scheduler's pattern.
- The commit-time clamp has no test of its own; the guide states the limit, as the tasks said.
- No open questions.

New concepts
- Slot (GCRA): each submission gets the next free time in the rate; the state keeps one timestamp, idleAt, when the full burst is free again.
- Burst: how many messages may be due at once after idle time, the first one included.
- Longest delay (maxDelay): a submission whose slot is further away than this is refused for good and reserves nothing.
- Due times, not runs: messages that wait past their slots can still run close together.

Breaking changes: none.
