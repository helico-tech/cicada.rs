---
id: scheduler-actor.code.1
kind: code
change: scheduler-actor
round: 1
base: 908ecf142bd01f4cfe152a622c044b6de94584e0
head: 7a34667fafcb1e11af1bca5b96dd67744966d1c2
requested: 2026-11-13T09:25:14+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done scheduler-actor.code.1 <file>`.

## Note from the requester

Version: 0.16.1

Builder report (round 1)

All 13 tasks of scheduler-actor are built and committed on feat/scheduler-actor, 5 commits on top of main (908ecf1). The leftover scheduler.rs from an earlier session was kept: it already matched the design; tests and the rest of the change were added around it.

Tasks
- 1.1–1.5 (the actor, the chain, Run and Update, failures, the 62/63-level depth limit): done. A test for every scenario of the spec in crates/cicada-actors/tests/scheduler.rs (33 tests). A store-read failure test for Run in tests/store_failure.rs.
- 2.1 contract, 2.2 sample service (contract snapshot, contract text, state sample tests/states/scheduler/): done.
- 3.1 guide, architecture, AGENTS.md map, README; 3.2 version 0.16.1, CHANGELOG, Cargo.lock, docs/openapi.json: done.
- 4.1, 4.2: done.

Checks
- cargo fmt --all --check and cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2,066 passed, 0 failed.
- cargo check -p cicada-actors: passes.
- openspec validate scheduler-actor --strict passes; openspec validate --specs --strict: 32 passed, 0 failed.
- End-to-end suite in Docker: 1 passed, 0 failed (188 s).

Departures from the design, and questions
- Schedule is now a subsection ("### Schedule") inside the guide's "Scheduler" section, anchor #schedule; the one link in core.md is updated.
- Run and Update are written with explicit null fields ({"type":"Run","slot":null}), because the design gives no rule to leave empty fields out. Clients may still leave the fields out.
- A FailOnce message was added to the shared test worker, to test the retry scenarios.
- openspec/specs/documentation/spec.md line 66 still names only three contract snapshots; left, as outside this change.

New concepts
- Scheduler (cicada.scheduler): one actor per schedule, sends a stored message to a target on each slot; schedule, target, message and retry policy live in its state.
- Slot header cicada-slot on each delivery for a slot; a Run sent by a client has none.
- Started / chain: started when armedTick is set; the chain stops when its armed tick is purged, cancelled or fails for good; Start arms it again.

Breaking changes: none.
