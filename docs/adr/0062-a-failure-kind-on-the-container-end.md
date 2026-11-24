# 0062. A failure kind on the container end

Date: 2026-11-24

## Status

Accepted for container-limits-and-kinds. Builds on ADR 0057.

## Context

The container actor of ADR 0057 ends a container once, as `exited`, `failed`, `timedOut` or `stopped`. A `failed` end
covers a refused key, spec, image, start or input, an out-of-memory kill, a container the runtime lost or ended, and
too many runtime errors in a row, told apart only by the reason text. Factory, a consumer, reports a refused or
unstarted run apart from a runtime failure. Its runs also each have their own time limit, while the settings' timeout
covers the whole type. The end is part of the actor's state and of the end message, so a new field changes contract
hashes, which a node compares exactly.

## Decision

- A `failed` end carries `kind`: `notStarted` (the key, spec, image, start or inputs were refused, or a runtime call
  is unsupported), `outOfMemory`, `runtimeEnded` (the runtime reported another failure), `missing` (a started
  container is gone) or `runtimeErrors` (too many errors in a row). Other ends carry none, and decoding an end refuses
  a `kind` on them. An end saved before has none; no kind is guessed from its reason.
- A runtime reports an out-of-memory kill as `Failed` with the reason `OUT_OF_MEMORY`, a constant of the crate, which
  the actor matches exactly.
- A start refused with `InvalidSpec` ends the container at once as `notStarted`.
- The create step begins with `inspect` and adopts a container past `Created` without building its spec or inputs,
  so a replayed step never ends a running container as `notStarted`.
- `ContainerJob::timeout(request, settings)` gives each request its deadline, the settings' timeout by default; it is
  computed at the start, capped at the largest instant and never moved.
- The change is breaking: `ContainerEnd` gains a public field, and the contract hashes of every container job type
  and of every end target change. The unreleased batch becomes 0.21.0.

## Consequences

- Hosts on 0.20.x follow "Changing a contract" (operations guide) for their container job types and end targets, and
  add `kind` to struct literals of `ContainerEnd`.
- A host can route failures by kind; a `failed` end from before 0.21.0 has no kind.
- A start refused with another error than `InvalidSpec` still ends later, as `runtimeEnded`.
- A new kind later is a contract change again.
- A runtime other than Docker must use `OUT_OF_MEMORY` for the actor to report `outOfMemory`.
