## MODIFIED Requirements

### Requirement: A container's status tells its own exit from the runtime's

`inspect` SHALL report a container as `Missing`, `Created`, `Running`, `Exited` with the program's exit code, or
`Failed` with an optional exit code and a reason when the container did not start, ran out of memory or was lost by
the runtime. A container ended by `stop` SHALL be reported as `Exited`. For a container killed for lack of memory,
the reason SHALL be exactly the crate's constant `OUT_OF_MEMORY`.

#### Scenario: Inspecting a container that never existed

- **WHEN** `inspect` is called for a name that has no container
- **THEN** it returns `Missing`, not an error

#### Scenario: Out of memory on every runtime

- **WHEN** the Docker backend reports a container killed for lack of memory
- **THEN** its status is `Failed` with the reason `OUT_OF_MEMORY`, which the container actor reads as `outOfMemory`
