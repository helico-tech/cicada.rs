## Implementation

- [x] 1. Restructure only the PostgreSQL cancelled-close/final-drop regression test as designed: setup and driver
  resource drops on the existing store runtime, and one poll/drop/count in a fresh current-thread runtime with no
  root yield. Exit the pinned future's scope before asserting zero live tasks. Preserve the test name, pending/pool
  closed checks, unfinished unlocked cleanup, terminal stream, sole ownership, Weak final-drop proof and both teardown
  legs; add the existing listener-stopped observation. Explain the count's scope and review captured-handle, blocking
  work and listener-abort paths against current sources. Leave the SQLite twin unchanged after confirming its setup
  has no Tokio-spawning driver/settle path. With a real PostgreSQL database and CICADA_PG_URL set, temporarily inject
  a direct tokio::spawn during close and separately a cancellation Drop guard using Handle::try_current().spawn;
  each must fail this targeted test. Revert both mutations and record failures, restored-source pass and exact commands
  in the progress log. Run both targeted PostgreSQL and SQLite tests, checking they executed rather than skipped;
  repeat the PostgreSQL test 20 times. No production hooks, counters, dependencies or lifecycle behavior changes.
- [ ] 2. Write `docs-handoff.md`: no new user behavior, no protected prose/guarantee/link/route changes, unchanged test
  names and Blocking: none; report evidence and limitations accurately, and include release version for docs-owned
  installation pins. Add a test-fix changelog entry and normal patch version at build time, including skill metadata
  and required generated version updates. Verify the docs and skill tests and `python3 scripts/status.py`. Preserve
  order before local-push-gate; do not edit its files or install a gate here.

## Finishing checks

- [ ] 3. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate pg-close-test-flake --strict`. PostgreSQL must be running with CICADA_PG_URL set; a skipped
  regression test does not verify this fix. Run a second full workspace pass with PostgreSQL for the load-sensitive
  regression, recording both results and any unavailable checks. Inspect the final diff to confirm all temporary
  mutations are gone, production code/behavior is unchanged and both existing teardown legs remain. Follow normal
  review and owner-approved landing/release gates.

## Progress log

- Task 1. The PostgreSQL test sets up the store, its stream and the borrowed connection on the store runtime, polls
  and drops close once on a fresh current-thread runtime and asserts zero live tasks there, then checks unfinished
  cleanup, the stopped listener, the ended stream and both teardown legs. Reviewed against the sources: the store holds
  no runtime handle, close and `hub.close()` spawn nothing, and `Run` has no `Drop`; the listener's only spawn is in
  `start`. SQLite twin unchanged: `WriteTx::drop` spawns only in `Phase::Committing` with changes, which the harness
  never reaches, and sqlx-sqlite 0.9.0 (locked) runs its connection workers on `thread::Builder` OS threads.
- Mutations, each with `CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada --features
  postgres --lib tests::postgres::changes::a_cancelled_close_leaves`: `tokio::spawn(async {})` right after close sets
  its flag failed with `left: 1, right: 0`; a local guard in close whose `Drop` calls
  `Handle::try_current().spawn(async {})` failed the same way. Source restored (`git status` shows only the test), and
  the test passes again.
- Targeted runs: `... --lib a_cancelled_close_leaves` ran both twins, 2 passed; the PostgreSQL test alone 20 times in a
  row, 20 passed, 0 failed.
