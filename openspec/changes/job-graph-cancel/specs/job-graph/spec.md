## MODIFIED Requirements

### Requirement: A job graph run under a fixed type name

`cicada-actors` SHALL provide a job graph coordinator, `cicada.job-graph/<run>`, hosted by `JobGraphActor` without
settings. Its state SHALL hold the graph (each job's name, needs, target, message type, optional cancel message type
and input), the in-flight limit, the report target and type, the run's status and end, and per job its status
(`waiting`, `running`, `succeeded`, `failed`, `skipped` or `cancelled`), output and attempts. Its contract SHALL match
`docs/reference/contracts/cicada.job-graph.json`.

#### Scenario: A run from its create

- **WHEN** a run is created with jobs `a` and `b`, `b` needing `a`, and in-flight limit 2
- **THEN** its status is `running`, both jobs are `waiting` with no attempts, and no job message is sent

### Requirement: A bad graph is refused before the run exists

The constructor and the state decoder SHALL refuse, with the reason, a graph with a cycle, a need that names no job of
the graph, a repeated need, a job that needs itself, a repeated job name, no jobs or more than 100, a name that is
not 1 through 64 ASCII letters, digits, `-`, `_` or `.`, a message type or cancel message type out of bounds, an
input over 16 KiB or 32 levels, or an in-flight limit outside 1 through 100.

#### Scenario: A cycle

- **WHEN** a state is built with `a` needing `c`, `b` needing `a` and `c` needing `b`
- **THEN** the constructor refuses it and names a job of the cycle, and the decoder refuses the same JSON

#### Scenario: An unknown need

- **WHEN** a state is built with `deploy` needing `test`, and no job is named `test`
- **THEN** the constructor and the decoder refuse it and name `deploy` and `test`

#### Scenario: A cancel message type out of bounds

- **WHEN** a state is built with a job whose cancel message type is empty
- **THEN** the constructor and the decoder refuse it and name the job

#### Scenario: An inconsistent state

- **WHEN** a state is decoded with more running jobs than its in-flight limit, a running job whose need has not
  succeeded, more than 20 attempts of a job, attempt numbers that are not consecutive or do not end at the current
  attempt, or status `running` while no job runs and none can start
- **THEN** the decoder refuses it with the reason

#### Scenario: Inconsistent skipped jobs or end status

- **WHEN** a state is decoded with a skipped job whose needs all succeeded, a skipped job with an attempt, a waiting
  job with a failed need, or an ended run with status `succeeded` and a failed job
- **THEN** the decoder refuses it with the reason

#### Scenario: Inconsistent cancelled jobs

- **WHEN** a state is decoded with a cancelled job whose last attempt ended `succeeded` or is still open, an attempt
  with outcome `cancelled` of a job that is not cancelled, or an ended run with status `cancelled` and a failed job
- **THEN** the decoder refuses it with the reason

## ADDED Requirements

### Requirement: Cancel cancels waiting and running jobs

`Cancel` SHALL name one job, or none for every job. A named waiting or running job SHALL become `cancelled` in its
commit; a running job's attempt SHALL end with outcome `cancelled`. Ended and skipped jobs, and an ended run, SHALL be
left alone. An unknown or out-of-bounds job name SHALL be refused for good, state unchanged. A result for a cancelled
job's attempt SHALL be refused, and a worker's result SHALL NOT name the outcome `cancelled`.

#### Scenario: Cancel one running job

- **WHEN** job `a` runs, `b` needs `a`, and the run gets `Cancel` for `a`
- **THEN** `a` becomes `cancelled`, `b` becomes `skipped`, and a later `Result` for `a`'s attempt is refused for good

#### Scenario: Cancel the whole run

- **WHEN** a run with one succeeded, one running and two waiting jobs gets `Cancel` without a job
- **THEN** the running and waiting jobs become `cancelled`, the succeeded job stays, and the run ends `cancelled` and
  is reported

#### Scenario: Cancel the whole run after a failure

- **WHEN** a run with one failed and one running job gets `Cancel` without a job
- **THEN** the running job becomes `cancelled`, and the run ends `failed`

#### Scenario: Cancel one job of a run that otherwise succeeds

- **WHEN** a run has independent jobs `a` and `b`, `a` is cancelled, and `b` succeeds
- **THEN** the run ends `cancelled`

#### Scenario: A cancel frees a slot

- **WHEN** a run with independent jobs `a` and `b` and in-flight limit 1 runs `a`, and gets `Cancel` for `a`
- **THEN** `a` becomes `cancelled`, and `b` starts in the same commit

#### Scenario: Cancel of an ended job

- **WHEN** a run gets `Cancel` for a job that has succeeded
- **THEN** nothing changes

#### Scenario: Cancel of an unknown job

- **WHEN** a run gets `Cancel` for a job name that is not in its graph
- **THEN** the message is refused for good and the state does not change

#### Scenario: A failed dispatch reported after the cancel

- **WHEN** job `a`'s target does not exist, its `Dispatch` fails, and the run gets `Cancel` for `a` before it handles
  the `Failed` about that `Dispatch`
- **THEN** `a` becomes `cancelled`, and stays `cancelled` after that `Failed`

### Requirement: A running worker is told through the job's cancel message type

When a dispatched running job with a cancel message type is cancelled, the run SHALL send itself an internal
`SendCancel`. Only a `SendCancel` the run sent itself, while that job is `cancelled` with that attempt, SHALL send the
cancel message type to the job's target with the run, job and attempt. A job without a cancel message type, or not
yet dispatched, SHALL NOT be told. A failed `SendCancel` SHALL change nothing.

#### Scenario: A worker is told

- **WHEN** a dispatched job with cancel message type `CancelBuild` is cancelled
- **THEN** its target gets `CancelBuild` with the run, the job and the attempt, in a later turn of the run

#### Scenario: A gone worker

- **WHEN** a dispatched job's target is deleted and the job is cancelled before the `Stopped` arrives
- **THEN** the `Cancel` commits and the job is `cancelled`, and only the `SendCancel` fails

#### Scenario: A job not yet dispatched

- **WHEN** a running job is cancelled before its `Dispatch` ran
- **THEN** its target gets neither the job message nor the cancel message
