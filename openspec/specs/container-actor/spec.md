# container-actor Specification

## Purpose
`ContainerActor` in `cicada-containers` runs a container from durable actors by reconciliation: messages set a goal,
one armed poll takes one step toward it, and the end is reported once, before removal (ADR 0057).

## Requirements

### Requirement: A container actor under the host's type name

`cicada-containers` SHALL provide a container actor hosted as `ContainerActor::new(job, runtime, settings)`, whose type
name is the host's `ContainerJob::TYPE`. Its state SHALL hold the host's typed request and never a `ContainerSpec`;
the job SHALL build the spec from the request at each create. The container's name SHALL be the settings' prefix
followed by the actor's key.

#### Scenario: The spec is built, not stored

- **WHEN** a container actor with the key `j1` and the prefix `app-` starts, and its job puts a secret variable in the
  spec
- **THEN** the runtime creates `app-j1` with that variable, and the actor's stored state holds no part of the spec

#### Scenario: A key that cannot name a container

- **WHEN** a container actor whose key holds a `/` is started
- **THEN** it ends `failed` with a reason that names the key, and makes no runtime call

### Requirement: Messages set a goal and one armed poll reconciles it

`Start`, `Stop` and `Remove` SHALL only set the goal and arm a poll. The goal SHALL move only forward from run to stop
to remove. Only the armed poll, sent by the actor itself, SHALL act: it SHALL take one step toward the goal from what
the runtime reports, save it and arm the next poll. A step that runs again SHALL find the container by its name.

#### Scenario: A repeated step starts one container

- **WHEN** the runtime already holds the running container that a step made before its commit was lost, and the poll
  runs that step again
- **THEN** one container with that name exists and it is not started a second time

#### Scenario: Stop before start

- **WHEN** an actor gets `Stop` and then `Start`
- **THEN** no container is created, the poll records the end `stopped` without counting an error, and the `Start`
  sends that end to its target

#### Scenario: Remove with no start

- **WHEN** an actor that never got `Start` gets `Remove`
- **THEN** it ends `stopped` and removes nothing that exists, without counting an error

#### Scenario: A missing image

- **WHEN** `create` fails with `ImageNotFound`
- **THEN** that poll records the end `failed` with the reason, with no backoff and no further create

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

### Requirement: A lost poll comes back

`Check`, and a goal asked again, SHALL replace the armed poll with a fresh one due when the old one was due, or at
once when that time has passed. With no work left they SHALL change nothing; a removed container whose outputs wait for
deletion has work left.

#### Scenario: A purged poll

- **WHEN** the armed poll of a running container is purged and the actor gets `Check`
- **THEN** a fresh poll is armed and the container is followed to its end

#### Scenario: A lost retention poll

- **WHEN** the armed poll of a removed container whose outputs wait for deletion is purged and the actor gets `Check`
- **THEN** a fresh poll is armed and the outputs are deleted at the saved time

### Requirement: Stored text is plain and bounded

Log text and reasons SHALL be stored with every NUL replaced by U+FFFD. Logs SHALL keep at most the settings' last
lines and last bytes, cut on a character boundary; a reason SHALL keep at most 1 KiB.

#### Scenario: A NUL in the logs

- **WHEN** an exited container's logs hold a NUL and more bytes than `log_bytes`
- **THEN** the recorded logs hold U+FFFD in its place and end with the last `log_bytes` bytes or fewer

### Requirement: Inputs are copied in before start

`ContainerJob::inputs(name, request)` SHALL give the folders and files of a container, none by default, built at each
create and never stored. With inputs, the create step SHALL make the folders and then write the files only while the
container is `Created`, then start it. Without inputs, its runtime calls SHALL be `inspect`, `create` and `start`.

#### Scenario: Inputs before start

- **WHEN** a job gives the folder `/out` and the file `/in/config.json`, and the actor is started
- **THEN** the runtime gets `inspect`, `create`, `make_dirs` with `/out`, `copy_in` with the file, and `start`, in
  that order, and the stored state holds no part of the inputs

#### Scenario: No inputs

- **WHEN** a job gives no inputs, and the actor is started
- **THEN** the runtime gets `inspect`, `create` and `start`

#### Scenario: A replayed step after start

- **WHEN** the runtime already holds the running container that a step made and started before its commit was lost,
  and the poll runs that step again
- **THEN** no `make_dirs` or `copy_in` is called and the container is not started a second time

### Requirement: Refused inputs end the container

