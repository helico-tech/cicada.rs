## MODIFIED Requirements

### Requirement: Errors say whether a retry can help

Every method SHALL fail only with a `ContainerError` whose kind is `DaemonUnavailable`, `Conflict`, `ImageNotFound`,
`InvalidSpec`, `Unsupported` or `Other`. `is_retryable()` SHALL be true only for `DaemonUnavailable`. A call that does
not fit the current state SHALL fail with `Conflict`: `start`, `copy_in` or `logs` on a missing container, and
`ensure_network` on an existing network whose `internal` flag differs. A call the runtime does not implement, or
cannot do for this container, SHALL fail with `Unsupported`: `logs` and `logs_since` of a container whose log the
runtime can never read.

#### Scenario: Starting a missing container

- **WHEN** `start` is called for a name that has no container
- **THEN** it fails with kind `Conflict` and `is_retryable()` is false

#### Scenario: A follower stops

- **WHEN** a host follows the log of a container whose log driver keeps none, with `logs_since`
- **THEN** the call fails with `Unsupported`, not `Other`
