## ADDED Requirements

### Requirement: Logs can be read in pages

`logs_since(name, cursor, max_lines)` SHALL return up to `max_lines` log lines after `cursor` (from the first line for
none), each with its time, its stream (out or err) and its text as lossy UTF-8 without the newline, and a cursor to
read on from. Reading with the same cursor again SHALL give the same lines first. Paging to the end SHALL give every
line once, in log order. `max_lines` outside 1 to 1000 SHALL fail with `InvalidSpec`.

#### Scenario: Paging through a log

- **WHEN** a container wrote 50 lines on stdout and stderr in one second, and `logs_since` is called from no cursor
  with `max_lines` 7, each time with the `next` of the page before, until a page has fewer than 7 lines
- **THEN** every line comes back once, in log order, with its stream

#### Scenario: Following a running container

- **WHEN** a host calls `logs_since` with the last `next` while the container writes more lines
- **THEN** each call returns only lines it has not returned before, and an empty page keeps the cursor

#### Scenario: A repeated cursor

- **WHEN** `logs_since` is called twice with the same cursor and `max_lines`, and nothing was written in between
- **THEN** both calls return the same page

#### Scenario: Containers that have no lines to give

- **WHEN** `logs_since` is called for a missing container, and for a created one
- **THEN** the missing one fails with `Conflict`, and the created one gives an empty page

#### Scenario: A page size out of bounds

- **WHEN** `logs_since` is called with `max_lines` 0 or 1001
- **THEN** it fails with `InvalidSpec` before it reads anything

### Requirement: A log cursor is an opaque string

`LogCursor` SHALL be written and read as a string, by `Display` and `FromStr` and by serde, with a JSON schema of a
string, and its form SHALL stay readable across the patch releases of a minor version. A string that is not a
cursor SHALL fail with `InvalidSpec`. A cursor SHALL mean the same on every runtime of the crate.

#### Scenario: A stored cursor

- **WHEN** a host writes `next` to JSON, reads it back and calls `logs_since` with it
- **THEN** the page is the one the original cursor gives

### Requirement: Adding a method keeps other runtimes compiling

`logs_since` SHALL have a default implementation that fails with `Other`, so an implementation of `ContainerRuntime`
outside this repo compiles unchanged. The fake and the Docker backend SHALL implement it.

#### Scenario: A runtime written before logs_since

- **WHEN** a host's own runtime implements every method but `logs_since`, and `logs_since` is called on it
- **THEN** it compiles, and the call fails with `Other`

## MODIFIED Requirements

### Requirement: A fake runtime for tests

The feature `testing` SHALL provide `FakeRuntime`, an in-memory `ContainerRuntime` that keeps containers, networks
and volumes by name with their labels and states and follows the repeat rules above. A test SHALL be able to end a
running container with a chosen status, set its logs or add timed log lines, paged as on Docker, make the next or
every call of a method fail with a chosen kind and without effect, and read every call in order. The fake SHALL use
no timer, so it runs under paused time.

#### Scenario: Scripted exit

- **WHEN** a test creates and starts `job-1` on the fake and then ends it with exit code 3
- **THEN** `inspect("job-1")` returns `Exited` with code 3 and `calls()` lists `create`, `start` and `inspect` in order

#### Scenario: Scripted failure

- **WHEN** a test makes the next `create` fail with `DaemonUnavailable`
- **THEN** that `create` fails with that kind, no container exists, and the following `create` succeeds

#### Scenario: Scripted log lines

- **WHEN** a test adds three lines to `job-1` at the same time, two on out and one on err, and calls `logs_since`
  with `max_lines` 2 and then with its `next`
- **THEN** the first page holds the first two lines, the second the third, and `logs` returns all three texts
