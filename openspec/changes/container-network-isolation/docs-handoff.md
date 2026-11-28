# Docs handoff for container-network-isolation

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`NetworkSpec::isolated` requests host isolation for an internal bridge and defaults to `false`. `ensure_network`
checks both `internal` and `isolated` on existing networks. Fake runtime tests exercise validation, repeated ensures
and conflicts. Docker mapping tests exercise the API floor and both gateway modes. The Docker suite's
`an_isolated_network_keeps_containers_from_reaching_the_host` checks that a plain internal bridge can reach a listener
on the Docker host and that an isolated bridge cannot.

## 1. Isolate an internal network from the host

**Where.** `docs/guide/containers.md`, `Runtime calls`, near the `ensure_network` row.

**Text.** Set `NetworkSpec::internal` and `NetworkSpec::isolated` to `true` when containers must be unable to reach
the Docker host and its published ports through this bridge. Docker 28 or later (API 1.48) leaves the bridge without
a host address by setting the IPv4 and IPv6 gateway modes to `isolated`. The option defaults to `false`.
`ensure_network` returns `InvalidSpec` if `isolated` is set without `internal` and `Unsupported` if the negotiated
Docker API is too old. Either flag differing on an existing network gives `Conflict`.

**Caveats.** Update custom implementations of `ContainerRuntime::ensure_network` to honour `isolated` or return
`Unsupported`. An implementation written before this change can ignore the flag and create a weaker network.

## 2. Replace existing profile networks before enabling isolation

**Where.** `docs/how-to/upgrading.md`, `Upgrade to 0.26.0`.

**Text.** Before enabling `isolated` on an existing internal network, disconnect or remove its containers and
remove the old network. Then call `ensure_network` with `internal` and `isolated` both set. Docker cannot update
these flags in place, and `ensure_network` returns `Conflict` for the old network.

**Caveats.** This step applies when opting into isolation for an existing network, such as a Factory profile network.

## Blocking

None.

## Status

The docs checks passed (11 tests), as did the all-features container crate tests (323 tests) and Docker E2E
(63 scenarios). Both passages are proposed for the docs session.
