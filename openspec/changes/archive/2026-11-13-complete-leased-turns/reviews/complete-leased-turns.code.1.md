---
id: complete-leased-turns.code.1
kind: code
change: complete-leased-turns
round: 1
base: 90def0399d1870853a5143af278685408ed64d0c
head: d9fd84d380fc9099859313c97ffd3195f6802947
requested: 2026-11-13T19:11:20+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done complete-leased-turns.code.1 <file>`.

## Note from the requester

Version: 0.16.14

Builder report (round 1)

complete-leased-turns is built on feat/complete-leased-turns, rebased on main (90def03). Two builders worked on it: the first was stopped mid-task 4 when the owner briefly parked remote workers; the second checked its work against the design, finished it and replaced its WIP commit with proper commits. Every task ticked, every finishing check passes. Nothing pushed, merged, tagged or archived.

Tasks
- Tasks 1–3 (code and tests): done in c0060d4 and cda1caa. complete_turn and the shared helpers checked against design.md; no differences.
- Task 4 (docs): done in efb8e2e: an operations note on completion and the 0.16.14 upgrade note, version 0.16.14 and its changelog section; ADR 0038 and its index row needed no change. Addition asked by the build loop (note of the lease-turns-in-core review): docs/architecture.md, docs/guide/core.md and system.rs now say watcher notifications are stored atomically with the commit even when its acknowledgement is lost, and only observers wait for confirmation.
- Task 5 (finishing checks): done in d9fd84d; docs/openapi.json regenerated (version only).

Checks
- cargo fmt --all --check: passed.
- cargo clippy --workspace --all-targets -- -D warnings: passed.
- cargo test --workspace with PostgreSQL: 2570 passed, 0 failed, no PostgreSQL test skipped.
- cargo test -p cicada-examples --doc: 53 passed; --test docs: 11 passed; the cicada crate's doctests passed.
- cargo check -p cicada with no feature, sqlite, postgres and testing: all build.
- openspec validate --strict: valid.
- Not run: the cicada-app feature checks and the Docker E2E tests.

Departures from the design: none; no open questions.

New concepts
- Turn report: a worker tells the server how its handler ended: returned (with a new state and its effects), failed, or cancelled. The server decides the outcome and whether to retry, using the message's own retry policy.
- Rejected: the store turned the commit down and nothing was applied; the answer carries a failure, and from then on the worker sends only that failure, never its original effects again.
- LeaseLost after a retry: no receipt is kept, so repeating a completion that already applied also gets LeaseLost; that answer does not prove the first attempt failed.

Breaking changes: none. ActorCreate, Control, ControlEffect and WatchChange became public, which only adds to the API.
