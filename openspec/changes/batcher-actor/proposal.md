# Proposal

## Why

Some targets work better on a list than on one item at a time: a bulk API, a digest email, one write for many rows.
Outside, batching by size or by time is a named feature of Broadway and Inngest, and a paid one of Oban Pro (Chunk)
and River Pro. The same actor with a key per group is the aggregator of the integration patterns. It is not on ADR
0018's list, and it comes last of the four small actors, because throughput is not a goal of cicada.

## What Changes

- A new ready-made actor `BatcherActor`, type name `cicada.batcher`, one instance per stream of items (the key is its
  name).
- Its message `Add { item }` takes a JSON value and appends it. The first item of a batch arms a `Flush` after the
  longest wait; the item that reaches the largest count makes the actor send itself a `Flush` at once.
- Its own `Flush` sends one message to the target, of a type name set at create, with the payload
  `{"items": [...]}`, and clears the batch in the same commit. The target declares a variant with a list field.
- A batch that the target cannot decode fails at the target and reaches its dead letters whole. A target that is
  absent fails the `Flush`, and the items stay.
- An item that arrives after a flush starts a new batch.
- Its settings are in its state, set at create: the target, the type name of the flush, the largest count and the
  longest wait. Both limits are required, so a batch always ends and the state stays bounded.
- Its contract is pinned in `docs/reference/contracts`; the actors guide gets a section with doctests.

The simplest version: a flush on count or on time, one target, items kept in order.

## Capabilities

### New Capabilities

- `batcher`: a ready-made actor that collects items and sends them to a target as one message, when the batch reaches
  its largest count or its longest wait.

### Modified Capabilities

None.

## Out of scope

- A batch that waits for an expected count per correlation id, or for every member to succeed (a batch with
  callbacks). The second needs a notice of success that the core does not give.
- A flush on the size of the batch in bytes.
- Checking an item against the target's schema at `Add`; the target's decoder is the judge.
- Changes to the core.

## Questions for the design

- What are the default and the cap of the largest count? The whole state is read and written on every turn, and the
  wrapping adds two levels toward the depth limit of a value.
- Is an expected count a third rule to end a batch, so that this actor is also the aggregator?

## Order

After `debouncer-actor`, before `durable-flows`. The last of the four small actors.

## Impact

- `crates/cicada-actors` (a module `batcher`, its tests on virtual time, `tests/wire_formats.rs`),
  `docs/reference/contracts` (a new file), [docs/guide/actors.md](../../../docs/guide/actors.md) and its doctests,
  `README.md` (the table of actors), `CHANGELOG.md`.
- Hosts: no host has asked for it.
