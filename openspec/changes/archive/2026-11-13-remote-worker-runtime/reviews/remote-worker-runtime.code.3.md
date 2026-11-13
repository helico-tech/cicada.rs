---
id: remote-worker-runtime.code.3
kind: code
change: remote-worker-runtime
round: 3
base: 1fd016b9b6161a23f73660fa66c7a3a47d597433
head: fc4df7fb217c9a6ab71c7efcfbffeab49ee1f02d
previous: 46194fc5df70a457eedf3e043c808e3e271d815d
requested: 2026-11-13T21:07:42+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done remote-worker-runtime.code.3 <file>`.

## Note from the requester

Version: 0.16.15

Builder report (round 3: fix of round 2; the owner allowed this extra round)

The round-2 blocker is fixed in fc4df7f on feat/remote-worker-runtime: a cancelled worker turn stops renewing at the cancel timeout, so stuck work's lease expires.

Fix (executor.rs): on shutdown a cancelled worker turn keeps its keeper (renewals and the deadline watchdog) for at most shutdown_cancel_timeout while its work stops; the timeout reaches the executor through a new private field, renew_while_stopping.
- If the work stops in time, the turn ends as before: a cancelled completion (still bounded by the watchdog), a release, or nothing.
- If it does not, renewals end and the turn is marked lost: never released, never completed; its lease expires.
- A node gets None and stops renewing at once, as before.
- The same bound applies to the turns of a dropped run_due_turns.

New test in remote_worker.rs: work_that_outlives_the_cancel_timeout_is_no_longer_renewed_and_its_lease_expires (600 ms TTL, a handler blocking 2.5 s without yielding, a 200 ms cancel timeout, renewals that succeed): after stop returns there are no more renewals; while the work still blocks another caller leases the message with lost_claims == 1 (expired, not released); after the work ends there is no release, no completion and no outcome. With the bound removed (keep renewing for an hour) the test fails with "renewed after the cancel timeout".

Docs: Worker::stop_with_grace and run_due_turns docs, the core guide's Life cycle bullet and the architecture's Workers section state the bound.

Checks
- cargo fmt --all --check: passes.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test -p cicada with PostgreSQL: 1714 passed, 0 failed (remote_worker 16/16).
- cargo test -p cicada-examples --doc: 54 passed; --test docs: 11 passed.
- Not rerun: the full workspace tests (only the cicada crate and docs changed).

Departures: none. Note: with shutdown_cancel_timeout zero, a worker abandons cancelled turns at once instead of releasing them, so they expire.
Breaking changes: none.
