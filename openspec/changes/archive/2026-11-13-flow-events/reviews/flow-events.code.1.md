---
id: flow-events.code.1
kind: code
change: flow-events
round: 1
base: 6e572af0f40e67fef725681248466d58dc40e306
head: b63b9293c14b1a564513a571f7badefc2a8a141d
requested: 2026-11-13T15:12:47+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-events.code.1 <file>`.

## Note from the requester

Version: 0.16.7

Builder report (round 1)

All five tasks of flow-events are done on branch feat/flow-events (5 commits); every finishing check passes.

Tasks
1. Done: EventActor (cicada.event) in crates/cicada-actors/src/event.rs, with the ref, state, messages and the two helpers.
2. Done: 23 tests in tests/event.rs, plus one store-outage test on SQLite in tests/store_failure.rs.
3. Done: contract in docs/reference/contracts/cicada.event.json. The sample service hosts the type, with its snapshot, text and a state sample. No existing contract changed.
4. Done: section "Event" in docs/guide/actors.md with 2 doctests, and example 14 examples/src/flow_events.rs with 5 tests. Tables, README, architecture, AGENTS map, changelog and OpenAPI updated; version 0.16.7.
   Addition asked by the build loop (note 1 of the flow-sleep review): in "Durable sleep", "at once" now applies only to a zero sleep, and the text says a positive sub-microsecond sleep rounds up to one microsecond.
5. Done: results recorded in tasks.md.

Checks
- fmt --check: clean.
- clippy -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2298 passed, 0 failed.
- openspec validate --strict: passes.
- Not run: the Docker E2E suite.

Departures and open questions
- The state schema is stricter than the design asked: each name must match the type-name pattern, and names must be unique. The message schemas stay loose, so an invalid Subscribe sent over HTTP fails in its turn, as the spec says.
- Open question: a batch item 62 levels deep fails the publication, but dead letters cannot keep a payload that deep. The guide says so instead of claiming dead letters can recover it. The batcher has the same limit today. (The build loop takes this to the owner with the core depth gap; it is not for this change.)

New concepts
- Event: an actor that keeps up to 100 flow types. Each publication creates and starts one run of each type in one commit. One bad type starts no run.
- Publication id: the message id becomes the key of every run (orders.receipt/<id>). A retry or redrive sends a new id, so it starts new runs.
- PublishBatch: a publication whose input is {"items": [...]}. A batcher whose message type is PublishBatch feeds an event directly.

Breaking changes: none.
