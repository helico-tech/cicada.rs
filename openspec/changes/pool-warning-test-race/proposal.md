# Proposal

## Why

`a_node_warns_at_start_when_its_pool_is_smaller_than_its_concurrency_plus_4`
(`crates/cicada/src/tests/postgres/runtime.rs`, from `pool-size-check`) failed once in a full gate run while building
`container-info` (2026-11-25): it caught 0 pool warnings where it expects 1, and then passed 6 times in a row. The test
catches the warning with a subscriber set as this thread's default (`tracing::subscriber::set_default`), while other
tests in the same process set and drop their own. `tracing` caches per call site whether anyone listens, and that cache
is global: a rebuild by another thread at the wrong moment can mark the warning's call site as unheard, so the event is
dropped. A flaky gate costs a 7-minute rerun and hides real failures.

## What Changes

- The test sees the warning without depending on the global call-site cache, for instance through a hook the node
  already has, or by checking the warning text from `pool_warning` and that `start` calls it, rather than through a
  thread-local subscriber. The design picks the way; the warning itself and its text stay as they are.

## Lane

Lane: fast

## Out of scope

The warning, its threshold and its text. Other tests that capture logs, unless the design finds the same race there.

## Order

After `container-registry-auth`. No following change.

## Impact

`crates/cicada/src/tests/postgres/runtime.rs`, perhaps a test-only seam in `crates/cicada/src/runtime/system.rs`.
Tests only: no release of its own.
