## ADDED Requirements

### Requirement: A log archiver keeps every container's log in the store

`LogArchiver::run` SHALL, at each interval until stopped, follow the log of every existing container that carries the
job's prefix label and append each page to the `OutputStore` with its cursor, so that every line is stored once and in
order, across restarts and with several archivers on one store, except lines written after its last read by a
container removed before the next round. An ended container SHALL be drained once and then skipped.

#### Scenario: A running container

- **WHEN** a container writes lines over several intervals while the archiver runs
- **THEN** the store holds every line once, in order, and its cursor is the last page's

#### Scenario: A restart

- **WHEN** an archiver stops after storing part of a log and a new one starts on the same store
- **THEN** the new one stores only the lines after the stored cursor

#### Scenario: Two archivers

- **WHEN** two archivers on one store follow the same container
- **THEN** every line is stored once

### Requirement: A slow log holds up only itself

A round SHALL not wait for its reads, and a container SHALL not be read twice at once.

#### Scenario: A slow log

- **WHEN** one container's log read hangs while other containers write lines
- **THEN** the others' lines arrive each round, and the hanging container is not read again until its read ends

#### Scenario: A container removed between rounds

- **WHEN** a container writes lines, ends and is removed between two rounds
- **THEN** the lines up to the archiver's last read are stored, and later lines are not

### Requirement: The archiver keeps going through errors

The archiver SHALL mark a container whose log is `Unsupported` as drained, drop a container that is gone, retry every
other error kind and a failed `list` in later rounds with a growing wait after 5 failures in a row, and SHALL not end
`run` for any error.

#### Scenario: A daemon that comes back

- **WHEN** `logs_since` fails with `DaemonUnavailable` for two rounds and then answers
- **THEN** the lines arrive in the store after the third round

#### Scenario: A log that can never be read

- **WHEN** `logs_since` fails with `Unsupported` for a container
- **THEN** the container is marked drained and not read again

#### Scenario: A failed list

- **WHEN** `list` fails with `DaemonUnavailable` for a round
- **THEN** the archiver goes on, and the next round lists again
