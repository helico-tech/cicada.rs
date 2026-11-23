# Proposal

## Why

A full gate run on 2026-11-23 failed once in
`a_caller_starts_a_heartbeat_round_through_the_executor_over_http` and passed 5 of 5 runs alone. The cause is in the
tests, not the node: `serve` writes `Listening on http://<address>` with `writeln!` (`crates/cicada-app/src/node.rs`),
which may reach the test's output buffer in pieces. The test helpers poll that buffer and accept the first line that
starts with `Listening on `, even before its newline arrives. A helper can then read the base URL as `http://`, and
every request goes to the wrong place.

Two helpers have the same loop: `sample-service/tests/sample_service.rs` (`start_as`) and
`crates/cicada-app/tests/common/mod.rs`. A flaky test fails a 7-minute gate run for nothing.

## What Changes

- Both helpers accept the listening line only once its newline has arrived: they look at the complete lines of the
  output, not at a last line still being written.
- No change to `serve` or its output.

## Lane

Lane: fast. Tests only.

## Out of scope

Writing the line in one call in `node.rs`: the output stays as it is, and the helpers must not rely on how it is
written. Other flaky tests.

## Order

After `dependency-advisories`, before `container-runtime`. It goes first because every gate run can hit it.

## Impact

Two test helpers. No crate's behaviour, API or output changes, so it lands without a release and waits for the next
batch.
