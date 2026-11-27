Verdict: pass

The revised lease wording matches the Stop Now cancellation path in `executor.rs:462–507` across all four files. The executor releases eligible leases and stops renewing abandoned ones so they expire. The changes resolve the round 4 findings. Diff check passed. Tests were not run.