An `Err` from `inputs`, a file at or above a folder (paths compared after the shared path rule's parsing), or a path
that parsing refuses SHALL end the container `failed` with the kind `notStarted` before anything is made. `InvalidSpec`
from `make_dirs` or `copy_in`, `Conflict` from either while a new `inspect` still finds the container `Created`, and
`Unsupported` from `make_dirs` SHALL end it `failed` with the kind `notStarted`. Other errors, `Other` too, SHALL count
and back off, and the next poll SHALL run the step again.

#### Scenario: Inputs that cannot be built

- **WHEN** a job's `inputs` returns an `Err`; or the folder `/out` and the file `/out`; or the folder `/a/b` and the
  file `/a`; or the folder `/out/.` and the file `/out`
- **THEN** each time the poll ends the container `failed` with the reason, and `create` is not called

#### Scenario: A refused input

- **WHEN** `copy_in` fails with `InvalidSpec`
- **THEN** the poll ends the container `failed` with the reason, and it is not started

#### Scenario: A conflict while copying

- **WHEN** `make_dirs` fails with `Conflict`, and the next `inspect` reports the container `Created`; and in another
  run reports it `Missing`
- **THEN** the first ends the container `failed` with the reason; the second counts an error and backs off, and the
  next poll creates the container again, copies and starts it

#### Scenario: A container gone after create

- **WHEN** the container is removed between `create` and the copies, so `make_dirs` fails with `Conflict` and the new
  `inspect` reports `Missing`
- **THEN** the error counts and backs off, and the next poll creates the container again, copies and starts it

#### Scenario: A runtime without make_dirs

- **WHEN** a job gives a folder, and the runtime's `make_dirs` is the trait's default, which fails with `Unsupported`
- **THEN** the poll ends the container `failed` with the kind `notStarted` at once, without backing off

#### Scenario: Another error from make_dirs

- **WHEN** `make_dirs` fails with `Other` once
- **THEN** the error counts and backs off, and the next poll makes the folders, writes the files and starts the
  container

#### Scenario: An unavailable daemon while copying

- **WHEN** `make_dirs` fails with `DaemonUnavailable` once
- **THEN** the error counts and backs off, and the next poll makes the folders, writes the files and starts the
  container

### Requirement: A replayed create step adopts a started container

The create step SHALL begin with `inspect`. A container of that name past `Created` SHALL be adopted: the step SHALL
save it as started with a fresh deadline and arm the next poll, without calling `spec`, `inputs`, `create`, the copies
or `start`. A missing or created container SHALL go through the step as before.

#### Scenario: A lost commit after start

- **WHEN** the runtime holds the running container that a step started before its commit was lost, the job's `spec`
  and `inputs` now return an `Err`, and the poll runs the step again
- **THEN** the actor follows the container, calls neither `spec` nor `inputs`, and records no `notStarted` end

### Requirement: A container job sets the turn limit of its actor

`ContainerJob` SHALL have `const MAX_TURN: Option<Duration>` with the default `None`, and `ContainerActor<J>` SHALL use
it as its `Actor::MAX_TURN`, so it wins over the node's default turn limit; `None` SHALL keep the node's default.

#### Scenario: A slow pull fits

- **WHEN** a job sets `MAX_TURN` to 35 minutes, the node's default is 15 minutes, and the create step of a container
  takes 20 minutes
- **THEN** the step ends `done` and the container starts

#### Scenario: No limit set

- **WHEN** a job does not set `MAX_TURN`
- **THEN** its actor uses the node's default turn limit

#### Scenario: A limit below the create step

- **WHEN** a job sets `MAX_TURN` to 1 minute and the create step takes 2 minutes
- **THEN** the step ends `failed` with the cause `timeout`, its retry runs the step again, and no second container is
  made

#### Scenario: A zero limit

- **WHEN** a host registers a container job whose `MAX_TURN` is zero
- **THEN** the registration is refused with "MAX_TURN must be positive"

### Requirement: The actor labels its containers with its prefix

`ContainerActor` SHALL add the label `cicada.container.prefix` with the settings' prefix to the spec of every container
it creates, after the job's `spec`, so a job cannot remove it.

#### Scenario: A job with its own labels

- **WHEN** a job's spec sets the label `team=ci` and the actor creates the container
- **THEN** the container carries `team=ci` and `cicada.container.prefix` with the prefix

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

### Requirement: Stored outputs are deleted after a retention from the removal

When the actor has an `OutputStore` and `ContainerSettings::output_retention` is set, the step that removes the
container SHALL save the due time in the state and arm a poll for it, and that poll SHALL call `OutputStore::remove`
for the container and clear the due time. A retryable store error SHALL be retried with the backoff without limit,
also after a restart; the saved due time SHALL decide even when the settings change later.

#### Scenario: A week after removal

- **WHEN** the retention is 7 days and a container with stored outputs is removed
- **THEN** its outputs are still there after 6 days and gone after 7

#### Scenario: No retention

- **WHEN** `output_retention` is not set
- **THEN** the outputs of a removed container stay

#### Scenario: A store that is down at the due time

- **WHEN** the store fails with `Unavailable` at the due time for three tries and then answers
- **THEN** the outputs are deleted at the fourth try and the due time is cleared

#### Scenario: A store that refuses the name

- **WHEN** the store answers `Invalid` at the due time
- **THEN** the due time is cleared with a warning and nothing is retried

### Requirement: A collection that keeps timing out still ends

A collection turn whose step message has failed 3 or more times SHALL make no copy or log read, and SHALL record and
send the pending end with `outputs.error` naming the failures and the last error, and the counts of what is stored.

#### Scenario: A copy longer than the turn limit

- **WHEN** a container's output copy takes longer than the job's `MAX_TURN` on every try
- **THEN** after three timed-out tries the end is sent with the error in `outputs`, and no fourth copy is made
