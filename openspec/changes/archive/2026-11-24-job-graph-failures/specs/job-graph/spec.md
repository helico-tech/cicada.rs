## MODIFIED Requirements

### Requirement: A job graph run under a fixed type name

`cicada-actors` SHALL provide a job graph coordinator, `cicada.job-graph/<run>`, hosted by `JobGraphActor` without
settings. Its state SHALL hold the graph (each job's name, needs, target, message type and input), the in-flight
limit, the report target and type, the run's status and end, and per job its status (`waiting`, `running`,
`succeeded`, `failed`, `skipped` or `cancelled`), output and attempts. Its contract SHALL match
`docs/reference/contracts/cicada.job-graph.json`.

#### Scenario: A run from its create

- **WHEN** a run is created with jobs `a` and `b`, `b` needing `a`, and in-flight limit 2
- **THEN** its status is `running`, both jobs are `waiting` with no attempts, and no job message is sent

### Requirement: A bad graph is refused before the run exists

The constructor and the state decoder SHALL refuse, with the reason, a graph with a cycle, a need that names no job of
the graph, a repeated need, a job that needs itself, a repeated job name, no jobs or more than 100, a name that is
not 1 through 64 ASCII letters, digits, `-`, `_` or `.`, a message type out of bounds, an input over 16 KiB or 32
levels, or an in-flight limit outside 1 through 100.

#### Scenario: A cycle

- **WHEN** a state is built with `a` needing `c`, `b` needing `a` and `c` needing `b`
- **THEN** the constructor refuses it and names a job of the cycle, and the decoder refuses the same JSON

#### Scenario: An unknown need

- **WHEN** a state is built with `deploy` needing `test`, and no job is named `test`
- **THEN** the constructor and the decoder refuse it and name `deploy` and `test`

#### Scenario: An inconsistent state

- **WHEN** a state is decoded with more running jobs than its in-flight limit, a running job whose need has not
  succeeded, more than 20 attempts of a job, attempt numbers that are not consecutive or do not end at the current
  attempt, or status `running` while no job runs and none can start
- **THEN** the decoder refuses it with the reason

#### Scenario: Inconsistent skipped jobs or end status

- **WHEN** a state is decoded with a skipped job whose needs all succeeded, a skipped job with an attempt, a waiting
  job with a failed need, or an ended run with status `succeeded` and a failed job
- **THEN** the decoder refuses it with the reason

### Requirement: Ready jobs start up to the in-flight limit

On `Start` and after every change of a job's status (a result, a failure from the watch on its target, a failed
dispatch, a cancel), the run SHALL start each waiting job whose needs all succeeded, in graph order, while fewer jobs
run than the in-flight limit. Starting a job SHALL mark it `running` with a new attempt, numbered per job from 1, in
that commit. A second `Start` SHALL change nothing.

#### Scenario: A diamond graph

- **WHEN** a run with `a`, `b` and `c` needing `a`, and `d` needing `b` and `c`, gets `Start` and every job succeeds
- **THEN** `a` starts first, `b` and `c` start only after `a` succeeded, and `d` starts only after both succeeded

#### Scenario: The in-flight limit

- **WHEN** a run with four independent jobs and in-flight limit 2 gets `Start`
- **THEN** two jobs run, and the third starts only when one of them has its result

#### Scenario: A slot freed by a gone worker

- **WHEN** a run with independent jobs `a` and `b` and in-flight limit 1 runs `a`, and a `Stopped` about `a`'s target
  arrives
- **THEN** `a` ends `failed`, and `b` starts in the same commit

### Requirement: Each job message is sent in a turn of its own

The commit that starts a job SHALL send the run an internal `Dispatch`. Only a `Dispatch` the run sent itself, while
the job is `running` and its current attempt is not yet dispatched, SHALL send the job's message type to its target
with the run, job, attempt and input, and record the dispatch time. A send the call refuses SHALL fail that
`Dispatch` for good. The run SHALL watch itself; when its own `Dispatch` ends failed while the job is `running` with
that attempt, the attempt and job SHALL end `failed` with the cause and reason, and the run SHALL go on.

