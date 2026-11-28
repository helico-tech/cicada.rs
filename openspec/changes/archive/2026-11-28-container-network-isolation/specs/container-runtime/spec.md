## ADDED Requirements

### Requirement: An isolated network cannot reach the host

`NetworkSpec` SHALL have `isolated`, false by default, for a network whose containers can reach neither the host nor
its published ports. `ensure_network` of an isolated spec that is not `internal` SHALL fail with `InvalidSpec` and
change nothing. A runtime that cannot make an isolated network SHALL fail `ensure_network` of an isolated spec with
`Unsupported` and SHALL NOT create a network that is less closed. `FakeRuntime` SHALL keep `isolated` with the network.

#### Scenario: Isolated without internal

- **WHEN** `ensure_network` is called with `isolated` true and `internal` false
- **THEN** it fails with `InvalidSpec` and no network exists with that name

#### Scenario: A repeated isolated ensure

- **WHEN** `ensure_network` is called twice with the same internal, isolated spec on `FakeRuntime`
- **THEN** both calls return `Ok`, and the one stored network is isolated

## MODIFIED Requirements

### Requirement: Errors say whether a retry can help

Every method SHALL fail only with a `ContainerError` whose kind is `DaemonUnavailable`, `Conflict`, `ImageNotFound`,
`InvalidSpec`, `Unsupported` or `Other`. `is_retryable()` SHALL be true only for `DaemonUnavailable`. A call that does
not fit the current state SHALL fail with `Conflict`: `start`, `copy_in` or `logs` on a missing container, and
`ensure_network` on an existing network whose `internal` or `isolated` flag differs. A call the runtime does not
implement, or cannot do for this container, SHALL fail with `Unsupported`: `logs` and `logs_since` of a container
whose log the runtime can never read, and `ensure_network` of an isolated spec on a runtime that cannot isolate.

#### Scenario: Starting a missing container

- **WHEN** `start` is called for a name that has no container
- **THEN** it fails with kind `Conflict` and `is_retryable()` is false

#### Scenario: A follower stops

- **WHEN** a host follows the log of a container whose log driver keeps none, with `logs_since`
- **THEN** the call fails with `Unsupported`, not `Other`

#### Scenario: An existing network with another isolated flag

- **WHEN** an internal network exists without isolation, and `ensure_network` is called for it with `isolated` true
- **THEN** it fails with `Conflict`, and the existing network is unchanged
