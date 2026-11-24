# Proposal

## Why

`job-graph` runs a graph of jobs to its end when every job succeeds or reports a failure. It has no answer for a job
whose worker is gone, and no way to stop a run. brood's `brood.pipeline` has both: a `Failed` or `Stopped` about a
running job marks it failed, a failed or cancelled job skips its dependents, and `Cancel` stops one job or all of them.
Issue #2 (higher-level actors) asks for the coordinator to own cancellation and skipped dependents.

## What Changes

- A failed job skips every job that needs it, directly or through others; the run ends when no job can start.
- The coordinator learns when a running job's worker failed or stopped, and marks that job failed.
- `Cancel` (one job, or the whole run) cancels waiting jobs at once and running ones by telling their workers; a
  cancelled job skips its dependents too.
- The end report names each job's outcome: succeeded, failed, skipped or cancelled.

## Out of scope

Retry (`job-graph-retry`). A graph that changes while it runs. Capacity across runs.

## Order

After `job-graph`, before `job-graph-retry`.

## Impact

The coordinator of `job-graph` and its tests and contract. Additive within the unreleased or next patch version. The
docs handoff extends the guide section of `job-graph`.
