# Proposal

## Why

Stopping normal work is not enough for jobs that own containers or other resources. They must request cleanup and await its completion across restarts before releasing capacity or reporting cancelled.

## What Changes

- Add a separate defaultable Flow finalizer with its own bounded cleanup journal and distinct external idempotency-key namespace.
- Cancel transitions running to cancelling and arms finalization; normal flow code is never replayed afterward. Only successful finalization records cancelled.
- Give finalization a read-only view of saved main progress and request handles, with ordinary steps, sends, waits and timeouts for durable cleanup.
- Specify repeated cancellation/recovery, cleanup failures, pending main requests and completion observation. Delete remains immediate and runs no finalizer.

## Design boundaries

Part of the six-change flow line requested by the owner. The shared architecture and wire choices are settled in the
first design; each later piece gets its own design, spec delta, review and tasks. The original use case is durable
job/resource work in cicada-code-factory. No core or store change is proposed. Exact API and edge cases remain subject
to that piece's design review; this proposal does not claim its behavior is already available.

## Out of scope

Library-managed external-resource cleanup, compensation of completed steps, forced deadlines that kill handlers, changing core watch semantics.

## Order

After `flow-cancellation`. Before `build-loop-docs-handoff`.

## Impact

`crates/cicada-actors` flow implementation, host flow contract fixtures, actor guide and examples, architecture/ADR
as required, operations and changelog. The first piece introduces the shared contract break; later pieces are intended
to use that shape without another contract change, subject to verification during design.
