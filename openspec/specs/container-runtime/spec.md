# container-runtime Specification

## Purpose
The optional crate `cicada-containers` gives a container runtime trait in which every call is safe to repeat, its
types and errors, and a fake for tests, so applications that run containers from actors share one layer (ADR 0056).

## Requirements

### Requirement: A container runtime trait outside the core

The workspace SHALL provide the crate `cicada-containers` with a `ContainerRuntime` trait, the types `ContainerSpec`,
`ContainerStatus`, `NetworkSpec`, `VolumeSpec`, `CopyFile` and `Label`, and one error type `ContainerError`. The trait
SHALL be usable as a trait object (`Arc<dyn ContainerRuntime>`). The trait and its types SHALL name no type of a
Docker client library. No other library crate of the workspace SHALL depend on `cicada-containers`; only
examples and tests may (ADR 0056).

#### Scenario: A host picks its runtime at run time

- **WHEN** a host holds an `Arc<dyn ContainerRuntime>` that is either the fake or another backend
- **THEN** it calls every method of the trait through that value

#### Scenario: The library crates stay free of the crate

- **WHEN** the gate checks the dependency trees of `cicada`, `cicada-test`, `cicada-axum`, `cicada-client`,
  `cicada-otel`, `cicada-actors` and `cicada-app`
- **THEN** none of them holds `cicada-containers`

### Requirement: Every container call is safe to repeat

Each container method of `ContainerRuntime` SHALL give the same end state when it is called again with the same
arguments. `create` SHALL find an existing container by name and return `Ok` without replacing it. `start` SHALL
return `Ok` and do nothing for a running, exited or failed container, so a container runs at most once. `stop` SHALL
return `Ok` and leave a created container `Created`. `stop` and `remove` SHALL return `Ok` for a missing container.

#### Scenario: A repeated create keeps the first container

- **WHEN** `create` is called twice with specs of the same name
- **THEN** both calls return `Ok` and one container with that name exists, made from the first spec

#### Scenario: A repeated start does not run an exited container again

- **WHEN** `start` is called for a container that has exited
- **THEN** it returns `Ok` and `inspect` still reports the same exit

#### Scenario: Removing what is gone

- **WHEN** `stop` or `remove` is called for a name that has no container
- **THEN** it returns `Ok`

#### Scenario: Stopping a running container

- **WHEN** `stop` is called for a running container
- **THEN** it returns `Ok` and `inspect` reports `Exited` with the code the runtime gave the stopped process

### Requirement: Networks, volumes and sweeps are safe to repeat

`ensure_network` and `ensure_volume` SHALL find an existing network or volume by name and return `Ok`. `sweep` SHALL
remove every container, then every network, then every volume that has the given label, and SHALL return `Ok` when
nothing has it. A network or volume that is still in use SHALL be skipped with its reason while the sweep goes on;
only an unavailable daemon SHALL end a sweep with an error. Its result SHALL name nothing already gone or being
removed by another caller.

#### Scenario: A repeated ensure keeps one network

- **WHEN** `ensure_network` is called twice with the same internal network spec
- **THEN** both calls return `Ok` and one network with that name exists

#### Scenario: A sweep removes by label

- **WHEN** containers, networks and volumes carry the label `run=r1` and others carry `run=r2`, and `sweep` is called
  with `run=r1`
- **THEN** the ones labelled `run=r1` are gone, the others remain, and the result names what was removed

#### Scenario: A sweep skips a network in use

- **WHEN** a network labelled `run=r1` is used by a container without that label, and `sweep` is called with `run=r1`
- **THEN** it returns `Ok`, the network remains, and the result names it as skipped with its reason

#### Scenario: Something is gone before the sweep removes it

