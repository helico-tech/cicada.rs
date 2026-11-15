---
id: lease-limit-docs.code.1
kind: code
change: lease-limit-docs
round: 1
base: 26ba63422dd44c4994d2e1d05aefc99adc07ab2d
head: b4537e221436180205347cfe2c11d9bb34448930
requested: 2026-11-15T12:16:52+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done lease-limit-docs.code.1 <file>`.

## Note from the requester

Version: 0.18.11 (doc comments, regenerated OpenAPI, one test)

Builder's report:

Task 1 done: a test in crates/cicada-axum/tests/openapi.rs requires each LeaseTurnsRequest property description to state the constants (`owner` at most MAX_ID_LENGTH characters, `types` 1 to MAX_LEASE_TYPES, `limit` 1 to MAX_LEASE_BATCH); it failed before the fix on "1 to 64 types". Task 2 done: the two descriptions in crates/cicada/src/api/leases.rs corrected, docs/openapi.json regenerated, version 0.18.11 in Cargo.toml, Cargo.lock and the skill, CHANGELOG and docs handoff. The audit found no other wrong limits in the renew, release or complete bodies, the core guide, the turn-leases spec or ADR 0039.

Task 3 (finishing checks) is not ticked yet: the builder's recorded `gate.py run --e2e` was killed when its turn ended and left an abandoned gate lock; the build loop is taking the recovery to the owner, then the gate run is repeated on this head and task 3 ticked. Checks run directly on c915092 with PostgreSQL: fmt and clippy -D warnings clean; cargo test --workspace 2947 passed, 0 failed; cargo test -p cicada-app 149 passed; script tests 64 passed; feature builds and dependency trees pass; Docker E2E passed; openspec validate --strict valid. The last commit (b4537e2) changes only the handoff.

Departures: none. Breaking: none (descriptions only; limits, validation and schema keywords unchanged). Docs: handed off, no blockers.
