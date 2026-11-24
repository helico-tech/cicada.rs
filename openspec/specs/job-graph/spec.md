# job-graph Specification

## Purpose
The ready-made job graph `cicada.job-graph` runs a graph of jobs given as data: it refuses a bad graph before
anything starts, dispatches ready jobs up to an in-flight limit, fences results by attempt and reports the run's end
(ADR 0059).

## Requirements

### Requirement: A job graph run under a fixed type name

`cicada-actors` SHALL provide a job graph coordinator, `cicada.job-graph/<run>`, hosted by `JobGraphActor` without
settings. Its state SHALL hold the graph (each job's name, needs, target, message type, optional cancel message type
and input), the in-flight limit, the report target and type, the run's status, end and count of ends, and per job its
status (`waiting`, `running`, `succeeded`, `failed`, `skipped` or `cancelled`), output and attempts. Its contract SHALL
match `docs/reference/contracts/cicada.job-graph.json`.

#### Scenario: A run from its create

- **WHEN** a run is created with jobs `a` and `b`, `b` needing `a`, and in-flight limit 2
- **THEN** its status is `running` with no ends, both jobs are `waiting` with no attempts, and no job message is sent

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
  job with a failed need, an ended run with status `succeeded` and a failed job, or an ended run with no ends
- **THEN** the decoder refuses it with the reason

#### Scenario: Inconsistent cancelled jobs

- **WHEN** a state is decoded with a cancelled job whose last attempt ended `succeeded` or is still open, or an ended
  run with status `cancelled` and a failed job
- **THEN** the decoder refuses it with the reason

#### Scenario: A waiting job with old attempts

- **WHEN** a state is decoded with a waiting job whose last attempt is closed `failed` or `cancelled`, and with a
  waiting job whose last attempt is open
- **THEN** the decoder takes the first and refuses the second

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

### Requirement: Only a result for the current attempt is taken

A `Result` SHALL name the job, the attempt and the outcome, `succeeded` or `failed`, with an optional output and
reason. The run SHALL take it only when the job is running and the attempt is its current one: the attempt ends with
the outcome and the job keeps the output. Any other `Result` SHALL be refused for good with the reason, the state
unchanged. The sender SHALL NOT be checked.

#### Scenario: A result for another attempt

- **WHEN** job `a` runs attempt 1, and a `Result` for `a` with attempt 2 arrives
- **THEN** the result is refused for good and `a` still runs attempt 1

#### Scenario: A late result

- **WHEN** job `a` has succeeded, and another `Result` for `a` with attempt 1 arrives
- **THEN** the result is refused for good and `a`'s outcome and output do not change

#### Scenario: A result from another actor

- **WHEN** an actor that is not `a`'s target sends a `Result` for `a`'s current attempt
- **THEN** the run takes it

### Requirement: Bad input is refused for good

A message with a field out of bounds SHALL be refused for good and leave the state unchanged: a job name out of
bounds, an output over 16 KiB as JSON or deeper than 32 levels, and a reason longer than 2,000 characters. A report
or job message that the send refuses SHALL fail only its own turn.

#### Scenario: An output too large

- **WHEN** a running job's current attempt reports `succeeded` with an output of 17 KiB
- **THEN** the result is refused for good and the job still runs

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

### Requirement: The end is reported to the report target

The commit that ends a run SHALL raise its count of ends and send the run a `Report`. A `Report` on an ended run SHALL
send the report message type to the report target with the run, the number of its current end, its status and each
job's status, last attempt, output and reason (the last attempt's, only when its outcome is the job's status), in
graph order; on a running run it SHALL do nothing. Anyone may send
`Report`, so a report target can get the same report more than once.

#### Scenario: The end report

- **WHEN** a run with `a` succeeded and `b` failed ends for the first time
- **THEN** its report target gets a report with end 1, status `failed`, `a` `succeeded` with its output and `b`
  `failed` with its reason

#### Scenario: A report sent again

- **WHEN** an ended run gets `Report` again
- **THEN** its report target gets the same report again with the same end number, and the run's state does not change

#### Scenario: A gone report target

