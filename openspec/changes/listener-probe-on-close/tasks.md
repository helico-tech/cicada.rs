## Implementation

- [ ] 1. Strengthen the PostgreSQL close regression in crates/cicada/src/tests/postgres/changes.rs. Keep its stream,
  unavailable and no-restart assertions. Use a separate test PgListener on the isolated TestDb channel, wait with a
  timeout for PROBE_PAYLOAD and save its notification sender PID. Capture the production listener PID before close
  from its application name and LISTEN query containing the test channel; require one distinct listener and keep
  the observer outside that identity. Close the store and use eventually to assert that neither saved PID remains
  in pg_stat_activity on the healthy test server. Clean up test observer/inspection connections; do not change
  production application_name or introduce test hooks. Run this targeted test with CICADA_PG_URL against the unfixed
  listener first and record the expected failure with the surviving probe PID. Use the existing real-PostgreSQL
  multi-thread harness and bounded positive-condition waits, not paused-time socket waits or a fixed pre-close sleep.
- [ ] 2. In Run::run, replace the spawned probe and its explicit abort with tokio::select! over receive and a scoped
  probe borrowing the attempt's pool/channel. Have the probe loop return Infallible; preserve cadence, silence timeout,
  error logging, hub transitions, fresh pools and reconnect backoff. Update listener/connect doc comments to describe
  scoped ownership and asynchronous driver cleanup; add the changelog entry and next patch version at build time.
  Do not alter the main pool's close protocol or add an await for driver cleanup. Run the new close regression green,
  the lost-listener/resync test, reconnect wake-up test and notification-flood test with PostgreSQL. Verify existing
  stream closure and unavailable behavior, and inspect that there is no independent probe JoinHandle left.

## Finishing checks

- [ ] 3. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with
  CICADA_PG_URL for PostgreSQL tests/conformance, `cargo check -p cicada` with no features and with `--features postgres`,
  and `openspec validate listener-probe-on-close --strict`. Record any unavailable database checks explicitly; a
  skipped PostgreSQL test does not establish that the regression passes. Confirm no public API, wire, schema or
  guarantee changes and no edits to the deferred store-close-cancellation behavior. Follow the build loop's
  normal code-review and release gates.
