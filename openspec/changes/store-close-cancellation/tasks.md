## Implementation

- [ ] 1. Add SQLite cancellation/concurrency regressions beside the private store change/lifecycle tests. Use the
  existing commit fault/settle harness to hold the writer, poll close to Pending, cancel it, and show a later caller
  actually removes writer/readers after the blocker is released. A concurrent second close must remain pending while
  the blocker is held. Inspect writer contents only after releasing its lock; do not deadlock the test on its own
  blocker. Cover never-polled close, unavailable/terminal-stream behavior and a reader held across close returning
  without refilling the pool. Record the cancellation/concurrency failures on the unfixed code. Use controlled
  readiness/polling, not elapsed sleeps; keep the existing SQLite driver harness and virtual time where no I/O deadline
  depends on real time. Run the targeted lifecycle/change test module.
- [ ] 2. Implement SQLite's idempotent admission-stop prefix and tokio cleanup gate/done flag. Retain untouched
  resources across cancellation: take the writer for its close, then pop and close idle readers one at a time. Move
  Reader::drop's closed check under the readers mutex. Preserve reads already in flight, writer settlement and driver
  cleanup of a connection whose own close future is cancelled. Test return/drain ordering with controlled concurrency
  where needed and review both lock orders for deadlocks. Run the new tests green plus existing SQLite close/settle
  tests; distinguish the before-writer-lock cancellation proof from the weaker driver-cleanup boundary after ownership
  transfer. Do not add a background close task or wait for active readers.
- [ ] 3. Add PostgreSQL regressions in the private PostgreSQL tests using minimal test-only pool access. Start LISTEN,
  hold a main-pool connection and leave an idle connection, poll close to Pending and cancel. Assert pool admission is
  closed, store operations are unavailable and streams end. A later/concurrent close must stay pending while the
  borrowed connection is held, finish after release and leave main-pool size zero; another close must finish normally.
  Add never-polled coverage. Demonstrate the regression on old code, whether cancellation occurred at the listener join
  or pool wait. Use the existing real-PostgreSQL multi-thread harness, bounded waits and positive barriers/polling, not
  arbitrary sleeps. Run targeted PostgreSQL lifecycle tests with CICADA_PG_URL; skipped tests are not successful proof.
- [ ] 4. Implement PostgreSQL's repeated synchronous prefix, marking pool admission closed by explicitly dropping
  the future returned by sqlx Pool::close, and document why this works with the pinned API. Split listener stop into
  synchronous abort/disable-start and handle transfer; retain and await the JoinHandle by mutable reference inside
  shared Closing progress. Complete main-pool close under the gate and write done only last. Add a deterministic
  private test of cancellation while joining that proves the handle remains reachable and a later caller joins it;
  a controlled handle fixture is preferable to racing task scheduling. Run the new PostgreSQL regressions green and
  the predecessor's PID-close, reconnect/resync and wake-up tests. No new runtime capture/task or Arc cycle; reconcile
  with the landed scoped-probe implementation before editing listener.rs.
- [ ] 5. Update Store::close and Backend::close docs, architecture, operations and changelog/version at build time.
  Explain first-poll effect, never-polled no-op, resumability, concurrent waiters and unfinished cleanup when no caller
  resumes. State the distinct backend completion limits: PostgreSQL borrowed connections, SQLite writer/active reads,
  and driver cleanup after an owned connection close is cancelled. No fixed shutdown deadline or graceful cleanup
  after runtime teardown; App/ActorSystem shutdown order stays. Reconcile the predecessor's scope disclaimer in the
  change-feed spec with the new resumable-close contract using this change's delta. Preserve the one-behavior scope,
  memory backend behavior and lack of API/schema/wire changes. Run examples doctests and docs checks.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with
  CICADA_PG_URL, `cargo check -p cicada` with no features and each of `--features sqlite` and `--features postgres`,
  and `openspec validate store-close-cancellation --strict`. Include both SQL conformance suites and existing memory,
  feed-close and SQLite settlement tests. Record unavailable database checks, verify all targeted cancellation tests
  exercised cleanup resources rather than only flags, and follow the build loop's normal code-review/release gates.
