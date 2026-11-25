## MODIFIED Requirements

### Requirement: A deadline and errors in a row end a container

A container past its deadline SHALL be stopped and end `timedOut`. The deadline SHALL be set at its start from
`ContainerJob::timeout` (by default the settings' timeout), capped at the largest instant, and not moved. A runtime
error SHALL count in a row and delay the next poll by a growing backoff up to the settings' maximum, never failing the
turn. At the maximum errors in a row, a container without an end SHALL end `failed` with the kind `runtimeErrors`; a
removal SHALL keep trying. A log read for the end that fails with `Unsupported` SHALL not count: the end SHALL be
recorded at once with `(no logs: …)` and the runtime's message.

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
