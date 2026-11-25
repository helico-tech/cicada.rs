## ADDED Requirements

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
