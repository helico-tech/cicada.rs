# Proposal

## Why

Factory's egress proxy container runs with `net.ipv4.ip_forward=1` from its image or host default, so a container with
`NET_ADMIN` on its network could route through it to the other network. Factory wants to set
`net.ipv4.ip_forward=0` on containers, and `ContainerSpec` has no sysctls (2026-11-28).

## What Changes

- `ContainerSpec` gains `sysctls`, a map passed to the Docker runtime's create; the fake runtime records it.

To decide in the design: any check of names (Docker allows only namespaced sysctls and fails others at create);
whether the container actor's job spec carries it (its contract).

## Out of scope

- Host sysctls, and capabilities beyond the existing `drop_all_capabilities`.

## Order

After `container-pull-policy`. Before `test-probe`.

## Impact

Crates: cicada-containers.

`crates/cicada-containers/src/spec.rs` (`#[non_exhaustive]`, so no break), `src/docker/map.rs`, `src/fake.rs`, the
container actor's contract if it carries the spec. Docs handoff: the containers guide.
