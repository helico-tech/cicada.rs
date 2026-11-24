# Proposal

## Why

A container cannot be started from a handler the naive way. Handlers run at least once, and a turn can be cut off
after Docker started a container but before the commit. The turn then runs again and starts a second container, or
the actor's state says nothing runs while something does.

cicada-code-factory solved this with its container actor (`src/actors/container.rs`, about 700 lines), a
**reconciled external resource**:

- A message (`Start`, `Stop`, `Remove`) only records what should be true, and arms a poll.
- Each poll asks the runtime what is true and takes one step toward the goal: create, start, wait, record the exit,
  stop, remove. Then it arms the next poll.
- The container's name comes from the actor's key, so a repeated step finds it instead of making a second one.
- A step's deadline and a count of errors in a row live in the actor; too many errors count as a failure.
- A lost poll comes back when the goal is asked again or when a periodic check arrives.

Issue #2 (higher-level actors) lists the same contract as "external resource reconciliation": start with one
adapter and check the shared contract against a second. Factory and brood are the two.

This is the hard part of running containers from actors. Every consumer gets the same details wrong in the same ways.
`container-runtime` gives the trait; this change gives the actor on top of it.

## What Changes

- A ready-made container actor in `cicada-containers`, generic over the `ContainerRuntime` trait of
  `container-runtime`, under a type name the host chooses (as a flow does, ADR 0031).
- Its messages: `Start`, `Stop`, `Remove`, and a `Check` that arms a fresh poll when one was lost.
- It tells one target, named in `Start`, how the container ended: exit code or reason, and the tail of
  the logs, recorded in the commit that records the exit and before anything removes the container.
- Its contract is checked like a flow's: a test job's contract pinned in the crate's `tests/contracts`.
- Tests on virtual time with `TestActors` and the fake runtime: a turn that runs twice starts one container, a lost
  poll comes back, the deadline, errors in a row, stop before start, remove after exit.

The decisions on the open questions are in [design.md](design.md).

## Out of scope

Groups of containers (factory's workspace with its slot and its services), slot limits, and a pool of containers.
Streaming output to an actor. Moving factory and brood onto the actor. Runtimes other than Docker
(`container-docker`).

## Order

After `container-docker`, before `capacity-permits`.

## Impact

`crates/cicada-containers` and its tests, with a test job's contract snapshot in its `tests/contracts`. A new ADR for the
ready-made container actor and its placement. Additive, so a patch release. The docs handoff carries a section for
the actors guide, a pattern for the design guide, and a skill route; nothing that exists breaks.
