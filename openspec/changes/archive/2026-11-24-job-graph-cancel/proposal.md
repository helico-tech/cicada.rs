# Proposal

## Why

`job-graph` and `job-graph-failures` run a graph of jobs to its end, but cannot stop a run or one of its jobs.
brood's `brood.pipeline` has `Cancel` of one job or all of them, and a cancelled job skips its dependents. Issue #2
(higher-level actors) asks for the coordinator to own cancellation. Split on 2026-11-24 after design review 1: the
skipped dependents and the failure notices moved to `job-graph-failures`.

## What Changes

- `Cancel` (one job, or the whole run) cancels waiting and running jobs at once; a cancelled job skips its dependents
  (`job-graph-failures`' rule), and a run that ends with a cancelled job and no failed one ends `cancelled`.
- A job may name a cancel message type, through which its running worker is told.
- The end report names each job's outcome: succeeded, failed, skipped or cancelled.

The decisions are in [design.md](design.md) (ADR 0060, supplementing ADR 0059).

## Out of scope

Skipped dependents and failure notices (`job-graph-failures`). Retry (`job-graph-retry`). A graph that changes while
it runs. Capacity across runs.

Telling work the worker handed on: cancel tells only the job's own target (its `to`); a worker that hands work on
passes the cancel along itself.

## Order

After `job-graph-failures`, before `job-graph-retry`.

## Impact

The coordinator of `job-graph` and its tests and contract. Telling a running worker to cancel needs a cancel message
type per job, a new field of the job. The statuses `skipped` and `cancelled` are already in `job-graph`'s contract, so
the end report's contract does not change. Additive: the next patch. The docs handoff extends the guide section of
`job-graph`.
