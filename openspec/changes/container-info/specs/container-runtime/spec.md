## ADDED Requirements

### Requirement: The runtime reports its processor kind and operating system

`info()` SHALL return a `RuntimeInfo` with the runtime's processor kind `arch` and operating system `os`, in the
spelling of image platforms (`amd64`, `arm64`, `linux`). It SHALL change nothing and be safe to repeat. A runtime that
cannot be reached SHALL fail with `DaemonUnavailable`. `RuntimeInfo` SHALL be `#[non_exhaustive]` with public fields
and a constructor `RuntimeInfo::new(arch, os)`.

#### Scenario: A repeated call changes nothing

- **WHEN** a host calls `info()` twice on a runtime that answers and holds a created container
- **THEN** both calls return the same `arch` and `os`, and the container is still `Created`

### Requirement: info keeps other runtimes compiling

`info` SHALL have a default implementation that fails with `Unsupported`, so an implementation of `ContainerRuntime`
outside this repo compiles unchanged. The fake and the Docker backend SHALL implement it.

#### Scenario: A runtime written before info

- **WHEN** a host's own runtime implements every method but `info`, and `info` is called on it
- **THEN** it compiles, and the call fails with `Unsupported`

### Requirement: The fake reports a scripted processor kind

`FakeRuntime::info` SHALL return `amd64` on `linux` until a test sets another `RuntimeInfo` with `set_info`, SHALL
record each call as `Call::Info`, and SHALL fail as scripted for `Method::Info`.

#### Scenario: A scripted machine and a daemon that stops answering

- **WHEN** a test sets `arm64` on `linux`, makes the next `info` fail with `DaemonUnavailable`, and calls `info` twice
- **THEN** the first call fails with `DaemonUnavailable`, the second returns `arm64` on `linux`, and `calls()` lists
  two `Info` calls
