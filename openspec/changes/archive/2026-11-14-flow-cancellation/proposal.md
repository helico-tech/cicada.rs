# Proposal

## Why

Deleting a run executes no cleanup handler. A public cancel message should stop future normal work, interrupt its saved sleep or wait, and preserve an observable terminal run. Durable user cleanup is the next piece.

## What Changes

- Add idempotent Cancel handling using the reserved contract: once handled, stop normal replay, cancel the armed continuation and mark the run cancelled.
- Preserve actor serialization: a running turn finishes first; Cancel does not preempt its handler. Done/cancelled runs ignore repeated requests.
- Retain the run and its journal; report status through reads/change hints or application messages, not a synthetic core Stopped notification.
- Define interruption of sleeps/open waits and late replies. State explicitly that this piece performs no user cleanup and the following finalizer piece defers cancelled until cleanup finishes.

## Design boundaries

Part of the six-change flow line requested by the owner. The shared architecture and wire choices are settled in the
first design; each later piece gets its own design, spec delta, review and tasks. The original use case is durable
job/resource work in cicada-code-factory. No core or store change is proposed. Exact API and edge cases remain subject
to that piece's design review; this proposal does not claim its behavior is already available.

## Out of scope

User finalization (next change), compensation, forced handler interruption, automatic child/resource cleanup, changing delete or core watches.

## Order

After `flow-wait-join`. Before `flow-finalizer`.

## Impact

`crates/cicada-actors` flow implementation, host flow contract fixtures, actor guide and examples, architecture/ADR
as required, operations and changelog. The first piece introduces the shared contract break; later pieces are intended
to use that shape without another contract change, subject to verification during design.
