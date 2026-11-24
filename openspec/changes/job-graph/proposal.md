# Proposal

## Why

Flows run fixed workflows written as code (ADR 0031), and reply waits cover "first of" and "all of" (ADR 0044). They do
not cover a graph of jobs that a caller supplies as data and wants to inspect and retry job by job: CI pipelines,
media processing, document imports. Issue #2 (higher-level actors) names bounded job graphs as its second candidate.

brood already has one. `brood.pipeline` (`crates/brood-core/src/pipeline/actor.rs` and `pipeline.rs`, about 2,200
lines) runs one commit's job graph:

- A job starts when every job in its `needs` succeeded. A cycle is refused before anything starts.
- Each job is a child actor that is created, watched and run in the pipeline's commits. A job reports `JobFinished`
  from its own turn, and the pipeline takes it only from one of its running jobs (Cicada sets the sender).
- A `Failed` or `Stopped` about a running job marks it failed. A failed or cancelled job skips its dependents.
- `Cancel` (one job or all) and `Retry` (a failed or cancelled job, under a new run ID) go to the pipeline.

The issue adds: an in-flight limit, a record of each attempt, and refusing a late result from an attempt that a retry
replaced. cicada-code-factory's run is a fixed flow today, but its steps would fit a graph.

## What Changes

The first of three changes (split on 2026-11-24; the owner asked the change loop to work through the queue on its own
picks): `job-graph` (this one), `job-graph-cancel`, `job-graph-retry`.

A coordinator actor per run that:

- accepts a graph as data and refuses a cycle, an unknown dependency or a graph out of bounds before it dispatches
  anything;
- starts ready jobs up to an in-flight limit, and records each attempt;
- takes an explicit result from a worker for a running attempt, marks the job succeeded or failed, and starts the jobs
  that became ready;
- reports the run's end to a target when every job has a result.

To decide in the design:

- A new crate `cicada-jobs`, or a module in `cicada-actors`.
- What a job is: a child actor of a type the host gives (as in brood), a message to a target, or a flow.
- How a worker reports, and how an attempt is named so that `job-graph-retry` can refuse a late result.
- What the coordinator keeps per job, so a console can show each one.

## Out of scope

Cancel, failure notices and skipped dependents (`job-graph-cancel`). Retry of one job and refusing a late result
(`job-graph-retry`). Capacity from `capacity-permits`: a later proposal if a consumer needs it. A graph that changes while it runs. Scheduling across several graphs beyond what `capacity-permits` gives. Moving
brood or the factory onto it. A YAML or other file format: the host parses its own format into the graph.

## Order

After `capacity-permits`, before `job-graph-cancel`.

## Impact

A new crate or module and its tests; new contracts in `docs/reference/contracts`. An ADR. Additive. The docs handoff
carries a guide section, a pattern for the design guide and a skill route. Link the result in issue #2.
