# job-graph Specification

## Purpose
The ready-made job graph `cicada.job-graph` runs a graph of jobs given as data: it refuses a bad graph before
anything starts, dispatches ready jobs up to an in-flight limit, fences results by attempt and reports the run's end
(ADR 0059).

## Requirements

### Requirement: A job graph run under a fixed type name

`cicada-actors` SHALL provide a job graph coordinator, `cicada.job-graph/<run>`, hosted by `JobGraphActor` without
settings. Its state SHALL hold the graph (each job's name, needs, target, message type and input), the in-flight
limit, the report target and type, the run's status and end, and per job its status, output and attempts. Job
statuses SHALL include `skipped` and `cancelled`, which this change never sets. Its contract SHALL match
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

### Requirement: Ready jobs start up to the in-flight limit

On `Start` and after every result it takes, the run SHALL start each waiting job whose needs all succeeded, in graph
order, while fewer jobs run than the in-flight limit. Starting a job SHALL mark it `running` with a new attempt,
numbered per job from 1, in that commit. A second `Start` SHALL change nothing.

#### Scenario: A diamond graph

- **WHEN** a run with `a`, `b` and `c` needing `a`, and `d` needing `b` and `c`, gets `Start` and every job succeeds
- **THEN** `a` starts first, `b` and `c` start only after `a` succeeded, and `d` starts only after both succeeded

#### Scenario: The in-flight limit

- **WHEN** a run with four independent jobs and in-flight limit 2 gets `Start`
- **THEN** two jobs run, and the third starts only when one of them has its result

### Requirement: Each job message is sent in a turn of its own

The commit that starts a job SHALL send the run an internal `Dispatch`. Only a `Dispatch` the run sent itself, for
the job's current attempt not yet dispatched, SHALL send the job's message type to its target with the run, job,
attempt and input, and record the dispatch time. A send the call refuses SHALL fail that `Dispatch` for good. The run
SHALL watch itself; when its own `Dispatch` ends failed, that attempt and job SHALL end `failed` with the cause and
reason, and the run SHALL go on.

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

A waiting job with a failed need SHALL stay `waiting`. Jobs that do not need a failed job SHALL still start. When no
job runs and none can start, the run SHALL end in that commit: `succeeded` when every job succeeded, else `failed`,
with its end time.

#### Scenario: A failed job

- **WHEN** a run has `a`, `b` needing `a`, and an independent `c`, and `a` fails while `c` runs
- **THEN** `b` stays `waiting`, `c` keeps running, and the run ends `failed` when `c` has its result

#### Scenario: Every job succeeds

- **WHEN** the last running job of a run succeeds and every other job has succeeded
- **THEN** the run ends `succeeded` in the commit of that result

### Requirement: The end is reported to the report target

The commit that ends a run SHALL send the run a `Report`. A `Report` on an ended run SHALL send the report message
type to the report target with the run, its status and each job's status, last attempt, output and reason, in graph
order; on a running run it SHALL do nothing. Anyone may send `Report`, so a report target can get the same report
more than once.

#### Scenario: The end report

- **WHEN** a run with `a` succeeded and `b` failed ends
- **THEN** its report target gets a report with status `failed`, `a` `succeeded` with its output and `b` `failed`
  with its reason

#### Scenario: A report sent again

- **WHEN** an ended run gets `Report` again
- **THEN** its report target gets the same report again, and the run's state does not change

#### Scenario: A gone report target

- **WHEN** the last result of a run arrives while the report target does not exist
- **THEN** the result is taken and the run ends, its `Report` fails, and a later `Report` after the target is created
  sends the report
