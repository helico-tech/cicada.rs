# Tasks

Fast lane, tests only: version `none` (no crate behaviour changes; a test-only seam in `cicada` is allowed). The test
`a_node_warns_at_start_when_its_pool_is_smaller_than_its_concurrency_plus_4` in
`crates/cicada/src/tests/postgres/runtime.rs` fails most runs of the gate's build
(`CARGO_PROFILE_DEV_DEBUG=line-tables-only`), also on main (e39fd912). The warning, its threshold and its text stay as
they are.

- [x] 1. **Reproduce and find the cause.** Run the test the way the gate builds it, with PostgreSQL:
  `CARGO_PROFILE_DEV_DEBUG=line-tables-only CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada --lib a_node_warns_at_start`,
  alone and with the whole `cicada` lib suite, several times. Find why the warning is not caught (the global
  call-site interest cache, the max level, another thread, the subscriber's writer, or the warning not being sent).
  Write the cause in a few lines into this change's `proposal.md` under a `## Cause` section. Verify: the failure
  reproduces before the fix.
- [x] 2. **Fix the test at the cause.** Catch the warning in a way that does not depend on what other tests do to
  `tracing`'s global state, for instance a `pub(crate)` seam that records the start warning for tests in
  `crates/cicada/src/tests`, while the pure function `pool_warning` keeps unit tests for the text; or a fix of the
  subscriber setup when the cause is there. No retry loop, no sleep, no weaker assertion: the test still proves that
  `start` warns once for a small pool and not for a pool of concurrency + 4 or a node without types. Verify: the
  command of task 1, 10 times in a row with the gate's profile, alone and in the lib suite.
- [ ] 3. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace` with PostgreSQL, commit, then `python3 scripts/gate.py run` (no E2E: nothing under
  `e2e/`, `sample-service/` or the HTTP crates changes).
