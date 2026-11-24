# Proposal

## Why

Factory (cicada-code-factory) wants to move its container runs onto `cicada-containers` 0.20.7 and drop its own
`crates/containers` with its `docker` CLI backend (request from the Factory session, 2026-11-24; reference: Factory's
`docs/design/containers.md` and `crates/containers/src/runtime.rs`). It would be the third consumer (ADR 0056). Each Factory run has its own time limit, but `settings.timeout` covers the whole type. And Factory reports refused,
failed to start and lost apart from a runtime failure, while the actor's end calls them all `failed`, told apart
only by the reason text.

## What Changes

**BREAKING** (design review): `ContainerEnd` gains a public field, and the contract hashes of every container job
type and of every target type that takes a `ContainerEnded` change, so the unreleased batch becomes 0.21.0.

- `ContainerJob::timeout(&self, &Request, settings: Option<Duration>) -> Option<Duration>`, returning the settings'
  timeout by default; computed at the create and saved as the deadline.
- A machine-readable `kind` next to `reason` on a `failed` end: `notStarted` (key, image, spec, start or inputs
  refused, an unsupported runtime call), `outOfMemory`, `runtimeEnded` (the runtime reported another failure),
  `missing` and `runtimeErrors`; the four outcomes stay.
- Refused inputs of `container-inputs` (an `Err` from `inputs`, a file at a folder's path, or a refusal by `make_dirs`
  or `copy_in`) get the kind `notStarted` (container-inputs design review).

## Out of scope

New outcomes. Per-request poll timing.

## Order

After `container-inputs`, before `pool-size-check`.

## Impact

The actor, its tests and its contract, and the runtime's out-of-memory reason as a constant. **BREAKING**: hosts on
0.20.x follow "Changing a contract" for their container job types and end targets, and a struct literal of
`ContainerEnd` needs `kind`. The batch is released as 0.21.0 when this change has landed (ADR 0062).
