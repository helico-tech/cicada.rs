# Docs handoff for pool-warning-test-race

Version: none. Nothing is released, so there are no dependency pins to refresh.

## What changed

None for users. The test `a_node_warns_at_start_when_its_pool_is_smaller_than_its_concurrency_plus_4`
(`crates/cicada/src/tests/postgres/runtime.rs`) no longer catches the pool warning with a thread-local `tracing`
subscriber. `ActorSystem::start` still logs the warning with `tracing::warn!`; in test builds of `cicada` only, it also
records the text for the current thread, and the test reads that record (`take_pool_warnings`, `pub(crate)` and
`#[cfg(test)]`). The warning, its threshold and its text stay the same. `cicada` drops its now unused
`tracing-subscriber` dev-dependency. No public item changes.

## Blocking

None.

## Status

Proposed text: none needed. Checks and their results are in the builder's report; no prose is still to be written.
