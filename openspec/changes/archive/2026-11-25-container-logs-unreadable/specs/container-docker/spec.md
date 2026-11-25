## MODIFIED Requirements

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
