# 0059. A ready-made job graph

Date: 2026-11-24

## Status

Accepted for job-graph. Builds on ADR 0028.

## Context

Flows run fixed workflows written as code (ADR 0031), and reply waits cover "first of" and "all of" (ADR 0044). CI
pipelines, media processing and document imports need a graph of jobs that the caller gives as data and inspects job
by job. brood's `brood.pipeline` runs one: each job is a child actor of one type, created, watched and run by the
pipeline, which takes a result only from one of its running children. Issue #2 adds an in-flight limit, a record of
each attempt, and refusing a result from an attempt that is not current. The graph must be refused before anything
starts when it has a cycle or a need that names no job.

## Decision

- A ready-made job graph in `cicada-actors` under the fixed type name `cicada.job-graph`, one actor per run. The graph
  is its state, given at the create; the constructor and the decoder refuse a cycle, an unknown need and a graph out
  of bounds, so a bad graph never becomes a run. `Start` starts it.
- A job is a message to a target named per job (target and message type, as the batcher), carrying the run, the job,
  the attempt and the job's JSON input. Any actor can be a worker; the graph does not create them.
- Ready jobs start in graph order while fewer than the run's in-flight limit run. Each start is a new attempt,
  numbered per job from 1 and kept with its dispatch and end times, outcome and reason.
- Each job message is sent in a turn of the run's own, so a gone target, or a send the call refuses (a target type
  not registered, a type that does not take the message type, a payload out of bounds), fails only that dispatch. The
  run watches itself: a failed dispatch of its own ends that attempt `failed` with the cause and reason, and the run
  goes on.
- A worker reports an explicit result for a job and attempt. Only a result for a running job's current attempt is
  taken; any other is refused for good, the state unchanged. The sender is not checked.
- A failed job blocks its dependents. The run ends when no job runs and none can start: `succeeded` when every job
  succeeded, else `failed`. The end is reported to a target named at the create, in a turn of its own, by a `Report`
  that anyone may send again.
- A message to the run with a field out of bounds is refused for good, the state unchanged.
- Job statuses include `skipped` and `cancelled` from the start, set only by `job-graph-cancel`, so that change does
  not change the end report's contract. The end report can arrive more than once (a `Report` sent again); a report
  target treats a repeat for the same run as one.

## Consequences

- One graph can mix workers of different types, and a worker can hand the work on: whoever reports names the attempt.
- Without a sender check, any actor or client that can reach the run can report for the current attempt; the attempt
  number fences only stale and foreign attempts.
- A refused result is kept as a failed message of the run, not in its state.
- A worker that fails after its job message arrived leaves its job running until `job-graph-cancel` adds failure
  notices and cancel; blocked jobs stay `waiting` until it sets `skipped`. The contract does not change for that, so
  `job-graph` can be released alone.
- The whole graph is one actor's state: bounds keep it small (100 jobs, 16 KiB of input and of output per job).
