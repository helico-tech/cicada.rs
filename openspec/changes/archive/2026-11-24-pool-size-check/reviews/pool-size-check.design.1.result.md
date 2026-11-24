Verdict: ready for tasks

## Blocking findings

None. One behaviour, about four tasks. Claims checked against the code: `Store` delegates `Backend` methods such as `wake_ups` (handle.rs:381). `start` already branches on `init.types.is_empty()` (system.rs:541). `Settings::pool_size` saturates at `concurrency + 4` (settings.rs:241). `serve` builds its `ActorSystem` on the opened store (node.rs:372, 408). `openspec validate` passes.

## Notes

1. **App scenario without a test.** In specs/app, "A given pool that is too small" expects a warning that names 5, 16 and 20. Design 6 tests only that the default pool never goes below the rule. Either add a `cicada-app` test that resolves `--store-pool-size 5 --concurrency 16` and runs `pool_warning` on the result, or rewrite the scenario as "the store is opened with 5 connections" and leave the warning to store-backends.
2. **Shared pool not covered.** `Store` is `Clone`, and `ActorSystem` implements `TurnSource`. Two nodes on one store, or a local `Worker` on a node, share one pool, but each node checks only its own concurrency. Add a line to Out of scope, next to the HTTP-worker exclusion, so the docs handoff does not promise a full check.
3. **Capturing the log.** A thread-local `set_default` subscriber only sees the warning if the test runs on a current-thread runtime and the warning is logged inside `start` before any spawn. Say this in the test task.

## Questions for the owner

- Q-SHARED: Should the warning mention shared pools (note 2), or is Out of scope enough? My pick: Out of scope.

## Not checked

I built nothing and ran no tests. I did not check whether some other path on the server holds connections for a long time, which would make the +4 margin too small. That margin comes from the existing docs.
