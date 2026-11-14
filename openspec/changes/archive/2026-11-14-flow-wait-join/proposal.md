# Proposal

## Why

A flow may start several independent jobs before waiting. It needs a bounded durable wait for all or the first, without concurrent mutable Step calls or a journal entry for every polling round.

## What Changes

- Add wait-all and wait-first for bounded lists of typed request handles, with homogeneous reply types (an enum can represent mixed data).
- Define reply order, duplicate handles, early replies, partial results on timeout and closure of unselected requests after wait-first.
- Use one wait journal entry and the existing reply/deadline protocol; pin deterministic replay matching and storage bounds.

## Design boundaries

Part of the six-change flow line requested by the owner. The shared architecture and wire choices are settled in the
first design; each later piece gets its own design, spec delta, review and tasks. The original use case is durable
job/resource work in cicada-code-factory. No core or store change is proposed. Exact API and edge cases remain subject
to that piece's design review; this proposal does not claim its behavior is already available.

## Out of scope

Parallel external step futures, automatic cancellation of losing target jobs, flow cancellation/finalization and unbounded fan-out.

## Order

After `flow-wait-timeout`. Before `flow-cancellation`.

## Impact

`crates/cicada-actors` flow implementation, host flow contract fixtures, actor guide and examples, architecture/ADR
as required, operations and changelog. The first piece introduces the shared contract break; later pieces are intended
to use that shape without another contract change, subject to verification during design.
