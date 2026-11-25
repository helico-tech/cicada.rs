# container-docker Specification

## Purpose
The `docker` feature of `cicada-containers` gives `DockerRuntime`, a backend of the container runtime trait over
bollard that keeps every repeat rule against a real Docker daemon and is tested in the E2E suite.

## Requirements

### Requirement: A Docker backend behind a feature

`cicada-containers` SHALL provide `DockerRuntime`, a `ContainerRuntime` over the Docker Engine API, behind the
feature `docker`. `DockerRuntime::connect()` SHALL use `DOCKER_HOST` when it is set; else the endpoint of the Docker
CLI context that `DOCKER_CONTEXT` names, when it is set; else the local Docker socket when it exists; else the endpoint
of the `currentContext` of the CLI's `config.json`. It SHALL connect without TLS, and SHALL not contact the daemon. No
public item SHALL name a type of the Docker client library. The crate without features SHALL not build that library.

#### Scenario: The default build has no Docker client

- **WHEN** the gate checks the dependency tree of `cicada-containers` without features
- **THEN** it holds no `bollard`, and with the feature `docker` it holds `bollard` and no TLS library

#### Scenario: No daemon

- **WHEN** a `DockerRuntime` points at a socket where no daemon listens and `inspect` is called
- **THEN** it fails with kind `DaemonUnavailable` and `is_retryable()` is true

#### Scenario: A Mac without the default socket

- **WHEN** `DOCKER_HOST` and `DOCKER_CONTEXT` are not set, `/var/run/docker.sock` is missing, and `config.json` has
  `currentContext` `orbstack` whose endpoint is a unix socket where Docker listens
- **THEN** `connect()` returns a runtime on that endpoint, and its calls reach that Docker

#### Scenario: A server keeps its socket

- **WHEN** `DOCKER_HOST` and `DOCKER_CONTEXT` are not set, the local socket exists, and `config.json` names another
  `currentContext`
- **THEN** `connect()` uses the local socket

#### Scenario: A context set in the environment

- **WHEN** `DOCKER_CONTEXT` names a context whose endpoint is a unix socket, and the local socket also exists
- **THEN** `connect()` uses the context's endpoint

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
`InvalidSpec`, an image Docker or the registry does not know to `ImageNotFound`, a 501 to a log read (a logging
driver that does not support reading) to `Unsupported`, and every other failure to `Other`,
except where a repeat rule makes the call `Ok` or the trait asks for `Conflict` on a missing container.

#### Scenario: Copying into a running container

- **WHEN** `copy_in` is called for a running container
- **THEN** it fails with kind `Conflict` and no file is written

#### Scenario: Log driver none

- **WHEN** a container made with the log driver `none` has run, and `logs` and `logs_since` are called
- **THEN** both fail with `Unsupported`

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

### Requirement: Docker's archive is read as a stream

`copy_out` SHALL read Docker's archive of `path` as a stream and SHALL hold no more than the kept files' bytes: the
data of a skipped file SHALL be passed over, not stored. An error during the stream SHALL fail the whole call with
its kind, with no partial result. It SHALL work on a created, running or exited container, volumes included; files
written to a tmpfs SHALL be gone once the container ended.

#### Scenario: Artifacts after the end

- **WHEN** a container wrote files to its root file system and to a volume and exited, and `copy_out` is called for
  each folder before `remove`
- **THEN** both return their files

#### Scenario: The stream fails in the middle

- **WHEN** Docker's archive stream fails after the first file, with a lost connection
- **THEN** `copy_out` fails with `DaemonUnavailable` and returns no files

#### Scenario: The container is removed during the call

- **WHEN** the container is found, then removed before Docker sends its archive, which answers 404
- **THEN** `copy_out` inspects again, finds no container, and fails with `Conflict`

#### Scenario: A tmpfs after the end

- **WHEN** a container with a tmpfs at `/scratch` wrote `/scratch/a` and exited, and `copy_out("/scratch")` is called
- **THEN** the result holds no `/scratch/a`

#### Scenario: A progress file while running

- **WHEN** a running container keeps renaming a fresh `/work/progress.json` into place, and `copy_out` is called
  for that file
- **THEN** it returns one whole file

### Requirement: Folders reach Docker without touching what exists

`make_dirs` SHALL learn what exists from Docker's archive stat and SHALL upload only the folders it makes, owned by
uid and gid 0 with their modes, one upload per mount with a read-only root. A folder of the image SHALL keep its
owner and mode, and so SHALL an existing parent.

#### Scenario: A writable folder for user 1000

- **WHEN** `make_dirs` makes `/out` with mode 0o777 in a created container that runs as user 1000, which then writes
  `/out/a`
- **THEN** the program exits with code 0 and `/out/a` exists

#### Scenario: Existing folders keep their owner and mode

- **WHEN** `make_dirs` makes `/tmp/out` and `/run/postgresql` in a created `postgres:18` container
- **THEN** `/tmp` keeps mode 1777, `/run/postgresql` keeps its owner and mode, and `/tmp/out` exists

#### Scenario: A path below a file

- **WHEN** `make_dirs` makes `/etc/passwd/x`, where `/etc/passwd` is a file of the image
- **THEN** it fails with `Conflict` and makes nothing

#### Scenario: A read-only root

- **WHEN** a created container has `read_only_root` and a volume at `/work`, and `make_dirs` makes `/work/cache` and
  `/etc/x`
