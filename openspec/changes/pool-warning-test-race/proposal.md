# Proposal

## Why

`a_node_warns_at_start_when_its_pool_is_smaller_than_its_concurrency_plus_4`
(`crates/cicada/src/tests/postgres/runtime.rs`, from `pool-size-check`) failed once in a full gate run while building
`container-info` (2026-11-25): it caught 0 pool warnings where it expects 1, and then passed 6 times in a row. The test
catches the warning with a subscriber set as this thread's default (`tracing::subscriber::set_default`), while other
tests in the same process set and drop their own. `tracing` caches per call site whether anyone listens, and that cache
is global: a rebuild by another thread at the wrong moment can mark the warning's call site as unheard, so the event is
dropped. A flaky gate costs a 7-minute rerun and hides real failures.

## Cause

Found in task 1. The test was the only `tracing` subscriber in the `cicada` test binary, and `tracing-core` 0.1.36 treats
a process with one registered dispatcher specially: it computes a newly hit call site's interest from the default of
the thread that hits it first (`Rebuilder::JustOne` in `callsite.rs`), and caches that interest for the whole process.
The warning's call site is not only hit by this test: `Cluster::store` opens a pool of 6 for nodes of concurrency 4, so
every other PostgreSQL runtime test logs the same warning from its own thread, which has no subscriber. When one of them
hits the call site first, after this test set its subscriber, the call site is cached as never heard, and this test's
own warning is dropped before it reaches the subscriber. A first hit before the test sets its subscriber is repaired
by the rebuild of `set_default`, so the result depends on test order and timing: the gate's profile
(`CARGO_PROFILE_DEV_DEBUG=line-tables-only`) and the workspace build, with its unified `tracing` features, make it
fail often (2 of 4 runs of the workspace-built lib suite here), `cargo test -p cicada` rarely. Diagnostics showed the
warning sent on the test's thread, the max level at INFO and a new call site on the same thread caught, so only the
cached interest of that one call site was wrong.

## What Changes

- The test sees the warning without depending on the global call-site cache, for instance through a hook the node
  already has, or by checking the warning text from `pool_warning` and that `start` calls it, rather than through a
  thread-local subscriber. The design picks the way; the warning itself and its text stay as they are.

## Lane

Lane: fast

## Out of scope

The warning, its threshold and its text. Other tests that capture logs, unless the design finds the same race there.

## Order

After `container-info`, before `container-registry-auth`: the test fails 3 of 3 gate runs on main (e39fd912) with the
gate's profile (`CARGO_PROFILE_DEV_DEBUG=line-tables-only`), so no head gets gate evidence until it is fixed.

## Impact

`crates/cicada/src/tests/postgres/runtime.rs`, perhaps a test-only seam in `crates/cicada/src/runtime/system.rs`.
Tests only: no release of its own.
