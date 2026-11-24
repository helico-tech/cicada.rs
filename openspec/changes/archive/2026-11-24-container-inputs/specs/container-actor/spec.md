## ADDED Requirements

### Requirement: Inputs are copied in before start

`ContainerJob::inputs(name, request)` SHALL give the folders and files of a container, none by default, built at each
create and never stored. With inputs, the create step SHALL make the folders and then write the files only while the
container is `Created`, then start it. Without inputs, its runtime calls SHALL be those of a step before this change.

#### Scenario: Inputs before start

- **WHEN** a job gives the folder `/out` and the file `/in/config.json`, and the actor is started
- **THEN** the runtime gets `create`, `inspect`, `make_dirs` with `/out`, `copy_in` with the file, and `start`, in
  that order, and the stored state holds no part of the inputs

#### Scenario: No inputs

- **WHEN** a job gives no inputs, and the actor is started
- **THEN** the runtime gets `create` and `start` only, as before

#### Scenario: A replayed step after start

- **WHEN** the runtime already holds the running container that a step made and started before its commit was lost,
  and the poll runs that step again
- **THEN** no `make_dirs` or `copy_in` is called and the container is not started a second time

### Requirement: Refused inputs end the container

An `Err` from `inputs`, a file at or above a folder (paths compared after the shared path rule's parsing), or a path
that parsing refuses SHALL end the container `failed` before anything is made. `InvalidSpec` from `make_dirs` or
`copy_in`, `Conflict` from either while a new `inspect` still finds the container `Created`, and `Other` from
`make_dirs` SHALL end it `failed`. Other errors SHALL count and back off, and the next poll SHALL run the step again.

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

#### Scenario: A runtime without make_dirs

- **WHEN** a job gives a folder, and the runtime's `make_dirs` is the trait's default, which fails with `Other`
- **THEN** the poll ends the container `failed` at once, without backing off

#### Scenario: A container gone after create

- **WHEN** `inspect` reports `Missing` right after `create`
- **THEN** the error counts and backs off, and the next poll creates the container again, copies and starts it

#### Scenario: An unavailable daemon while copying

- **WHEN** `make_dirs` fails with `DaemonUnavailable` once
- **THEN** the error counts and backs off, and the next poll makes the folders, writes the files and starts the
  container
