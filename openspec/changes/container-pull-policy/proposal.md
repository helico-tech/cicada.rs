# Proposal

## Why

The Docker runtime pulls an image that is not on the machine, always. Factory's local images (`factory/<name>:dev`)
would then be pulled from Docker Hub's `factory` namespace on a fresh machine: someone else's image, run as Factory's
(2026-11-28).

## What Changes

- `ContainerSpec` gains a pull policy: never, if missing (today's behaviour, the default) or always. `never` with a
  missing image fails with the existing image-not-found error and pulls nothing.

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
