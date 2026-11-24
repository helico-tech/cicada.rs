# 0057. A reconciled container actor

Date: 2026-11-24

## Status

Accepted for container-actor. Builds on ADRs 0031, 0052, 0053 and 0056.

## Context

A handler runs at least once, and a turn can be cut off after Docker started a container but before the commit. A
handler that starts a container directly then starts a second one, or its state says nothing runs while something
does. cicada-code-factory and brood both run containers from actors; ADR 0056 lets their shared layer live in
`cicada-containers`, and `container-runtime` gives it a trait whose every call is safe to repeat.

ADR 0053 kept process ownership as an example only: a PID does not prove ownership after a host restart, so its actor
keeps live handles in the host and marks old runs as needing an operator. A container is different: the daemon keeps
it under a name, and any node that reaches the same daemon can ask about it.

## Decision

- A ready-made container actor in `cicada-containers`, under a type name the host chooses, as a flow's (ADR 0031): the
  host brings its runtime, its settings and the code that builds a spec from a small typed request in the state.
- The actor reconciles: a message only records the goal (run, stop, remove). One armed poll at a time asks the runtime
  what is true and takes one step toward the goal. Runtime errors count in the state and back off; they never fail a
  turn.
- The container's name is a host prefix plus the actor's key. The name, not a handle or an id in the state, is the
  identity: a repeated step finds the container instead of making another, so no host keeps handles and a host
  restart needs no operator.
- A container ends once. Its end, with the tail of its logs, is saved and sent to a target in the commit that records
  it, before anything removes the container.
- Placement stays with the host. Every node that can claim the actor must reach the same daemon; the host pins with
  ADR 0052's claim owner when only some do.

## Consequences

- A turn that runs again cannot start a second container, as long as the key and prefix are not reused for another
  job. A host must never reuse a key, also after a delete: a new actor under an old key adopts the old container and
  reports its old end. Delete an actor only after `Remove`, or its container stays.
- A node that reaches another daemon than the one that made the container sees it missing and ends it `failed`, or
  creates a second container there. Pinning prevents this; the actor cannot detect it.
- The secrets of a spec never reach the store; the host builds them at each create.
- ADR 0053 stands for processes. Its example and this actor show the two cases: a resource with no durable outside
  identity needs host-owned handles; one with a stable name can be reconciled.
- Polling costs one turn per interval for each running container; throughput is not a goal.
