# Proposal

## Why

Several flows may react to the same business event. A topic sends to existing actors; flow runs need creation with
input before their first Continue. A dedicated event actor can atomically create and start those runs.

## What Changes

- Add `EventActor`, type name `cicada.event`, one instance per event name, with a bounded list of subscribed flow types.
- Public Subscribe and Unsubscribe edit that list idempotently; the host explicitly creates and subscribes it.
- Publish carries JSON input. In one turn it creates one independent run per subscribed type, keyed by the Publish's
  envelope id, and sends each Continue. One invalid target prevents the entire fan-out; store failures follow the
  Publish's retry policy. The run's own continuation uses the flow's retry policy.
- A repeated publication is governed by core dedup and existing run identities. Done runs ignore Continue; existing
  running runs can recover without replacing their input or saved results. Fresh scheduler messages create fresh runs.
- Document that only flow types should be subscribed; registry metadata cannot prove that an arbitrary actor is a
  FlowActor. A hosting node validates typed input on create; a non-hosting node cannot, as for core raw creates.
- Pin the contract, host the event actor in the sample service and show event starts and composition with the scheduler,
  limiter, debouncer and batcher in the guide and examples.

## Capabilities

### New Capabilities

- `flow-events`: atomically create and start subscribed flow runs using the publication id as their key.

### Modified Capabilities

None.

## Out of scope

Partial-success fan-out, filtering expressions, automatic subscription, wait-for-event inside a run, invocation,
parallel steps, step sends, new endpoints and core changes. Runs are not children of the event actor.

## Order

After `flow-sleep`, the last of the line.

## Impact

`crates/cicada-actors` (event actor, exports, tests and wire samples), `docs/reference/contracts/cicada.event.json`,
actors-guide doctests, tested examples, README, crate docs, architecture's crate row, AGENTS map, changelog, and
sample-service catalog and contract snapshots/texts. Existing actors and flow contracts do not change.
