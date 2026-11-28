## ADDED Requirements

### Requirement: Docker isolates a network with its gateway modes

`DockerRuntime` SHALL create an isolated network as an `internal` bridge with the driver options
`com.docker.network.bridge.gateway_mode_ipv4=isolated` and `com.docker.network.bridge.gateway_mode_ipv6=isolated`,
and a network that is not isolated without them. An existing network SHALL count as isolated only when both options
are `isolated`.

#### Scenario: A container on an isolated network cannot reach the host

- **WHEN** a listener runs in a container on the host network, a container on an internal network connects to its
  network's gateway address on that port, and a container on an isolated network to the first address of its subnet
- **THEN** the container on the internal network connects, and the one on the isolated network does not

### Requirement: Docker refuses isolation below API 1.48

When the agreed API version is older than 1.48 (Docker 28), `ensure_network` of an isolated spec SHALL fail with
`Unsupported` before it looks for an existing network or creates one, and its message SHALL name the agreed version and Docker 28.
Networks that are not isolated SHALL keep working from API 1.44.

#### Scenario: An old daemon

- **WHEN** the agreed API version is 1.47 and `ensure_network` is called with an isolated spec
- **THEN** it fails with `Unsupported`, names API 1.47 and Docker 28, and creates no network

#### Scenario: An old daemon with a network that carries the options

- **WHEN** the agreed API version is 1.47, an internal network of that name exists with both gateway mode options set
  to `isolated`, and `ensure_network` is called with an isolated spec
- **THEN** it fails with `Unsupported`, not `Ok`
