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

#### Scenario: No store

- **WHEN** an actor without a store records an end
- **THEN** it records and sends the end in one commit, without `outputs`

#### Scenario: A store that comes back

- **WHEN** the store fails with `Unavailable` for two polls during the collection and then answers
- **THEN** the files are stored and the end is delivered without an error

#### Scenario: An end from 0.22

- **WHEN** an owner decodes an end message without `outputs`
- **THEN** it decodes, with `outputs` absent

## MODIFIED Requirements

### Requirement: The end is reported once, before removal

A container SHALL end once, as `exited` with the program's exit code, `failed` with the runtime's reason, `timedOut`
or `stopped`. A new `failed` end SHALL carry a `kind`: `notStarted`, `outOfMemory`, `runtimeEnded`, `missing` or
`runtimeErrors`; other ends SHALL carry none. The commit that records the end SHALL save it in the state with the tail
of the logs and SHALL send `{key, end}` to the target named in the first `Start`; an actor with an `OutputStore` SHALL
first hold an end the container ran for as pending in the phase `Collecting` and record and send it, with `outputs`,
in the commit that finishes the collection. No step SHALL remove a container before its end is recorded.

#### Scenario: An exit is reported with its logs

- **WHEN** a started container exits with code 3 and logs `done`
- **THEN** one commit records the end `exited` with code 3, no kind and logs `done`, and sends it to the target

#### Scenario: Remove after the exit

- **WHEN** an exited container whose end is recorded gets `Remove`
- **THEN** the runtime removes it and the actor sends no second end

#### Scenario: An unknown target type

- **WHEN** `Start` names a target type that is not registered, and the container exits and then gets `Remove`
- **THEN** the end is saved as undelivered with the reason, no poll fails, and the container is removed

#### Scenario: Kinds of failure

- **WHEN** a container's image is unknown; its start is refused with `InvalidSpec`; the runtime reports it `Failed`
  with the reason `OUT_OF_MEMORY`; `Failed` with another reason; `Missing` while it runs
- **THEN** the ends are `failed` with the kinds `notStarted`, `notStarted`, `outOfMemory`, `runtimeEnded` and
  `missing`, and the refused start ends in the poll that tried it

#### Scenario: A kind where none belongs

- **WHEN** a state or a `ContainerEnded` holds a `stopped` end with a `kind`
- **THEN** decoding it fails

#### Scenario: An end saved before kinds

- **WHEN** a state saved by 0.20.8 holds a `failed` end without `kind`
- **THEN** it decodes with no kind

#### Scenario: Start and Remove during the collection

- **WHEN** a container is `Collecting` and gets a second `Start` and then `Remove`
- **THEN** the end is sent once, to the first target, when the collection finishes, and the container is removed after

#### Scenario: An end saved by 0.22

- **WHEN** a state saved by 0.22 is `Ended` with an end, and the actor now has a store
- **THEN** it collects nothing and sends no second end


### Requirement: A deadline and errors in a row end a container

A container past its deadline SHALL be stopped and end `timedOut`. The deadline SHALL be set at its start from
`ContainerJob::timeout` (by default the settings' timeout), capped at the largest instant, and not moved. A runtime
error SHALL count in a row and delay the next poll by a growing backoff up to the settings' maximum, never failing the
turn. At the maximum errors in a row, a container in the phase `New` or `Started` SHALL end `failed` with the kind
`runtimeErrors`; a removal SHALL keep trying. A log read for the end that fails with `Unsupported` SHALL not count: the
end SHALL be recorded at once with `(no logs: …)` and the runtime's message. At the maximum errors in a row in the phase
`Collecting`, the pending end SHALL be recorded and delivered with its outcome and the error in `outputs`.

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

### Requirement: A container reports progress through a file

When `Start` names a `progressType`, the container actor SHALL set `CICADA_PROGRESS_FILE` to the job's progress path in
the container's environment, read that file at each poll while the container runs and once in the turn that delivers its
end (the turn that finishes the collection when the actor has an `OutputStore`), and send `ContainerProgress { key, seq,
text }` to the end's target as `progressType` whenever the text changed, with `seq` counting from 1. A missing,
oversized or unreadable file SHALL send nothing and SHALL not count as a runtime error.

#### Scenario: Three updates

- **WHEN** a running container's program writes three different progress texts over several polls
- **THEN** the owner gets three progress messages with `seq` 1, 2 and 3, and none for a poll where the text did not
  change

#### Scenario: The last progress before the end

- **WHEN** the program writes progress and exits before the next poll
- **THEN** that progress message is sent in the turn that records the end (with a store, the turn that finishes the
  collection), before the end message

#### Scenario: A NUL in the file

- **WHEN** the progress file holds a NUL byte
- **THEN** the message's text has U+FFFD in its place and is sent

#### Scenario: A progress path the runtime refuses

- **WHEN** `Start` names a `progressType` and the job's `PROGRESS_FILE` is a relative path
- **THEN** the container ends `failed` with the kind `notStarted` and nothing is made

#### Scenario: No progress asked for

- **WHEN** `Start` has no `progressType`
- **THEN** the actor sets no `CICADA_PROGRESS_FILE` and reads no file
