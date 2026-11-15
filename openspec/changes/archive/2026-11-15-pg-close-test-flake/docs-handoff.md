# Docs handoff for pg-close-test-flake

Version: 0.18.8. Refresh the dependency pins of `docs/how-to/install.md` once it is published. No API, wire format,
contract hash, store schema, guarantee, CLI or HTTP route changes.

## What changed

None for users. The change fixes a flaky test, not behaviour:

- `a_cancelled_close_leaves_its_resources_to_the_final_drop_or_the_runtime_teardown_without_a_task` in
  `crates/cicada/src/tests/postgres/changes.rs` keeps its name and its proofs (pending close, closed pool, unfinished
  cleanup, ended change stream, sole ownership, final drop, both teardown legs) and adds the stopped listener. It now
  polls and drops the cancelled close on a fresh current-thread runtime and asserts zero live tasks there, instead of
  comparing the store runtime's task count, which sqlx's own cleanup could raise.
- The count proves only that no task is left on the runtime that polls and drops the close. It does not see a spawn
  through a runtime handle captured earlier, blocking work, an OS thread, or work in the listener's abort path; review
  covers those (the store holds no handle, close spawns nothing, the listener's `Run` has no `Drop`).
- The spec delta of `store-backends` clarifies the `No retry yet` scenario: the cancelled close schedules no Cicada
  shutdown task, while sqlx's connection cleanup on the store runtime is allowed. This was already the documented
  contract; the documentation's statement that cancellation schedules no shutdown task stays true.
- The SQLite twin is unchanged.

Why no prose: no guide, guarantee row, doctest, link or skill route names this test or describes its method.

## Blocking

None. Every doctest, link, guarantee row and skill route passes without prose edits.

## Status

Checks run on the code branch, PostgreSQL on port 55432 with `CICADA_PG_URL` set: two temporary mutations of close (a
`tokio::spawn` after the flag, a `Drop` guard spawning through `Handle::try_current()`) each failed the test, then
were reverted; the PostgreSQL test 20 times in a row (20 passed); both twins together (2 passed); `--test docs` (11
passed) and `--test skill` (8 passed); `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
warnings` (clean); `openspec validate pg-close-test-flake --strict` (valid); `cargo test --workspace` twice (2905
passed, 0 failed each time). The final diff touches no production code. Repeated passes support the structural fix;
they do not promise the test can never flake. Nothing is written in docs-owned files, and no owner fix is integrated.
