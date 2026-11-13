---
id: complete-leased-turns.code.2
kind: code
change: complete-leased-turns
round: 2
base: 90def0399d1870853a5143af278685408ed64d0c
head: 463bad070499500387735f038de0bfcffc58fc06
previous: d9fd84d380fc9099859313c97ffd3195f6802947
requested: 2026-11-13T19:21:19+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done complete-leased-turns.code.2 <file>`.

## Note from the requester

Version: 0.16.14

Builder report (round 2: fix of round 1)

Both blocking findings of round 1 are fixed in 463bad0 on feat/complete-leased-turns. Only tests and a test-only import changed; runtime code unchanged.

Finding 1 (SQLite cancellation test used real-time sleeps): the 24-round sleep sweep is removed. New test on_a_shared_sqlite_file_a_completion_cancelled_at_each_commit_phase_applies_once stops the completion's commit with the SQLite commit-fault hooks at each point:
- BeforeCommit: the transaction rolls back; the retry through the second handle is Completed.
- Queued (commit asked for, not yet sent to the driver): rolls back; the retry is Completed.
- InFlight (the driver runs the commit): applies; the retry is LeaseLost.
Each case waits until the hook is reached, aborts the task and asserts it was cancelled, not finished. A write through the first handle settles the open transaction before the retry. Every case ends with version 1, exactly one message sent, no lease held and no observer events. No sleeps. A test-only re-export of CommitFault in crates/cicada/src/store/sqlite/mod.rs; the renamed test is updated in docs/guarantees.md.

Finding 2 (no test that completion wakes the local dispatcher): new paused-time test a_remote_completion_wakes_the_local_dispatch_loop_also_when_its_answer_is_lost_and_tells_no_observer. A started server with a one-hour poll interval hosts the counter actor; the memory store sends no wake-ups of its own. It leases a job turn (never claimed by that server's dispatcher) and completes it with a send to the counter, once acknowledged and once applied with the answer lost. Both times the counter's message is delivered in under 1 s of virtual time; the observer records only the counter's two turns. Removing each wake_up call in complete_turn in turn made the test fail both times (then restored).

Checks
- cargo fmt --all --check: clean.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- remote_completion tests: 12 passed, 5 runs in a row.
- store::sqlite tests: 26 passed.
- cicada-examples --test docs: 11 passed.
- cargo test --workspace with PostgreSQL: 2571 passed, 0 failed.

No departures, no open questions.
