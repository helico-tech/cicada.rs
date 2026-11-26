# container-outputs Specification

## Purpose
A host keeps a container's log and output files outside the container, in an `OutputStore` it provides, so they
survive the container's removal and can be served to users; `cicada-containers` defines the store's rules and checks a
host's implementation against them.

## Requirements

### Requirement: A host provides a store for container outputs

`cicada-containers` SHALL define `OutputStore`, an async trait keyed by container name, through which a container's
log lines, its log cursor and its output files are stored, read back and removed, and SHALL provide an in-memory
implementation and a conformance check of the trait behind the feature `testing`.

#### Scenario: A host's own store

- **WHEN** a host runs `check_output_store` against its own implementation
- **THEN** the check passes for a store that keeps the rules, and panics naming the rule for one that does not

### Requirement: Log lines and their cursor are stored in one guarded write

`append_logs(container, after, page)` SHALL store the page's lines and its cursor together or not at all, and only
when the stored cursor equals `after`; otherwise it SHALL fail with `Conflict` and store nothing.

#### Scenario: A crash after a write

- **WHEN** an archiver appends a page, crashes before it records that, reads the stored cursor and appends from there
- **THEN** every line is stored once

#### Scenario: A stale writer

- **WHEN** `append_logs` is called with an `after` that is not the stored cursor
- **THEN** it fails with `Conflict` and the stored lines and cursor are unchanged

### Requirement: Stored lines are read back by line number

`read_logs(container, from, limit)` SHALL return up to `limit` stored lines from line number `from`, in stored order,
each with its number, and none past the end.

#### Scenario: Paging for an HTTP route

- **WHEN** a container has 25 stored lines and a host reads from 0, 10 and 20 with a limit of 10
- **THEN** it gets lines 0 to 9, 10 to 19 and 20 to 24, each once

### Requirement: A store refuses names and paths outside the rules

Every `OutputStore` method SHALL fail with `Invalid` for a container name outside the container actor's name rule or
over 200 characters, and for a file path that is relative, has a `..` part, is `/` or holds a control character (NUL
included) in a part, before it touches anything, and SHALL treat paths that
differ only in empty and `.` parts as one file.

#### Scenario: A path from a user

- **WHEN** a host's route asks `read_file` for the path `/out/../../etc/passwd`
- **THEN** it fails with `Invalid` and reads nothing

#### Scenario: A NUL in a path

- **WHEN** a host asks `read_file` for a path with a NUL in it
- **THEN** it fails with `Invalid`

### Requirement: Files are stored, listed, read and removed per container

`put_file` SHALL store a file under its container, replacing one at the same path; `files` SHALL list the stored files
with their sizes and modes in path order; `read_file` SHALL give a stored file or none; `remove` SHALL delete a
container's lines, cursor and files, and SHALL succeed when nothing is stored. `log_end` SHALL give the stored cursor
and the number of stored lines.

#### Scenario: A file replaced and removed

- **WHEN** a host puts `/out/a.txt` twice with other contents, lists the files, then removes the container
- **THEN** the list holds one `/out/a.txt` with the second contents, and afterwards `files` is empty and `log_end`
  gives no cursor and 0 lines

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

### Requirement: Containers without their actor are reported

`OrphanWatch::run` SHALL, at each interval until stopped, list the containers with the job's prefix label, and SHALL
call `on_orphan` once for each container whose container actor was not found in two rounds in a row, without removing
it. A failed list or actor read SHALL be retried in the next round and SHALL not count as a round without the actor.

#### Scenario: A container left without its actor

- **WHEN** a labelled container runs and no actor exists for its name, over three rounds
- **THEN** `on_orphan` is called once with its name, after the second round, and the container still runs

#### Scenario: An actor that appears

- **WHEN** an orphan was reported and its actor is then created
- **THEN** it is dropped from the set and not reported again while its actor exists

#### Scenario: A removed orphan

- **WHEN** a reported orphan is removed from the runtime
- **THEN** it leaves the set, and a new container of that name starts unreported

#### Scenario: A failed list

- **WHEN** `list` fails with `DaemonUnavailable` in one round
- **THEN** the watch goes on, and that round does not count toward an orphan
