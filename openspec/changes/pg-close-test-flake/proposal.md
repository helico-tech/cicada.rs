# Proposal

## Why

The PostgreSQL test `a_cancelled_close_leaves_its_resources_to_the_final_drop_or_the_runtime_teardown_without_a_task`
(`crates/cicada/src/tests/postgres/changes.rs`, about line 505, from `store-close-cancellation` in 0.17.4) is flaky.
It reads Tokio's `num_alive_tasks()` before and after it cancels a close, and asserts the count did not rise. That
metric counts every task in the runtime, sqlx's own pool and connection tasks included, so it can rise for reasons
that have nothing to do with the cancelled close. On a full workspace run under load it failed with "2 before, 3 after"
(found while building `flow-reply-waits`), then passed alone three times and on the next full run. A flaky test fails
any full run at random, the local checks of a release included.

## What Changes

- Prove "the cancelled close spawns no task" with something only Cicada controls, not the runtime-wide task count.
  For example: a test-only count of the tasks Cicada's PostgreSQL store spawns, or ownership of the listener's task
  handle. The design picks one.
- Check the SQLite twin (`crates/cicada/src/store/sqlite/change_tests.rs`, about line 421) for the same weakness. It
  asserts an exact count, which is safe only while nothing else in that runtime spawns.
- Keep what the test proves today: unfinished cleanup, the ended change stream, sole ownership and the final drop
  freeing the store.

## Out of scope

The close behaviour itself; other tests; sqlx.

## Order

After `dead-letters-upgrade-wording`. Before `local-push-gate`.

## Impact

Tests in `crates/cicada` only, and at most a test-only hook in the PostgreSQL store. No API, wire format, schema or
guarantee changes.
