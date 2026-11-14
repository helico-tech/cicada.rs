# Proposal

## Why

A job or human-decision actor may take minutes or days to answer. Keeping a handler open holds its lease; polling makes late answers and grows the journal. This piece builds on atomic request sends and durably suspends without polling.

## What Changes

- Add a typed request handle and ReplyTo route carrying run address and request message id, plus a helper for the target actor to reply.
- Accept only a known request from its recorded target; retain early replies, ignore unknown/duplicate replies without growing state, and use explicit bounded storage.
- Add a single typed wait with one journal entry and no timeout; commit the selected reply before later flow code can act on it.
- Specify decode errors, reply depth, terminal-run behavior, recovery and sender/correlation semantics using the shared flow wire contract.

## Design boundaries

Part of the six-change flow line requested by the owner. The shared architecture and wire choices are settled in the
first design; each later piece gets its own design, spec delta, review and tasks. The original use case is durable
job/resource work in cicada-code-factory. No core or store change is proposed. Exact API and edge cases remain subject
to that piece's design review; this proposal does not claim its behavior is already available.

## Out of scope

Timeouts, joined waits and cancellation/finalization (following changes); topic/event waits, automatic target watches and core changes.

## Order

After `flow-step-sends`. Before `flow-wait-timeout`.

## Impact

`crates/cicada-actors` flow implementation, host flow contract fixtures, actor guide and examples, architecture/ADR
as required, operations and changelog. The first piece introduces the shared contract break; later pieces are intended
to use that shape without another contract change, subject to verification during design.