#### Scenario: A gone target

- **WHEN** jobs `a` and `b` start together and `a`'s target does not exist
- **THEN** `b` is dispatched and can report, and `a` ends `failed` with a reason that names the cause

#### Scenario: An unregistered target type

- **WHEN** jobs `a` and `b` start together and `a`'s target is of a type no node registers
- **THEN** `a`'s `Dispatch` fails for good with the cause `refused`, `a` ends `failed` with that reason, and `b` is
  dispatched

#### Scenario: A type that does not take the message

- **WHEN** a job's target type does not take the job's message type
- **THEN** its `Dispatch` fails for good with the cause `refused`, the job ends `failed` with that reason, and the
  run goes on as for any failed job

#### Scenario: A message the run did not send itself

- **WHEN** a client sends the run a `Dispatch` for a running job, or a `Failed` that names one of its dispatches
- **THEN** the run sends no job message and changes no job

#### Scenario: A failed dispatch of a job that is no longer running

- **WHEN** job `a`'s target does not exist, and a client's `Result` for `a`'s attempt 1 reaches the run's mailbox
  after `a`'s `Dispatch` and before the `Failed` about it
- **THEN** the `Dispatch` fails, the result is taken and `a` ends `succeeded`, and the `Failed` changes nothing

### Requirement: A failed job blocks its dependents and the run ends when nothing can start

In the commit that a job becomes `failed`, `skipped` or `cancelled`, every waiting job that needs it, directly or
through other jobs, SHALL become `skipped`, with no attempt. Jobs that do not need it SHALL still start. When no job
runs and none can start, the run SHALL end in that commit, with its end time: `succeeded` when every job succeeded,
else `failed` when a job failed, else `cancelled`.

#### Scenario: A failed job

- **WHEN** a run has `a`, `b` needing `a`, `c` needing `b`, and an independent `d`, and `a` fails while `d` runs
- **THEN** `b` and `c` become `skipped` in the commit of `a`'s result, `d` keeps running, and the run ends `failed`
  when `d` has its result

#### Scenario: Every job succeeds

- **WHEN** the last running job of a run succeeds and every other job has succeeded
- **THEN** the run ends `succeeded` in the commit of that result

## ADDED Requirements

### Requirement: A job whose worker failed or went away fails

The turn that sends a job message SHALL watch the job's target, and the run SHALL unwatch a target once none of its
running jobs has it. A `Failed` the store wrote about a target for a running job's current job message from the run
SHALL end that job `failed` with the cause. A `Stopped` the store wrote about a target SHALL end every running,
dispatched job with that target `failed`. Other notifications, and a client's, SHALL change nothing.

#### Scenario: The job message fails at the worker

- **WHEN** job `a`'s worker fails its handler on `a`'s job message until the retries run out, and `b` needs `a`
- **THEN** `a` ends `failed` with a reason that names the cause, and `b` becomes `skipped`

#### Scenario: The worker is deleted

- **WHEN** jobs `a` and `b` run on the same target, both dispatched, and an operator deletes that target
- **THEN** both end `failed` with a reason that names the cause

#### Scenario: Another message of the worker fails

- **WHEN** job `a` runs on a shared worker, and a message to that worker that is not `a`'s job message fails
- **THEN** `a` keeps running

#### Scenario: A worker that reports and stops

- **WHEN** a worker sends `a`'s `succeeded` result and stops itself in the same turn, and the run's turn on the result
  is not rescheduled
- **THEN** the run takes the result, and `a` ends `succeeded`

#### Scenario: A notification a client sent

- **WHEN** a client sends the run a `Failed` that names a running job's message, or a `Stopped` about its target
- **THEN** no job changes
