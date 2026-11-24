## MODIFIED Requirements

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

## ADDED Requirements

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
