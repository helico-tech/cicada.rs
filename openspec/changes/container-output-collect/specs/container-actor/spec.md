## ADDED Requirements

### Requirement: The actor collects a container's outputs before its end

When the actor has an `OutputStore` and the container ended after it ran, the actor SHALL, before delivering the end,
store the rest of the container's log with the guarded append and, when the job names outputs, copy that folder
within its limits into the store, and SHALL report the stored and skipped files in the end's `outputs`. A retryable
failure SHALL arm the next poll and count as a runtime error; at the maximum errors in a row the end SHALL be delivered
with the error in `outputs`.

#### Scenario: Outputs at the end

- **WHEN** a container with outputs `/work/out` exits after writing two files there, one over the byte limit
- **THEN** the store holds the small file and the rest of the log, and the end reports 1 stored and 1 skipped with its
  reason

#### Scenario: A slow copy

- **WHEN** two containers end at once and one's copy is slow
- **THEN** the other's end is delivered without waiting for it

#### Scenario: A file the store refuses

- **WHEN** the output folder holds a file whose name has a control character, next to two plain files
- **THEN** the two plain files are stored and the odd one is listed as skipped with the store's reason

#### Scenario: A container removed during the collection

- **WHEN** the container is removed by someone else while its outputs are collected
- **THEN** the end is delivered with what was stored

#### Scenario: A bad outputs path

- **WHEN** the job's outputs path is relative
- **THEN** the collection ends at once and the end is delivered with the error in `outputs`

#### Scenario: An end from 0.22

- **WHEN** an owner decodes an end message without `outputs`
- **THEN** it decodes, with `outputs` absent

## MODIFIED Requirements

### Requirement: A deadline and errors in a row end a container

A container past its deadline SHALL be stopped and end `timedOut`. The deadline SHALL be set at its start from
`ContainerJob::timeout` (by default the settings' timeout), capped at the largest instant, and not moved. A runtime
error SHALL count in a row and delay the next poll by a growing backoff up to the settings' maximum, never failing the
turn. At the maximum errors in a row, a container without an end SHALL end `failed` with the kind `runtimeErrors`; a
removal SHALL keep trying. A log read for the end that fails with `Unsupported` SHALL not count: the end SHALL be
recorded at once with `(no logs: …)` and the runtime's message. At the maximum errors in a row while a recorded end's
outputs are collected, the end SHALL be delivered with its recorded outcome and the error in `outputs`.

#### Scenario: The deadline passes

- **WHEN** a container with a timeout of 60 s still runs after 60 s of virtual time
- **THEN** the actor stops it and records the end `timedOut`

#### Scenario: A timeout per request

- **WHEN** the settings' timeout is 60 s and the job returns 10 s for one request and none for another
- **THEN** the first container ends `timedOut` after 10 s, and the second still runs after 120 s

#### Scenario: Too many errors in a row

- **WHEN** every `inspect` of a started container fails with `DaemonUnavailable` and `max_errors` is 3
- **THEN** the polls wait longer after each error, and the third error records the end `failed` with the kind
  `runtimeErrors` and the last error

#### Scenario: An ended container without logs

- **WHEN** a container ends and the runtime answers its log read with `Unsupported`
- **THEN** the end is recorded in that turn with the logs `(no logs: …)`, the errors in a row are not counted, and the
  log read is not retried

#### Scenario: Errors while outputs are collected

- **WHEN** an exited container's output copy fails with `DaemonUnavailable` for `max_errors` polls in a row
- **THEN** the end is delivered as `exited` with its code, and `outputs.error` holds the error
