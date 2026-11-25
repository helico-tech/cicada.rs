Verdict: pass

## Blocking findings

None.

## Notes

1. `crates/cicada/src/runtime/system.rs:574-575`: the record is pushed next to `tracing::warn!`, not through it, so the
   test now proves that `start` decides to warn, not that the warning reaches the log or that it is at WARN level.
   Deleting the `tracing::warn!` line would keep the test green. Task 2 suggested this seam and the builder reported
   the departure, so this is a note. A one-line comment at the call site ("keep the two together") would help.
2. `runtime.rs:395`: the first assert (empty record at the start) relies on each test getting its own thread. That
   holds for libtest (it spawns a thread per test, also with `--test-threads=1`) and for nextest (a process per
   test), so it is fine today.
3. Other PostgreSQL runtime tests fill their own threads' `POOL_WARNINGS` and never take them. The amount is tiny and
   only exists in test builds; nothing to change.
4. The `## Cause` section is clear and fits the fix: the other tests' first hit of the call site with no subscriber,
   cached through `Rebuilder::JustOne`. Removing the unused `tracing-subscriber` dev-dependency is correct (no other
   use in `crates/cicada`).

## Checked

Tasks 1-3 are ticked and their work is in the diff. Fast lane, `skip_specs: true`, no specs. The scope matches the
proposal: a `#[cfg(test)]`, `pub(crate)` seam, no change to public items, and the warning text and threshold are
unchanged. The test still proves one warning for pool 6 and none for pool 8 or a node without types. `start` records
the warning before any `.await` that could move work to another thread, and the test runs on a current-thread runtime.
No protected docs are touched. `docs-handoff.md` is there and true to the code ("none for users"). Version none: no
`Cargo.toml` or skill version change, and no CHANGELOG entry is needed for a tests-only change. Gate: `gate.py status
7d9b6585` shows a full pass.

## Not checked

I did not run the test or the suite. I did not reproduce the race before the fix. E2E has no evidence, as expected:
nothing under `e2e/` changed.
