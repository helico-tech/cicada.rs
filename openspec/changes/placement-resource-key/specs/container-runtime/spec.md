## ADDED Requirements

### Requirement: The runtime info names the daemon

`RuntimeInfo` SHALL carry an optional `id` of the runtime instance; `DockerRuntime::info` SHALL fill it from Docker's
`/info` field `ID`.

#### Scenario: The Docker daemon's id

- **WHEN** a host calls `info()` on a `DockerRuntime`
- **THEN** `id` holds the daemon's id, the same for every node that talks to that daemon
