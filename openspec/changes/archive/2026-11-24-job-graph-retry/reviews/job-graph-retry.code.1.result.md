Verdict: pass

## Blocking findings

None.

## Notes

1. `crates/cicada-actors/src/job_graph.rs:1101`: the decoder takes a running state with `ends` = `u32::MAX`. When that run then ends, the `checked_add` fails for good, so the last `Result`, `Cancel` or `Failed` is refused and the run stays running. Only a hand-made state, not a `Retry`, gets there. Refuse it in the decoder, or document it.
2. `job_graph.rs:1088`: a pending `SendCancel` of a replaced attempt sends nothing if 20 newer attempts start before it runs, because the oldest kept attempt is dropped. A caveat in the handoff is enough.
3. The modified scenario "A run from its create" says "with no ends". `a_run_from_its_create_sends_no_job_message` does not check `ends == 0`. Only the `full["ends"] = 0` round trip in `wire_formats.rs` covers it. Add one assert.

## Checked

- Tasks 1–6 are ticked, and each one's work is in the diff.
- Every scenario in the spec delta has a test.
- `retry`, `reopen`, the end count in `settle`, the report's `reason` filter and the decoder rules (`check_job`, `ends` on an ended run) match the design's decisions 1–7.
- A job with old attempts always has needs that succeeded, so it can never be skipped.
- The contract is regenerated. CHANGELOG and ADR 0061 have their rows. The version is 0.20.8 in `Cargo.toml`, both skills and `plugin.json`.
- No protected docs are edited. The docs handoff matches the code.
- `main` moved since the base, but only under `openspec/changes/container-logs`, so nothing overlaps.

## What I did not check

I ran no tests, builds or gate checks. I relied on the builder's report and its gate evidence for 1437b0a. I did not read `docs/adr/0061-retry-in-the-job-graph.md` in full, only its index row. I read old tests only for the modified scenarios.