- **WHEN** the last result of a run arrives while the report target does not exist
- **THEN** the result is taken and the run ends, its `Report` fails, and a later `Report` after the target is created
  sends the report

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
`SendCancel`. Only a `SendCancel` the run sent itself, while that attempt is still kept, dispatched and closed
`cancelled`, SHALL send the cancel message type to the job's target with the run, job and attempt, whatever the job's
status is then. A job without a cancel message type, or not
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

### Requirement: Retry puts a failed or cancelled job back to waiting

`Retry` SHALL name one job that is `failed` or `cancelled` and whose needs are none of `failed`, `skipped` or
`cancelled`. In its commit the job SHALL become `waiting` with its output cleared and its attempts kept, every
`skipped` job SHALL become `waiting` and the skips SHALL be computed again, an ended run SHALL go back to `running`,
and ready jobs SHALL start, the retried one with the next attempt number.

#### Scenario: Retry a failed job

- **WHEN** in a run with in-flight limit 2 job `a` failed in attempt 1, and the run gets `Retry` for `a`
- **THEN** `a` starts with attempt 2 in the same commit, and its job message names attempt 2

#### Scenario: Skipped dependents come back

- **WHEN** `a` and `x` failed, `b` needs `a`, `c` needs `b`, `d` needs `a` and `x`, all three skipped, and the run gets
  `Retry` for `a`
- **THEN** `b` and `c` become `waiting`, and `d` stays `skipped`

#### Scenario: A late result of a replaced attempt

- **WHEN** `a` failed in attempt 1, was retried and runs attempt 2, and a `Result` for `a`'s attempt 1 arrives
- **THEN** the result is refused for good, and `a` still runs attempt 2

#### Scenario: Retry before the cancel notice ran

- **WHEN** dispatched job `a` with cancel message type `CancelBuild` is cancelled, and the run gets `Retry` for `a`
  before it handles the `SendCancel` of attempt 1
- **THEN** `a` starts attempt 2, and the target still gets `CancelBuild` for attempt 1

#### Scenario: Retry after the cancel notice ran

- **WHEN** dispatched job `a` with cancel message type `CancelBuild` was cancelled, its target got `CancelBuild` for
  attempt 1, and the run gets `Retry` for `a`
- **THEN** `a` starts attempt 2, and no second cancel is sent

#### Scenario: A retried job cancelled before it starts

- **WHEN** failed job `a` is retried while no slot is free, and is cancelled before it starts
- **THEN** `a` is `cancelled`, and a later end report shows `a` without the old attempt's reason

#### Scenario: Retry within the in-flight limit

- **WHEN** a run with in-flight limit 1 runs `b`, and gets `Retry` for the failed independent `a`
- **THEN** `a` becomes `waiting`, and starts only when `b` has its result

### Requirement: A retried run ends again with a higher end number

A retry of an ended run SHALL set it `running` and clear its end time. It SHALL end again by the same rule, and its
report SHALL carry the next end number, so a report target can keep the report with the highest end number per
run. An end whose `Report` had not run before the run was reopened SHALL NOT be reported, so numbers can have gaps.

#### Scenario: A run that ends twice

- **WHEN** a run ended `failed` with end 1, its failed job is retried and succeeds, and every other job succeeded
- **THEN** the run ends `succeeded` and its report target gets a report with end 2

#### Scenario: A report while running again

- **WHEN** a reopened run that has not ended again gets `Report`
- **THEN** no report is sent

### Requirement: A bad retry is refused for good

A `Retry` SHALL be refused for good, state unchanged, for an unknown job or a job name out of bounds, a job that is not
`failed` or `cancelled`, a job with a need that is `failed`, `skipped` or `cancelled`, a job whose last attempt
number is the largest an attempt can have, and an ended run whose count of ends is the largest it can have.

#### Scenario: Retry of a running job

- **WHEN** a run gets `Retry` for a job that is `running`, as after a second `Retry` of the same job
- **THEN** the message is refused for good and the state does not change

#### Scenario: Retry of a job whose need failed

- **WHEN** `b` needs `a`, `b` was cancelled while waiting, then `a` failed, and the run gets `Retry` for `b`
- **THEN** the message is refused for good with a reason that names `a`, and the state does not change

#### Scenario: Retry of an unknown job

- **WHEN** a run gets `Retry` for a job name that is not in its graph
- **THEN** the message is refused for good and the state does not change
