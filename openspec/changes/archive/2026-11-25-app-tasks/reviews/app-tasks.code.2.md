---
id: app-tasks.code.2
kind: code
change: app-tasks
round: 2
base: 782aca33187843e9f37c4ba82dc29138f2e11485
head: f32ca855946059f0d4f3ee2328ab55939df9d07c
previous: 75fabb3766167e32cd6e0a98902bec6b74cba368
requested: 2026-11-25T18:34:34+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done app-tasks.code.2 <file>`.

## Note from the requester

Version: 0.22.0 (the unreleased batch)

Round 2 (head f32ca855, rebased onto main 782aca33): each task writes its end into a shared record (first early end wins)
before it sets Stopping; Tasks::finish takes the exit code from it; the abort warning names only tasks without a written
end. New test an_early_end_with_a_grace_period_of_zero_still_fails_the_command (20 rounds; fails on the old code with a
5 ms sleep after stopping.set()). The warnings test moved to crates/cicada-app/tests/task_logs.rs (a separate binary, as
another thread's log line without a subscriber switched the call site off). Long lines in the new code wrapped. Skipped
note 1 (failed HTTP server test): axum's serve does not fail on accept errors without a test hook; the path shares the
tested stop. Checks: cicada-app 197, workspace with PostgreSQL 3528, examples 11/41/18, fmt, clippy; gate full and e2e
pass of f32ca855.