- **WHEN** a container, a network or a volume labelled `run=r1` is listed by `sweep` and removed by another caller
  before the sweep removes it, or a container is still being removed by another caller (Docker's 409)
- **THEN** the sweep goes on and returns `Ok`, and that name is neither in its kind's removed list nor in `skipped`

### Requirement: A container's status tells its own exit from the runtime's

`inspect` SHALL report a container as `Missing`, `Created`, `Running`, `Exited` with the program's exit code, or
`Failed` with an optional exit code and a reason when the container did not start, ran out of memory or was lost by
the runtime. A container ended by `stop` SHALL be reported as `Exited`.

#### Scenario: Inspecting a container that never existed

- **WHEN** `inspect` is called for a name that has no container
- **THEN** it returns `Missing`, not an error

### Requirement: Files are copied only before start

`copy_in` SHALL write the given files into a `Created` container, and SHALL fail with `Conflict` for a container
that is missing, running or ended. Every runtime SHALL fail with `InvalidSpec`, before any write, for a path that is
relative, has a `..` part, names no file once empty and `.` parts are dropped, equals the target of a volume or
bind mount of the container, or lies at or below a tmpfs target with no deeper volume or bind mount.

#### Scenario: Copying into a running container

- **WHEN** `copy_in` is called for a running container
- **THEN** it fails with `Conflict` and no file is written

#### Scenario: Copying onto a mount target

- **WHEN** a created container has a volume at `/work`, and `copy_in` writes `/work` and `/etc/b`
- **THEN** it fails with `InvalidSpec` and writes nothing, on the fake and on Docker, with a read-only or a writable
  root

#### Scenario: Copying onto a tmpfs of a writable root

- **WHEN** a created container has a writable root and a tmpfs at `/scratch`, and `copy_in` writes `/scratch/c`
- **THEN** it fails with `InvalidSpec` and writes nothing, on the fake and on Docker

#### Scenario: One file under two spellings

- **WHEN** `copy_in` writes `/work/./a` and then `/work/a` with other contents
- **THEN** the container holds one file `/work/a`, with the second contents

#### Scenario: Copying to the root path

- **WHEN** `copy_in` writes a file at `/`
- **THEN** it fails with `InvalidSpec` and writes nothing

### Requirement: Errors say whether a retry can help

Every method SHALL fail only with a `ContainerError` whose kind is `DaemonUnavailable`, `Conflict`, `ImageNotFound`,
`InvalidSpec` or `Other`. `is_retryable()` SHALL be true only for `DaemonUnavailable`. A call that does not fit the
current state SHALL fail with `Conflict`: `start`, `copy_in` or `logs` on a missing container, and `ensure_network` on
an existing network whose `internal` flag differs.

#### Scenario: Starting a missing container

- **WHEN** `start` is called for a name that has no container
- **THEN** it fails with kind `Conflict` and `is_retryable()` is false

### Requirement: Secret environment stays hidden

`ContainerSpec` SHALL keep secret environment apart from plain environment. Its `Debug` output SHALL show the names
of the secret variables and never their values. A backend SHALL never pass a secret value on a command line.

#### Scenario: Debug output of a spec with a secret

- **WHEN** a spec with the secret variable `TOKEN=s3cret` is formatted with `{:?}`
- **THEN** the output contains `TOKEN` and does not contain `s3cret`

### Requirement: A spec can grow without breaking callers

`ContainerSpec` SHALL be constructed by `ContainerSpec::new(name, image)` with every other field at its default, and
SHALL be `#[non_exhaustive]` with public fields. The hardening fields (read-only root file system, drop all
capabilities, no new privileges, tmpfs mounts, an init process, a process limit) SHALL be off by default.

#### Scenario: A new spec is plain

- **WHEN** a caller makes `ContainerSpec::new("job-1", "alpine:3")`
- **THEN** it has no command override, no environment, no mounts, the default network and no hardening

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

### Requirement: Files can be copied out within limits

`copy_out(name, path, limits)` SHALL return the regular files at or below `path` of a created, running or ended
container, each whole with its absolute path and mode, or name it in `skipped` with a reason. A file larger than the
bytes left SHALL be skipped and the copy go on. An entry after the first `max_files` SHALL stop the copy and add one
skipped entry for the rest. A zero limit SHALL be `InvalidSpec`, a missing path an empty result, a missing container
`Conflict`.

#### Scenario: Collecting a folder

- **WHEN** an ended container wrote `/out/a` and `/out/sub/b`, and `copy_out("/out")` is called with room for both
- **THEN** it returns both files with their contents and modes, and nothing skipped

#### Scenario: One file too big

- **WHEN** a folder holds files of 10, 20 and 5000 bytes, and `copy_out` is called with `max_bytes` 100
- **THEN** it returns the two small files whole and names the big one in `skipped` with its size

#### Scenario: Too many files

- **WHEN** a folder holds five files, and `copy_out` is called with `max_files` 2
- **THEN** it returns two entries and one more skipped entry, for the folder, saying the rest was not read

#### Scenario: Exactly at the file limit

- **WHEN** a folder holds two files, and `copy_out` is called with `max_files` 2 and room for both
- **THEN** it returns both files and nothing skipped

#### Scenario: A zero limit

- **WHEN** `copy_out` is called with `max_files` 0 or `max_bytes` 0
- **THEN** it fails with `InvalidSpec` before it reads anything

#### Scenario: What is not there

- **WHEN** `copy_out` is called for a path that does not exist, for a missing container, and with a relative path
- **THEN** the first gives an empty result, the second fails with `Conflict`, and the third with `InvalidSpec`

### Requirement: A copy out reads only regular files

`copy_out` SHALL copy only regular files. It SHALL follow no symbolic link, inside the folder or out of it, and SHALL
skip with a reason every link, device, pipe or socket, and every entry whose name is not under `path`.

#### Scenario: Links in the folder

- **WHEN** a folder holds a file `a`, a link to `a` and a link to `/etc/passwd`
- **THEN** `copy_out` returns `a` and names both links in `skipped`

### Requirement: copy_out keeps other runtimes compiling

`copy_out` SHALL have a default implementation that fails with `Other`, so an implementation of `ContainerRuntime`
outside this repo compiles unchanged. The fake and the Docker backend SHALL implement it.

#### Scenario: A runtime written before copy_out

- **WHEN** a host's own runtime implements every method but `copy_out`, and `copy_out` is called on it
- **THEN** it compiles, and the call fails with `Other`

### Requirement: The fake holds files to copy out

`FakeRuntime` SHALL return from `copy_out` the files that `copy_in` wrote and those a test added with `put_file`, in
the order Docker's archive walks a folder, under the same limits and skip rules as the Docker backend. On an ended
container it SHALL leave out files written to a tmpfs.

#### Scenario: A file the program wrote

- **WHEN** a test adds `/out/report.txt` to `job-1` with `put_file` and calls `copy_out("job-1", "/out", …)`
- **THEN** it returns that file with its contents and mode

#### Scenario: A tmpfs after the end

- **WHEN** `job-1` has a tmpfs at `/scratch`, a test adds `/scratch/a` and ends the container, and calls
  `copy_out("job-1", "/scratch", …)`
- **THEN** it returns an empty result
