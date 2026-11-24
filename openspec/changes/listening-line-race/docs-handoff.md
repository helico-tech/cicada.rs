# Docs handoff for listening-line-race

Version: none. Nothing is released, so there are no dependency pins to refresh.

## What changed

None for users. Only two test helpers change: `Running::serve` in `crates/cicada-app/tests/common/mod.rs` and
`Node::start_as` in `sample-service/tests/sample_service.rs`. They now look for the `Listening on <url>` line only
among lines that end in a newline. Before, they could read a line that `serve` had not finished writing. `serve`, its
output and every public item stay the same.

## Blocking

None.

## Status

Proposed text: none needed. `cargo test -p cicada-app --all-features` passed 188 of 188 tests in each of three runs,
and `cargo test -p cicada-sample-service` passed 38 of 38 in each of three runs. No prose is still to be written.
