## ADDED Requirements

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
only an unavailable daemon SHALL end a sweep with an error.

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

### Requirement: A container's status tells its own exit from the runtime's

`inspect` SHALL report a container as `Missing`, `Created`, `Running`, `Exited` with the program's exit code, or
`Failed` with an optional exit code and a reason when the container did not start, ran out of memory or was lost by
the runtime. A container ended by `stop` SHALL be reported as `Exited`.

#### Scenario: Inspecting a container that never existed

- **WHEN** `inspect` is called for a name that has no container
- **THEN** it returns `Missing`, not an error

### Requirement: Files are copied only before start

`copy_in` SHALL write the given files into a `Created` container, and SHALL fail with `Conflict` for a container
that is missing, running or ended.

#### Scenario: Copying into a running container

- **WHEN** `copy_in` is called for a running container
- **THEN** it fails with `Conflict` and no file is written

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
running container with a chosen status, set its logs, make the next or every call of a method fail with a chosen
kind and without effect, and read every call in order. The fake SHALL use no timer, so it runs under paused time.

#### Scenario: Scripted exit

- **WHEN** a test creates and starts `job-1` on the fake and then ends it with exit code 3
- **THEN** `inspect("job-1")` returns `Exited` with code 3 and `calls()` lists `create`, `start` and `inspect` in order

#### Scenario: Scripted failure

- **WHEN** a test makes the next `create` fail with `DaemonUnavailable`
- **THEN** that `create` fails with that kind, no container exists, and the following `create` succeeds
