# Proposal

## Why

A target can fail or disappear without replying. A durable reply wait needs a deadline that survives recovery and selects one outcome before later code can perform effects.

## What Changes

- Add a wait with an absolute deadline derived once from store claim time and a duration; replay uses the saved deadline.
- Return timeout as branchable data, not an endlessly retried error. Persist the selected outcome in its own turn before resuming code.
- Resolve reply/timer races by actual mailbox handling order, retain the decision across retries, and ignore late replies.
- Preserve public Continue recovery and ordinary scheduling delays without polling or holding a handler open.

## Design boundaries

Part of the six-change flow line requested by the owner. The shared architecture and wire choices are settled in the
first design; each later piece gets its own design, spec delta, review and tasks. The original use case is durable
job/resource work in cicada-code-factory. No core or store change is proposed. Exact API and edge cases remain subject
to that piece's design review; this proposal does not claim its behavior is already available.

## Out of scope

Joined waits and cancellation/finalization; automatic target watches; changing core scheduling guarantees.

## Order

After `flow-reply-waits`. Before `flow-wait-join`.

## Impact

`crates/cicada-actors` flow implementation, host flow contract fixtures, actor guide and examples, architecture/ADR
as required, operations and changelog. The first piece introduces the shared contract break; later pieces are intended
to use that shape without another contract change, subject to verification during design.
