# Proposal

## Why

The Docker runtime pulls an image that is not on the machine, always. Factory's local images (`factory/<name>:dev`)
would then be pulled from Docker Hub's `factory` namespace on a fresh machine: someone else's image, run as Factory's
(2026-11-28).

## What Changes

- `ContainerSpec` gains a pull policy: never, if missing (today's behaviour, the default) or always. `never` with a
  missing image fails with the existing image-not-found error and pulls nothing.
- Also, from the code review of `container-network-isolation` (note 1): the E2E check that an isolated network cannot
  reach the host dials the first address of its subnet, which Docker gives to the probe container itself, so the
  refusal does not come from the host. Start another container on the network first, or check that the probe's own
  address is not the one it dials, and fix the comment (`e2e/tests/suite/container_docker.rs`).

To decide in the design: the names; whether `always` re-pulls on every create or only when the job starts; how the
fake runtime models it; the container actor's contract if the policy is part of a job's spec.

## Out of scope

- Registry credentials (they stay as `docker/auth.rs` finds them) and image signing.

## Order

After `topic-delivery-retry`. Before `container-sysctls`.

## Impact

Crates: cicada-containers.

`crates/cicada-containers/src/spec.rs` (`#[non_exhaustive]`, so no break), `src/docker/mod.rs`, `src/fake.rs`, the
container actor's contract in `docs/reference/contracts` if it carries the spec. Docs handoff: the containers guide.
