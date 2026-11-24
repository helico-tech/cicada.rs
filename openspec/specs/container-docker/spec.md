# container-docker Specification

## Purpose
The `docker` feature of `cicada-containers` gives `DockerRuntime`, a backend of the container runtime trait over
bollard that keeps every repeat rule against a real Docker daemon and is tested in the E2E suite.

## Requirements

### Requirement: A Docker backend behind a feature

`cicada-containers` SHALL provide `DockerRuntime`, a `ContainerRuntime` over the Docker Engine API, behind the
feature `docker`. `DockerRuntime::connect()` SHALL use `DOCKER_HOST` when it is set and the local Docker socket
otherwise, without TLS, and SHALL not contact the daemon. No public item SHALL name a type of the Docker client
library. The crate without features SHALL not build that library.

#### Scenario: The default build has no Docker client

- **WHEN** the gate checks the dependency tree of `cicada-containers` without features
- **THEN** it holds no `bollard`, and with the feature `docker` it holds `bollard` and no TLS library

#### Scenario: No daemon

- **WHEN** a `DockerRuntime` points at a socket where no daemon listens and `inspect` is called
- **THEN** it fails with kind `DaemonUnavailable` and `is_retryable()` is true

### Requirement: The repeat rules hold against Docker

`DockerRuntime` SHALL follow every repeat rule of `container-runtime`. `create` SHALL return `Ok` when a container
with the name exists. `start` SHALL act only on a `Created` container. `stop` SHALL act only on a `Running`
container. `stop` and `remove` SHALL return `Ok` for a missing container. `ensure_network` and `ensure_volume` SHALL
return `Ok` for an existing network or volume of that name.

#### Scenario: An exited container is not run again

- **WHEN** a container ran and exited with code 3, and `start` is called again
- **THEN** it returns `Ok`, `inspect` still reports `Exited` with code 3, and the program ran once

#### Scenario: Stopping a running container

- **WHEN** `stop` is called for a running container whose program ends on SIGTERM
- **THEN** `inspect` reports `Exited` with code 143, and with code 137 when the program outlives the grace period

#### Scenario: A repeated create after the image is gone

- **WHEN** a container exists, its image is removed from the daemon, and `create` is called again with the same spec
- **THEN** it returns `Ok` without a pull, and the container is unchanged

### Requirement: Files reach a read-only container only on a mount

With `read_only_root`, `copy_in` SHALL write a file only when its path lies below the target of a volume or bind
mount of the container, and SHALL fail with `InvalidSpec` before any write when a path lies elsewhere, a tmpfs
included.

#### Scenario: Copying onto a volume of a read-only container

- **WHEN** a created container has `read_only_root` and a volume at `/work`, and `copy_in` writes `/work/a` and `/etc/b`
- **THEN** it fails with `InvalidSpec` and writes nothing; with `/work/a` alone it writes the file, read back after start

### Requirement: Containers are found by their exact name

Every container method of `DockerRuntime` SHALL treat a container as missing unless its name is exactly the given
name, and SHALL act on the container it found by that name only.

#### Scenario: An ID prefix is not a name

- **WHEN** `inspect` and then `remove` are called with the first 12 characters of an existing container's ID
- **THEN** `inspect` returns `Missing`, `remove` returns `Ok`, and that container still exists

### Requirement: A missing image is pulled

`create` SHALL pull the image when the daemon does not have it, and SHALL fail with `ImageNotFound` and create no
container when the registry does not have it.

#### Scenario: An unknown tag

- **WHEN** `create` is called with an image tag that no registry has
- **THEN** it fails with kind `ImageNotFound` and `inspect` of the name returns `Missing`

### Requirement: Docker's state maps onto the container status

`inspect` SHALL report Docker's `created` as `Created`, `running`, `paused` and `restarting` as `Running`, and
`exited` as `Exited` with Docker's exit code. It SHALL report `Failed` for a container killed for lack of memory
(reason "out of memory"), one with an error from the daemon (that error as reason), one whose start was refused,
and a `dead` one.

#### Scenario: Out of memory

- **WHEN** a container with a 32 MiB memory limit runs a program that keeps allocating
- **THEN** `inspect` reports `Failed` with the reason "out of memory"

#### Scenario: A refused start

