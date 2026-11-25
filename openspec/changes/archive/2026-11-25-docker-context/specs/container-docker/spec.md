## MODIFIED Requirements

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

## ADDED Requirements

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
