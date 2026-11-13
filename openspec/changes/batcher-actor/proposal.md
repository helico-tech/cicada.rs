# Proposal

## Why

Some targets work on lists: a bulk API, a digest email, or one write for many rows. One actor per stream can collect
items in handling order and forward a batch on count or time. The actor uses the public core API, following the
ready-made actors of ADR 0028; throughput is not a goal.

## What Changes

- Add `BatcherActor`, type name `cicada.batcher`, with fixed target, message type, maxItems, maxWait and optional
  target retry. Both limits are required; maxItems is 1 through 1,000, and maxWait is positive.
- `Add { item }` appends one JSON value. The first retained item arms a public `Flush` at the batch's deadline.
  An Add that reaches maxItems, or finds an overdue batch, sends and clears the batch in that same turn.
- Public `Flush` sends the current items as `{"items": [...]}`, cancelling the timer and clearing atomically.
  Empty is a no-op. A timer carries the crate's shared retry policy; an operator may flush early or redrive it.
- A missing target fails the turn. An Add that would flush commits neither its item nor the delivery; a failed
  timer keeps the earlier batch. State therefore remains below maxItems even while a target is absent.
- A target's decoding or handling failure occurs after forwarding; its failed message contains the whole batch.
- Never stop automatically. Hosts explicitly create instances, including create-and-add in one turn.
- Pin the contract, add a tested example and guide section, and host the type in the sample service without instances.

This changes the proposal's earlier self-sent immediate Flush: queued Adds could otherwise exceed the count limit
and grow state without bound while the target was absent. Inline delivery keeps the limit enforceable.

## Capabilities

### New Capabilities

- `batcher`: collect items per key in handling order and forward them as one message on count, scheduled wait or
  public Flush, with atomic forwarding and bounded item count.

### Modified Capabilities

None.

## Out of scope

- Flush on bytes, a bound on total state bytes, target-schema checking of each item, callbacks and success tracking.
- A separate expected-count rule: a key per group with maxItems as the expected count already provides aggregation.
- Live settings changes, automatic idle deletion, and a guarantee of delivery despite a missing target or lost timer.
- Changes to the core.

## Order

After `debouncer-actor`, before `durable-flows`.

## Impact

`crates/cicada-actors` (module, exports, tests and wire samples), `docs/reference/contracts`, the actors guide and its
doctests, a tested example, README, crate docs, architecture's crate row, AGENTS map, changelog, and sample-service
catalog and contract snapshots/texts. No existing contract changes.