- **THEN** it fails with `InvalidSpec` and makes nothing; with `/work/cache` alone it makes the folder

### Requirement: Links on Docker are followed to a folder

A link at a named path or a parent SHALL be followed: to a folder it counts as that folder, else it is `Conflict`.
The walk of one named folder SHALL follow at most 8 links and SHALL fail with `Conflict` on one more. Docker's stat
resolves a chain of links itself, so a chain counts as one link, and a link loop SHALL fail the call with `Other`, the
kind of Docker's refusal.

#### Scenario: A folder through a linked parent

- **WHEN** `make_dirs` makes `/var/run/app` in a created `postgres:18` container, where `/var/run` links to `/run`
- **THEN** it returns `Ok`, `/run/app` is a folder, and `/var/run` is still the link

### Requirement: Docker's version call gives the runtime info

`DockerRuntime::info` SHALL call Docker's `GET /version` and return its `Arch` and `Os` as they are. A failed
connection or a timeout SHALL be `DaemonUnavailable`; a response without `Arch` or `Os`, or with an empty one, SHALL be
`Other`. It SHALL set no time limit of its own.

#### Scenario: The local daemon

- **WHEN** the E2E test calls `info()` on the daemon of the current Docker context
- **THEN** it returns `os` `linux` and the `arch` that `docker version --format '{{.Server.Arch}}'` prints

#### Scenario: No daemon behind the socket

- **WHEN** `info()` is called twice on a runtime whose socket has no daemon
- **THEN** both calls fail with `DaemonUnavailable`

### Requirement: A pull uses the machine's Docker login

When `create` must pull an image, `DockerRuntime` SHALL read Docker's `config.json` from the folder set with
`docker_config`, else `DOCKER_CONFIG`, else `$HOME/.docker`, and SHALL pass the credentials for the image's registry
to the pull: from the helper named in `credHelpers` for that registry, else from the `credsStore` helper, else from the
`auths` entry. The registry SHALL be found as Docker finds it, with Docker Hub under `https://index.docker.io/v1/`.

#### Scenario: A private image after docker login

- **WHEN** an image is only in a registry with basic auth, `config.json` has an `auths` entry for that registry, and
  `create` is called for a container of that image
- **THEN** the image is pulled and the container is created

#### Scenario: Credentials from a helper

- **WHEN** `credsStore` names a helper that answers with a user name and a secret for the image's registry, and
  `create` pulls an image from it
- **THEN** the pull uses that user name and secret, and the `auths` entry for that registry is not used

#### Scenario: A login made after connect

- **WHEN** a runtime is made, then a credential is written to `config.json`, then `create` pulls a private image
- **THEN** the pull uses that credential

### Requirement: A failing credential source means a pull without credentials

A missing or broken `config.json`, a registry without an entry, and a helper that is missing, exits non-zero, gives no
JSON or takes more than 10 seconds SHALL give a pull without credentials. A refused login SHALL keep the kind its
error maps to today. A failed pull's message SHALL say where its
credentials came from, or why there were none, and SHALL never hold a credential or a helper's output.

#### Scenario: A public image with a broken helper

- **WHEN** `credsStore` names a helper that is not installed and `create` pulls a public image
- **THEN** the pull runs without credentials and the container is created

#### Scenario: A helper that hangs or fails

- **WHEN** the helper for a registry takes longer than the limit, or exits non-zero, and `create` pulls an image
- **THEN** the helper is stopped, the pull runs without credentials, and a failed pull's message names the helper and
  `timed out` or the exit code

#### Scenario: A private image without a login

- **WHEN** `config.json` has no entry for a private registry and `create` pulls an image from it
- **THEN** it fails, and the message says that there were no credentials for that registry

### Requirement: A context connect cannot use is refused with its name

`connect()` SHALL fail with `Other`, naming the context, for a context it cannot read or use (no meta file, no
docker endpoint, broken JSON, no config folder, TLS, or a scheme other than `unix`, `tcp` and `http`), and SHALL never
fall back to the local socket or to plain `tcp`. A unix endpoint whose socket is missing SHALL fail with
`DaemonUnavailable`, naming the context and the endpoint.

#### Scenario: A TLS context

- **WHEN** `DOCKER_CONTEXT` names a context with `SkipTLSVerify` set
- **THEN** `connect()` fails with `Other`, and the message names the context and says to set `DOCKER_HOST` or use
  `connect_to`

#### Scenario: A context that does not exist

- **WHEN** `DOCKER_CONTEXT` names a context that has no meta file
- **THEN** `connect()` fails with `Other` and the message names the context and `DOCKER_CONTEXT`

#### Scenario: A context from config.json that does not exist

- **WHEN** the local socket is missing and `config.json` has a `currentContext` that has no meta file
- **THEN** `connect()` fails with `Other` and the message names the context and `currentContext`

#### Scenario: No config folder

- **WHEN** `DOCKER_CONTEXT` names a context and neither `DOCKER_CONFIG` nor `HOME` is set
- **THEN** `connect()` fails with `Other` and the message names the context

#### Scenario: A context whose socket is missing

- **WHEN** `DOCKER_CONTEXT` names a context whose endpoint is a unix socket file that does not exist
- **THEN** `connect()` fails with `DaemonUnavailable`, and the message names the context and the endpoint
