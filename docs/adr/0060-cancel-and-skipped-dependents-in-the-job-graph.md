# 0060. Cancel and skipped dependents in the job graph

Date: 2026-11-24

## Status

Accepted for job-graph-failures and job-graph-cancel. Supplements ADR 0059.

## Context

ADR 0059's job graph leaves a failed job's dependents waiting, has no answer for a worker that fails or goes away
without reporting, and cannot stop a run. brood's pipeline skips the dependents of a failed or cancelled job, marks a
job failed on a `Failed` or `Stopped` about its child actor, and cancels one job or all. A job graph's worker is any
target, not a child, and Cicada tells only the watchers of an actor that a message to it failed, never its sender.

## Decision

- A failed, skipped or cancelled job skips every waiting job that needs it, directly or through others, in the same
  commit. The run ends when nothing runs and nothing can start: `succeeded` when every job succeeded, else `failed`
  when a job failed, else `cancelled`.
- The run watches each job's target from the turn that sends the job message, and unwatches it when none of its
  running jobs has it. A `Failed` about the target fails the job only when it is about that job's message for its
  current attempt; a `Stopped` about the target fails every dispatched running job with that target.
- `Cancel` names one job, or none for all. Waiting and running jobs become `cancelled` in its commit; ended and
  skipped jobs and an ended run are left alone; an unknown job name is refused for good.
- A job may name a cancel message type. A cancelled job that was dispatched and names one is told so in a turn of the
  run's own, so a gone worker fails only that notice. A job without one is not told.

## Consequences

- The run cannot see work its worker handed on, a worker that lost its outside work without failing a message,
  stopping or reporting, or the failure of a later message of the worker. A target that hands work on and then stops
  itself fails its job.
- A `Stopped` does not say which instance stopped, and a result whose turn is rescheduled can be overtaken by a
  `Stopped` of the same worker: in both rare cases the job ends `failed`, and a retry can run it again.
- A job message and its failure count only while the job is `running`, so a job that was cancelled or got its result
  first keeps that status. A worker may get a cancel before the job message it belongs to.
- A shared worker's other failures reach the run while it watches that worker: one ignored turn each.
- A cancelled job's outside work goes on unless its worker takes the cancel message; its late result is refused.
- The end report's contract does not change: the statuses existed since `job-graph`. The run's own contract does.
