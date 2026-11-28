# Proposal

## Why

Factory's profile networks are `internal: true`, but on Docker an internal bridge still lets its containers reach the
host through the gateway, and every published port, so they bypass Factory's egress proxy. Docker 28 and later close
this with the bridge option `com.docker.network.bridge.gateway_mode_ipv4=isolated` (Factory tested it by hand on Docker
29.4 / OrbStack: host and published ports unreachable, the proxy still works). `NetworkSpec` has no way to pass it.
Factory calls this a security must-fix and put it first (2026-11-28).

## What Changes

- `NetworkSpec` can ask for a network whose containers cannot reach the host, and `ensure_network` of the Docker
  runtime creates it so; the fake runtime records it.
- A daemon that cannot honour it fails the call instead of creating a weaker network.

To decide in the design: an `isolated` flag (portable, says the intent) or free driver options (a map passed to
Docker, more general, easy to get wrong), or both; how a too-old daemon is detected (its API version, or inspecting the
created network) so isolation never silently fails open; what `ensure_network` does with an existing network of that
name whose isolation differs (fail, as Docker cannot change it in place); IPv6 (`gateway_mode_ipv6`).

## Out of scope

- Firewall rules outside Docker, and the egress proxy itself.
- Sysctls of containers: `container-sysctls`.

## Order

After `remove-inspector`. Before `actor-ports`.

## Impact

Crates: cicada-containers.

`crates/cicada-containers/src/spec.rs` (`NetworkSpec`, `#[non_exhaustive]`, so no break), `src/docker/mod.rs` and
`src/docker/map.rs` (`ensure_network`), `src/fake.rs` and `check_runtime`-style tests; an E2E check against a real
daemon if the gate's Docker allows it. Docs handoff: the containers guide's network section.
