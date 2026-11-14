---
id: store-close-cancellation.code.3
kind: code
change: store-close-cancellation
round: 3
base: e5f770ab77c65fe0d423a5843dd065ebcb431463
head: db02d5a6c507d16bdfa870576e3343b189cefebe
previous: c080e678c6bd78c04621a7bf461f119e616446dc
requested: 2026-11-14T13:20:53+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done store-close-cancellation.code.3 <file>`.

## Note from the requester

Version: 0.17.4

Round 3 (owner allowed one more fix round, decision 25a). Fixes only the round 2 blocker, in commit db02d5a.

- crates/cicada/src/store/sqlite/change_tests.rs: the shared helper `close_cancelled_after_taking` no longer spawns a task. It pins `store.close()` in the test's own future, drives it until the hook reports the connection was taken, then drops it there; bounded at 10 s, and it panics if the close finishes instead of stopping. With no helper task, the exact task-count checks cannot count a finished task the scheduler has not removed yet. The retained-reader checks (4, 3, 2 idle readers) and the final-drop/teardown checks are unchanged; the cancelled-connection test uses the same helper.
- PostgreSQL twin: unchanged; it never spawned a helper (it polls a pinned close in its own future and drops it) and checks only that the task count did not rise.

Checks: fmt --check and clippy -D warnings pass; cargo test --workspace with PostgreSQL 2726 passed, 0 failed; the 17 SQLite change tests 50/50 runs; the SQLite final-drop test alone 300/300; both final-drop tests with PostgreSQL 50/50.

Please check the round 2 finding is closed and the fix adds no new problem; everything else was confirmed in round 2.
