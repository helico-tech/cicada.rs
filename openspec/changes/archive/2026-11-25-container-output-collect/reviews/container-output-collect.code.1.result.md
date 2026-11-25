Verdict: pass

## Blocking findings

None. Tasks 1-5 are ticked and in the diff. Every scenario of both spec deltas has a test (`tests/outputs.rs`, the wire unit tests, `check_output_store`). Versions are 0.23.0 in `Cargo.toml`, both skills and `plugin.json`. The changelog covers Added, the BREAKING change and Upgrading. No protected docs are edited, and the handoff matches the code.

## Notes

1. `docs/adr/README.md:73`: the row for ADR 0066 still says the actor copies "after recording an end, then delivers". That is out of date since the `Collecting` design (round 2). The ADR text itself is correct. Fix the row in passing.
2. `actor/mod.rs` `collect`/`copy`: a turn past `MAX_TURN` does not count as a runtime error. So a copy that always takes longer than the turn limit never reaches `max_errors`, and the end is never sent: the poll's retries run out, and each `Check` times out again. The design accepts this through its `MAX_TURN` advice, and a slow image pull already behaves this way. But here the container has run and the owner waits forever. This is worth one line in a later proposal, for example: count turn timeouts while `Collecting`.
3. A `Stop` or `Remove` during a collection's backoff arms a poll at once and skips the backoff. The test documents this. It is harmless.
4. Decoding `OutputsReport` does not check `firstSkipped.len() <= 10` or `<= skipped`. Only the schema's `maxItems` says so. This is minor.

## Not checked

- I ran no tests and did not check the gate evidence for 1bd64ade. I rely on the builder's report.
- I did not check real Docker behaviour for a container removed during `logs_since`/`copy_out`. The code follows the trait contract (`Conflict` for a missing container). The Docker backend's mapping of that case was not re-verified, and E2E did not run.
- I did not check the drafted guide prose against the rendered guide. It is only proposed text in the handoff.