- **WHEN** `start` is called for a container whose executable does not exist
- **THEN** it fails with `InvalidSpec`, `inspect` reports `Failed`, and a second `start` returns `Ok`

### Requirement: The spec's limits and hardening reach Docker

`create` SHALL pass the spec's command, environment, labels, user, mounts, networks, CPU and memory limits, and each
hardening field that is on (read-only root file system, all capabilities dropped, no new privileges, tmpfs mounts, an
init process, a process limit) to Docker. A memory limit SHALL allow no swap beyond it.

#### Scenario: A hardened container

- **WHEN** a container with every hardening field on writes to its root and to its tmpfs and prints its capabilities
- **THEN** the root write fails, the tmpfs write succeeds, and it has no effective capability and no new privileges

### Requirement: Secret environment stays out of messages

`DockerRuntime` SHALL send secret environment to Docker only in the body of the create request, SHALL log nothing
itself, and SHALL never put a secret value in a `ContainerError` message.

#### Scenario: A refused create with a secret

- **WHEN** Docker refuses a create whose message contains the value of a secret variable
- **THEN** the returned error's message shows `***` in place of that value

### Requirement: Docker's errors map onto error kinds

`DockerRuntime` SHALL map a failed connection or a timeout to `DaemonUnavailable`, Docker's 409 to `Conflict`, 400 to
`InvalidSpec`, an image Docker or the registry does not know to `ImageNotFound`, and every other failure to `Other`,
except where a repeat rule makes the call `Ok` or the trait asks for `Conflict` on a missing container.

#### Scenario: Copying into a running container

- **WHEN** `copy_in` is called for a running container
- **THEN** it fails with kind `Conflict` and no file is written

### Requirement: A sweep removes in order and skips what is in use

`sweep` SHALL remove the containers with the label, then the networks, then the volumes. A network or volume that
Docker refuses to remove SHALL be skipped with Docker's message while the sweep goes on; only `DaemonUnavailable`
SHALL end it with an error.

#### Scenario: A network still in use

- **WHEN** containers, a network and a volume carry `run=a`, a container without the label uses that network, others
  carry `run=b`, and `sweep` is called with `run=a`
- **THEN** the `run=a` containers and volume are gone, the network is skipped with its reason, and `run=b` remains

### Requirement: The Docker test runs only in the E2E kind

The test of `DockerRuntime` against a real daemon SHALL run in the `cicada-e2e` suite, only with `CICADA_E2E=1`, and
SHALL remove everything it made, also after a failure.

#### Scenario: A run without Docker

- **WHEN** `cargo test --workspace` runs without `CICADA_E2E=1`
- **THEN** the Docker test does not contact a daemon and the run passes

### Requirement: A list leaves out what it cannot report

`list` SHALL leave out a container that was removed after Docker listed it, and one whose state Docker does not
report, and SHALL return the others. `inspect`, `start`, `copy_in` and `stop` of a container whose state Docker does
not report SHALL still fail with `Other`.

#### Scenario: A container without a state

- **WHEN** Docker lists two containers with the label and reports no state for one of them, and `list` is called
- **THEN** it returns `Ok` with the other container only

### Requirement: Log pages come from Docker's timed stream

`logs_since` SHALL read Docker's log stream with timestamps from the whole second `S` of the cursor, drop lines
stamped before `S` whatever Docker's filter let through, skip the lines the cursor counts, and stop once the page is
full. Lines sharing a second, or out of time order within it, SHALL come back once each. A line Docker split at
16 KiB SHALL come back as several lines. A byte that is not UTF-8 SHALL become U+FFFD. A frame without a time SHALL
fail with `Other`.

#### Scenario: Lines across seconds while the program runs

- **WHEN** a container prints a numbered line every 300 ms for 3 seconds, and a host calls `logs_since` every
  500 ms with `max_lines` 2 and the last `next`, and once more after the container exited
- **THEN** the host has every numbered line once, in order

#### Scenario: A line stamped before the cursor's second

- **WHEN** the log holds A stamped in second T, then B stamped in T-1, then C stamped in T, a page ends after C, and
  Docker's `since=T` returns A, B and C
- **THEN** the next read drops B, skips A and C, and does not return C again

#### Scenario: Text that is not UTF-8

- **WHEN** a container prints the byte 0xFF and a newline
- **THEN** `logs_since` returns one line whose text is U+FFFD
