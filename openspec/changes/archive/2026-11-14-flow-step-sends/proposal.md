# Proposal

## Why

A flow currently cannot send actor work with its saved progress. An external step cannot make a core send atomic with the journal, so retries can enqueue it again. This is the first piece of the reply-waits and cancellation line.

## What Changes

- Add a replayable send step with an optional independent or child create; commit effects, one send journal entry and the next continuation together.
- Settle the shared flow wire shape for the agreed reply, timeout, join and cancellation pieces in ADR 0044, with explicit rejection of reserved behavior until implemented.
- Keep Step separate from Turn. Replay skips message/create-state serialization and buffers no effects; mismatches fail permanently.
- Document the contract-breaking upgrade, old-progress compatibility and the difference between once-committed sends and at-least-once target execution.

## Design boundaries

Part of the six-change flow line requested by the owner. The shared architecture and wire choices are settled in the
first design; each later piece gets its own design, spec delta, review and tasks. The original use case is durable
job/resource work in cicada-code-factory. No core or store change is proposed. Exact API and edge cases remain subject
to that piece's design review; this proposal does not claim its behavior is already available.

## Out of scope

Reply handling, waits, timeouts, joins and cancellation behavior (the following changes); core/store changes; arbitrary Turn access.

## Order

After `store-close-cancellation`. Before `flow-reply-waits`.

## Impact

`crates/cicada-actors` flow implementation, host flow contract fixtures, actor guide and examples, architecture/ADR
as required, operations and changelog. The first piece introduces the shared contract break; later pieces are intended
to use that shape without another contract change, subject to verification during design.
