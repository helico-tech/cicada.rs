# Proposal

## Why

Factory (cicada-code-factory) wants to move its container runs onto `cicada-containers` 0.20.7 and drop its own
`crates/containers` with its `docker` CLI backend (request from the Factory session, 2026-11-24; reference: Factory's
`docs/design/containers.md` and `crates/containers/src/runtime.rs`). It would be the third consumer (ADR 0056). Each Factory run has its own time limit, but `settings.timeout` covers the whole type. And Factory reports refused,
failed to start and lost apart from a runtime failure, while the actor's end calls them all `failed`, told apart
only by the reason text.

## What Changes

- `ContainerJob::timeout(&self, &Request) -> Option<Duration>`, defaulting to the settings' timeout.
- A machine-readable `kind` next to `reason` on a `failed` end, for example `notStarted` (image, spec or start
  refused), `missing`, `outOfMemory` and `runtimeErrors`; the four outcomes stay.

## Out of scope

New outcomes. Per-request poll timing.

## Order

After `container-inputs`, before `pool-size-check`.

## Impact

The actor, its tests and its contract: the end gains a field, so hosts on 0.20.7 follow "Changing a contract". A patch,
released with the rest of the container line.
