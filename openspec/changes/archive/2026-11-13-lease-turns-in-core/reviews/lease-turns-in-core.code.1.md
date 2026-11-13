---
id: lease-turns-in-core.code.1
kind: code
change: lease-turns-in-core
round: 1
base: 5882a292ba3fc78b825d94b42310c48c1d277592
head: 5a7213d47ae84d61ad2b5a26154189f31f746a85
requested: 2026-11-13T18:33:19+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done lease-turns-in-core.code.1 <file>`.

## Note from the requester

Version: 0.16.13

Builder report (round 1)

All six tasks of lease-turns-in-core are built on branch feat/lease-turns-in-core. Every check passes. Nothing pushed, merged or tagged.

Tasks
1. Done: renewal keeps the later of the held expiry and store now plus the TTL, in memory, SQLite and PostgreSQL. Four new conformance cases; the s5 expired-commit case now waits past the kept expiry before it commits.
2. Done: lease_turns, renew_turn and release_turn on ActorSystem, with TurnLease, LeaseTurnsResult, RenewTurnResult, MAX_LEASE_BATCH and MAX_LEASE_TYPES. 16 public tests in crates/cicada/tests/leases.rs.
3. Done: messages at the claim limit are filtered before handoff. 6 tests in src/tests/runtime/remote_claim_limit.rs.
4. Done: 11 fault and race tests in src/tests/runtime/remote_lease.rs, including a shared SQLite file and PostgreSQL.
5. Done: core guide section with a tested example, architecture, operations, guarantees table, CHANGELOG, version 0.16.13.
6. Done: finishing checks.

Checks
- cargo test --workspace --no-fail-fast with PostgreSQL: 2539 passed, 0 failed.
- Conformance alone: 916 passed. Fuzz: 89 passed.
- Docs doctests: 52 passed. Docs table and links test: 11 passed.
- cargo fmt --all --check and cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo check -p cicada with no feature and with each feature alone: clean.
- openspec validate lease-turns-in-core --strict: valid.
- docs/openapi.json rewritten only for the version bump.
- Not run: the long proptest run and the Docker E2E tests.

Departures and open questions
- The claim-limit filtering code landed in the task 2 commit; task 3 added its tests.
- Claims that are not handed out are released in the background, one attempt each.
- Order of errors in lease_turns: request bounds, then NodeConfig, then stopping state, then clock overflow, then the registry read.
- release_turn also wakes the local dispatch loop.
- A release hook added to the Faults test harness.
- The clock-overflow test is in tests/leases.rs, not in remote_lease.rs.
- ADR 0037 needed no change.
- No open questions.

New concepts
- TurnLease: a turn handed to a worker outside the system; holds the claim and the TTL the server asked for. The worker renews it or gives it back by actor and epoch, through any server on the same store.
- Conservative deadline: the worker's own monotonic time when it sent the request, plus the TTL, minus the smaller of TTL/10 and one second. Store times are not used for deadlines.
- Renewal never shortens a lease: a server with a shorter TTL cannot end a lease a worker still holds.

Breaking changes: none in the API. One behaviour change: a renewal no longer shortens a held lease; nodes that share one TTL see no difference.
