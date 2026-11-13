---
id: remote-worker-runtime.code.2
kind: code
change: remote-worker-runtime
round: 2
base: 1fd016b9b6161a23f73660fa66c7a3a47d597433
head: 46194fc5df70a457eedf3e043c808e3e271d815d
previous: 23bc447e473e11824b78712b552b7a4b5e45ae90
requested: 2026-11-13T20:07:23+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done remote-worker-runtime.code.2 <file>`.

## Note from the requester

Version: 0.16.15

Builder report (round 2: fix of round 1)

All three blocking findings of round 1 are fixed in 46194fc on feat/remote-worker-runtime, each with a new test.

1. Lease request after stop (worker.rs, lease): the stopping/stopped check and the spawn of the lease request now happen together under the lifecycle lock. A stopping or stopped worker sends no request and returns WorkerError::Start; covers manual batches and polling; once a stop has begun no new request can start. New test in tests/worker.rs: a_stop_while_a_manual_batch_registers_sends_no_lease_request_after_it (blocks registration, stops the worker, unblocks: "Worker is STOPPED", no lease request, no release).
2. Cancelled completion outliving the lease (executor.rs, cancel branch): a worker no longer stops renewing (heartbeat and watchdog) on shutdown; renewal stops only after the cancelled completion, at the release, or once the handler is known to have ended. A node still stops renewing at once, as before. New multi-thread test in remote_worker.rs: a_cancelled_completion_after_a_shutdown_stays_within_the_confirmed_lease (operator cancel wins, shutdown while the blocking work is still stopping, complete_turn hangs, renewals fail): the completion is cut off at the confirmed deadline (~1.85 s), no release. With the fix reverted the test fails (completion ran to ~2.7 s, the end of its budget).
3. Handler time: new virtual-time test in remote_worker.rs, a_handlers_now_is_the_claim_and_current_time_counts_from_the_worker_lease_request: the lease answer reaches the worker 3 s after its request, the handler sleeps 2 s; now stays the claim time; current_time is claim time + 5 s, counted from the worker's lease request.

Checks
- cargo fmt --all --check: passes.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test -p cicada with PostgreSQL: 1713 passed, 0 failed (remote_worker 15/15, tests/worker.rs 18/18).
- Not rerun: the full workspace tests and the docs tests (only the cicada crate changed, no docs).

Departures: none; the worker-only strict deadline guard stays; nodes keep their behaviour. Breaking changes: none.
