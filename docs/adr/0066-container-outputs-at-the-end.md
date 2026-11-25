# 0066. Container outputs at the end

Date: 2026-11-25

## Status

Accepted for container-output-collect. Builds on ADR 0057, ADR 0062 and ADR 0065.

## Context

At a container's end a host keeps the rest of its log and an output folder outside the container and tells the owner
what was kept (Factory). Done one container after another, one slow copy holds up the others (Factory issue 0005).

## Decision

- With a store on the actor (`ContainerActor::with_store`), the container actor collects in its own turns after it has
  recorded an end the container ran for: the rest of the log through the guarded append, a few pages per turn, then,
  when the job names outputs, `copy_out` of that folder within its limits and `put_file` per file. A file the store
  refuses is skipped with its reason. Actors run in parallel, so a slow copy holds only its own actor.
- The end is delivered after the collection, with the new optional field `outputs` (`stored`, `bytes`, `skipped`, the
  first 10 skipped with path, size and reason, and `error`). Retryable errors count as runtime errors; at the maximum
  in a row the end is delivered with its recorded outcome and the error in `outputs`.
- The field changes the contract of every container job type: a breaking release with ADR 0065's change (0.23.0).

## Consequences

The owner learns what was kept with the end itself. The job's `MAX_TURN` must cover a copy, and memory per collecting
container is at most its byte limit. Hosts on 0.22.x follow "Changing a contract" for their container job types.
