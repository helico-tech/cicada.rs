## ADDED Requirements

### Requirement: Inputs are copied in before start

`ContainerJob::inputs(name, request)` SHALL give the folders and files of a container, none by default, built at each
create and never stored. With inputs, the create step SHALL make the folders and then write the files only while the
container is `Created`, then start it. An `Err`, or a file at a folder's path, SHALL end it `failed` before anything
is made; `InvalidSpec`, or `Conflict` from `make_dirs`, SHALL end it `failed`; other errors SHALL back off.

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

#### Scenario: Inputs that cannot be built

- **WHEN** a job's `inputs` returns an `Err`, or a file whose path is also one of its folders
- **THEN** the poll ends the container `failed` with the reason, and `create` is not called

#### Scenario: A refused input

- **WHEN** `copy_in` fails with `InvalidSpec`, or `make_dirs` with `Conflict` for a file where a folder was asked
- **THEN** the poll ends the container `failed` with the reason, and it is not started

#### Scenario: A container gone after create

- **WHEN** `inspect` reports `Missing` right after `create`
- **THEN** the error counts and backs off, and the next poll creates the container again, copies and starts it

#### Scenario: An unavailable daemon while copying

- **WHEN** `make_dirs` fails with `DaemonUnavailable` once
- **THEN** the error counts and backs off, and the next poll makes the folders, writes the files and starts the
  container
