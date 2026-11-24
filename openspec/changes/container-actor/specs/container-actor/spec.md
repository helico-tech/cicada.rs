## ADDED Requirements

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
or `stopped`. The commit that records the end SHALL save it in the state with the tail of the logs and SHALL send
`{key, end}` with the message type to the target named in the first `Start`. No step SHALL remove a container before
its end is recorded.

#### Scenario: An exit is reported with its logs

- **WHEN** a started container exits with code 3 and logs `done`
- **THEN** one commit records the end `exited` with code 3 and logs `done`, and sends it to the target

#### Scenario: Remove after the exit

- **WHEN** an exited container whose end is recorded gets `Remove`
- **THEN** the runtime removes it and the actor sends no second end

#### Scenario: An unknown target type

- **WHEN** `Start` names a target type that is not registered, and the container exits and then gets `Remove`
- **THEN** the end is saved as undelivered with the reason, no poll fails, and the container is removed

### Requirement: A deadline and errors in a row end a container

A container that runs past the settings' timeout SHALL be stopped and end `timedOut`. A runtime error SHALL count in
a row and delay the next poll by a growing backoff up to the settings' maximum; it SHALL never fail the turn. At the
settings' maximum errors in a row, a container without an end SHALL end `failed`; a removal SHALL keep trying.

#### Scenario: The deadline passes

- **WHEN** a container with a timeout of 60 s still runs after 60 s of virtual time
- **THEN** the actor stops it and records the end `timedOut`

#### Scenario: Too many errors in a row

- **WHEN** every `inspect` of a started container fails with `DaemonUnavailable` and `max_errors` is 3
- **THEN** the polls wait longer after each error, and the third error records the end `failed` with the last error

### Requirement: A lost poll comes back

`Check`, and a goal asked again, SHALL replace the armed poll with a fresh one due when the old one was due, or at
once when that time has passed. With no work left they SHALL change nothing.

#### Scenario: A purged poll

- **WHEN** the armed poll of a running container is purged and the actor gets `Check`
- **THEN** a fresh poll is armed and the container is followed to its end

### Requirement: Stored text is plain and bounded

Log text and reasons SHALL be stored with every NUL replaced by U+FFFD. Logs SHALL keep at most the settings' last
lines and last bytes, cut on a character boundary; a reason SHALL keep at most 1 KiB.

#### Scenario: A NUL in the logs

- **WHEN** an exited container's logs hold a NUL and more bytes than `log_bytes`
- **THEN** the recorded logs hold U+FFFD in its place and end with the last `log_bytes` bytes or fewer
