# Proposal

## Why

`job-graph` leaves the dependents of a failed job `waiting`, and has no answer for a job whose worker fails on the
job message or goes away without reporting: the job stays `running` and the run never ends. brood's `brood.pipeline`
marks a job failed on a `Failed` or `Stopped` about its worker, and skips the dependents of a failed job. Issue #2
(higher-level actors) asks the coordinator to own skipped dependents. Split from `job-graph-cancel` on 2026-11-24
after its design review: that change keeps `Cancel` and the cancel message type.

## What Changes

- A failed job skips every waiting job that needs it, directly or through others (status `skipped`).
- The run watches each job's target, and marks a job failed when its job message failed at the worker or its worker
  went away.
- The run ends `succeeded` when every job succeeded, else `failed` when a job failed, else `cancelled`; the last is
  reached only once `job-graph-cancel` cancels jobs.
- Sending a job message, and failing a job on a failed dispatch, apply only while the job is `running`.
- Ready jobs start after every change of a job's status, not only after a result.

The decisions are in [design.md](design.md) (ADR 0060, supplementing ADR 0059).

## Out of scope

`Cancel` and the cancel message type (`job-graph-cancel`). Retry (`job-graph-retry`). Work the worker handed on to
another actor. A graph that changes while it runs. Capacity across runs.

## Order

After `job-graph`, before `job-graph-cancel`.

## Impact

The coordinator of `job-graph` and its tests; no message, field or state shape changes. The end report's contract does
not change: `skipped` and the run's `cancelled` exist since `job-graph`. Additive: the next patch. The docs handoff
extends the guide section of `job-graph`.
