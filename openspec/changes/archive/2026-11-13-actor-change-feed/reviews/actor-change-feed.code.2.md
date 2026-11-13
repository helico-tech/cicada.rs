---
id: actor-change-feed.code.2
kind: code
change: actor-change-feed
round: 2
base: 45f7fc4ad422394fbd9ec4ecf3fadc3c75393fba
head: fad1f59d5279c4b116b78cdeabf9fbb21504d7ba
previous: 4af1aac5b0598b842d64e839606442e3fd515493
requested: 2026-11-13T16:52:29+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-change-feed.code.2 <file>`.

## Note from the requester

Version: 0.16.10

Builder report (round 2: fix of round 1)

Both blocking findings of round 1 are fixed in fad1f59 on feat/actor-change-feed.

Finding 1 (runtime shutdown during SQLite settlement), in crates/cicada/src/store/sqlite/change_tests.rs:
- New test a_runtime_shut_down_during_settlement_frees_the_writer_and_a_fresh_subscription_reads_the_settled_file opens one store on a main runtime and keeps it.
- It starts a create on a second, single-threaded runtime, stops it at the commit, then drops that runtime (and with it the settle task).
- On the main runtime it checks: the next write finishes within 10 s (the writer is freed); that write's BEGIN rolls back the left-over transaction first; a new subscription starts with Resync and a full read shows the settled file.
- Two cases: COMMIT already sent to the driver (the create landed), and COMMIT asked for but not sent (rolled back). No hint is expected during the shutdown.

Finding 2 (the PostgreSQL re-run test could not see a leak):
- The test hook rerun_next_commit in crates/cicada/src/store/postgres/store.rs now takes a future to run between the rollback and the re-run, instead of a flag.
- a_re_run_transaction_starts_with_an_empty_collection deletes p with children p-000, p-001, p-002; the first attempt collects all four and rolls back. Before the re-run the hook detaches p-001 with plain SQL, so the committing attempt never touches it.
- It asserts the committed batch is exactly p, p-000, p-002 and that p-001 still exists; a surviving collection would add p-001. The comment says so. (No mutation test run.)

Checks
- SQLite change tests: 8 passed, 5 runs in a row.
- PostgreSQL change tests: 9 passed, 3 runs in a row.
- tests::postgres, the SQLite tests and the PostgreSQL conformance suite: 252 passed, 0 failed.
- cargo fmt --all --check and cargo clippy --workspace --all-targets -- -D warnings: clean.
- Not re-run this round: the full workspace tests.

The listener probe-task issue (postgres/listener.rs) is left alone; the build loop takes it to the owner. No new departures, no new breaking changes; the only non-test code change is the shape of the test-only hook.
