# Proposal

## Why

A flow can run steps and sleep (ADR 0031). It cannot hand work to another actor and wait for the answer. A flow that
needs a result from an actor has two ways today, and neither is enough:

- Await the result inside a step. The handler stays open for as long as the work takes: minutes for a container, days
  for a human decision. The turn's lease must be held all that time, and a node restart starts the step again.
- Poll: read the other actor in a step, sleep, repeat. Every round adds a step and a sleep to the journal, which holds
  at most 1,000 entries, and the answer is always late by the interval.

The durable-flows change left this out on purpose ("step sends to actors, wait-for-event, invoke, parallel steps").

Found in cicada-code-factory: one run of its work starts containers (clone, setup, agent, verify with its services,
publish), waits for each to exit, and sometimes waits for a person to approve an action. Today a hand-written actor
with a `step` field does this. It would be a flow if a flow could wait for a reply
(`docs/proposals/development-workflow-actors.md` of that repository).

## What Changes

- A flow can send a message to an actor, and create that actor first, as part of a step: the create, the send and the
  journal entry commit together, so a replay never sends twice.
- A flow can then wait for a typed reply that belongs to that request, or for a timeout. The run is suspended
  meanwhile, as in a durable sleep: no handler open, no polling, one journal entry for the wait whatever its length.
- A reply that comes twice is taken once. A reply that comes before the flow reaches the wait is kept and taken when
  it gets there. Both hold across restarts and retries.
- The target is a child of the run (a job) or an independent actor that outlives it (a resource, a human decision).
- Several waits can be open at once, up to a bound, with a join: wait for all, or for the first.
- This is more than one small change. The design loop splits it, for instance: (1) a step that creates and sends;
  (2) one wait for a correlated reply, with duplicate and early replies; (3) the timeout; (4) parallel waits and the
  join.

## To decide

- Where the waiting lives: in `FlowActor`'s state and messages, or in a reusable wait/join actor that the flow talks
  to. The second keeps the flow's contract small, the first needs no extra actor.
- How a reply finds its wait: a correlation id made by the flow, the step id, or the id of the request's message; and
  how a replying actor learns the address and id to answer to.
- The reply's type: one typed message per wait, checked on replay like a step's result, and what an undecodable reply
  does.
- What a timeout is: a result the flow branches on, or an error retried by the flow's policy. What a late reply after
  a timeout does.
- A target that fails for good or is deleted while the flow waits: a watch (ADR 0018, ADR 0023) that ends the wait,
  or only the timeout.
- The bound on open waits, and what the journal holds for a wait and its reply against the 1,000 entries.
- The flow's contract: these are new messages and journal entries. In-flight runs of older code must keep replaying.

## Capabilities

### New Capabilities

- `flow-waits`: a flow can send to an actor and suspend until its correlated reply or a timeout, alone or joined.

### Modified Capabilities

- `flows` (steps that send and create, the journal), `flow-sleep` (a wait and a sleep share the suspended run).

## Out of scope

Cancelling a run and its open waits (the next change, `flow-cancellation`); waiting for a published event or a topic;
calling another flow and awaiting its result as one operation; compensation of finished steps; large results outside
the state; any change to how turns are claimed or dispatched.

## Order

After `store-close-cancellation`, before `flow-cancellation`.

## Impact

`crates/cicada-actors` (`flow`, its contract in `docs/reference/contracts` and `tests/contracts`), perhaps a new
ready-made actor, an ADR beside ADR 0031, the actors guide and its doctests, an example, the changelog. No store or
core change is proposed. Once it is released, cicada-code-factory can write a run of its work as a flow.
